//! The region copy-out (ADR 13.10.26q D3, G3 of region-scoped deallocation).
//!
//! Between `__albite_region_detach` and `__albite_region_release`, the code
//! generator stores the region's value to a stack slot and calls
//! [`__albite_region_copy_out`] with a static **copy layout** for the value's
//! type. The walk rewrites every pointer that lies in the detached region's
//! chunks to a copy in the parent allocator, so nothing the value reaches is
//! freed by the release that follows.
//!
//! Three properties, each load-bearing:
//!
//! - **No recursion.** An explicit work stack, so a million-node list copies
//!   in constant C stack.
//! - **Sharing is kept.** A forwarding table maps each copied block to its
//!   copy, so a DAG copies each node once and two references to one block
//!   stay one block.
//! - **The walk stops at the region's edge.** A pointer outside the detached
//!   chunks — a string literal, a shared nullary cell, anything allocated
//!   before the region opened — is kept and never descended into: the
//!   language has no assignment, so an older block cannot point into a
//!   younger region.
//!
//! The work stack and the table use Rust's allocator, never
//! `__albite_alloc`, so they never land in the parent arena.
//!
//! # The layout encoding
//!
//! A layout is a `u64` array. Word 0 is its length in words; the root node
//! starts at word 1. A node is addressed by its word index, so a recursive
//! type refers back to itself and the layout is a finite graph:
//!
//! | kind | words | at the location |
//! |---|---|---|
//! | [`KIND_STRING`] | `[1]` | `{ ptr, i64 len }`: the `len` bytes at `ptr` |
//! | [`KIND_STRUCT`] | `[2, n, (offset, node) × n]` | the fields that hold a pointer |
//! | [`KIND_TAGGED`] | `[3, payload_offset, n, (size, node) × n]` | an `i32` tag at 0 selects a variant; `node` is [`NO_NODE`] for a pointer-free one |
//! | [`KIND_BOX`] | `[4, alloc_size, node]` | a pointer to a block of `alloc_size` bytes described by `node` |
//!
//! A boxed block whose node is tagged is copied only to its variant's payload
//! size (`payload_offset + size`), so a short variant's block is never read at
//! the largest variant's size; the copy is still allocated at `alloc_size`
//! and zero-filled, because generated code loads the whole blob.

use std::collections::HashMap;

/// A `{ ptr, i64 }` string.
pub const KIND_STRING: u64 = 1;
/// A by-value struct: the offsets of its pointer-holding fields.
pub const KIND_STRUCT: u64 = 2;
/// A `{ i32, [N x i8] }` tagged union: one optional node per variant.
pub const KIND_TAGGED: u64 = 3;
/// A pointer to a heap block — a μ-type's fold.
pub const KIND_BOX: u64 = 4;
/// A variant or block that holds no pointer.
pub const NO_NODE: u64 = u64::MAX;

/// The detached region's chunks as sorted `[base, end)` ranges, so the
/// membership test is a binary search rather than `Arena::contains`'s scan —
/// a region holding a million-node list has thousands of chunks.
#[derive(Debug, Default)]
pub struct ChunkIndex {
    ranges: Vec<(usize, usize)>,
}

impl ChunkIndex {
    /// Index `(base, len)` spans; the order they arrive in does not matter.
    pub fn new(spans: impl IntoIterator<Item = (usize, usize)>) -> Self {
        let mut ranges: Vec<(usize, usize)> = spans
            .into_iter()
            .map(|(base, len)| (base, base + len))
            .collect();
        ranges.sort_unstable();
        Self { ranges }
    }

    /// Whether `addr` lies in one of the chunks.
    #[must_use]
    pub fn contains(&self, addr: usize) -> bool {
        // The last range starting at or before `addr` is the only candidate.
        let after = self.ranges.partition_point(|&(base, _)| base <= addr);
        after > 0 && addr < self.ranges[after - 1].1
    }
}

/// What one copy-out did — for the tests and the throughput record (D7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyTally {
    /// Blocks and strings copied into the parent.
    pub copied: usize,
    /// Bytes copied (block payloads and string bytes).
    pub bytes: usize,
    /// Pointers found in the region whose target had already been copied.
    pub shared: usize,
}

/// A layout the walk could not read: a code-generator bug, never a user
/// error, so the export aborts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MalformedLayout {
    /// The word index that was out of range or held an unknown kind.
    pub word: usize,
}

/// Copy everything `slot` reaches inside the region out of it.
///
/// `in_region` answers the membership question and `alloc` allocates in the
/// parent; both are injected so the tests drive the walk with no process
/// globals.
///
/// # Errors
///
/// [`MalformedLayout`] when a node index or kind is out of range.
///
/// # Safety
///
/// `slot` must hold a value of the type `layout` describes, and every pointer
/// it reaches must be valid for the reads its node implies; `alloc` must
/// return blocks valid for the requested size.
pub unsafe fn copy_out_with(
    slot: *mut u8,
    layout: &[u64],
    in_region: &impl Fn(usize) -> bool,
    alloc: &mut impl FnMut(usize) -> *mut u8,
) -> Result<CopyTally, MalformedLayout> {
    let mut walk = Walk {
        layout,
        in_region,
        alloc,
        stack: vec![(slot, 1)],
        blocks: HashMap::new(),
        strings: HashMap::new(),
        tally: CopyTally::default(),
    };
    while let Some((location, node)) = walk.stack.pop() {
        // SAFETY: the caller's contract, carried to each reachable location.
        unsafe { walk.visit(location, node)? };
    }
    Ok(walk.tally)
}

struct Walk<'a, F, A> {
    layout: &'a [u64],
    in_region: &'a F,
    alloc: &'a mut A,
    stack: Vec<(*mut u8, usize)>,
    /// Source block address → its copy.
    blocks: HashMap<usize, usize>,
    /// Source `(ptr, len)` → the copied bytes: a slice into one buffer is
    /// keyed by its own extent, so two slices never alias a short copy.
    strings: HashMap<(usize, usize), usize>,
    tally: CopyTally,
}

impl<F: Fn(usize) -> bool, A: FnMut(usize) -> *mut u8> Walk<'_, F, A> {
    fn word(&self, index: usize) -> Result<u64, MalformedLayout> {
        self.layout
            .get(index)
            .copied()
            .ok_or(MalformedLayout { word: index })
    }

    fn node_at(&self, index: usize) -> Result<usize, MalformedLayout> {
        let node = self.word(index)?;
        usize::try_from(node)
            .ok()
            .filter(|n| *n < self.layout.len())
            .ok_or(MalformedLayout { word: index })
    }

    fn offset_at(&self, index: usize) -> Result<usize, MalformedLayout> {
        usize::try_from(self.word(index)?).map_err(|_| MalformedLayout { word: index })
    }

    unsafe fn visit(&mut self, location: *mut u8, node: usize) -> Result<(), MalformedLayout> {
        match self.word(node)? {
            // SAFETY (all arms): the caller's contract on `location`.
            KIND_STRING => unsafe { self.copy_string(location) },
            KIND_STRUCT => {
                let fields = self.offset_at(node + 1)?;
                for field in 0..fields {
                    let offset = self.offset_at(node + 2 + 2 * field)?;
                    let child = self.node_at(node + 3 + 2 * field)?;
                    // SAFETY: the field lies inside the struct at `location`.
                    self.stack.push((unsafe { location.add(offset) }, child));
                }
            }
            KIND_TAGGED => {
                // SAFETY: a tagged union starts with its `i32` tag.
                let tag = unsafe { location.cast::<u32>().read_unaligned() } as usize;
                let payload = self.offset_at(node + 1)?;
                let variants = self.offset_at(node + 2)?;
                if tag < variants && self.word(node + 4 + 2 * tag)? != NO_NODE {
                    let child = self.node_at(node + 4 + 2 * tag)?;
                    // SAFETY: the payload lies inside the union.
                    self.stack.push((unsafe { location.add(payload) }, child));
                }
            }
            KIND_BOX => unsafe { self.copy_block(location, node)? },
            _ => return Err(MalformedLayout { word: node }),
        }
        Ok(())
    }

    unsafe fn copy_string(&mut self, location: *mut u8) {
        // SAFETY: a string is `{ ptr, i64 }` at `location`.
        let (ptr, len) = unsafe {
            (
                location.cast::<*const u8>().read_unaligned(),
                location.add(8).cast::<usize>().read_unaligned(),
            )
        };
        if ptr.is_null() || !(self.in_region)(ptr as usize) {
            return;
        }
        let copy = if let Some(&copy) = self.strings.get(&(ptr as usize, len)) {
            self.tally.shared += 1;
            copy as *mut u8
        } else {
            // One byte at least, so an empty string never gets a null buffer.
            let copy = (self.alloc)(len.max(1));
            // SAFETY: `len` bytes at `ptr` are the string; `copy` is fresh.
            unsafe { core::ptr::copy_nonoverlapping(ptr, copy, len) };
            self.strings.insert((ptr as usize, len), copy as usize);
            self.tally.copied += 1;
            self.tally.bytes += len;
            copy
        };
        // SAFETY: as the read above.
        unsafe { location.cast::<*mut u8>().write_unaligned(copy) };
    }

    unsafe fn copy_block(&mut self, location: *mut u8, node: usize) -> Result<(), MalformedLayout> {
        // SAFETY: a box is one pointer at `location`.
        let block = unsafe { location.cast::<*mut u8>().read_unaligned() };
        if block.is_null() || !(self.in_region)(block as usize) {
            return Ok(());
        }
        if let Some(&copy) = self.blocks.get(&(block as usize)) {
            self.tally.shared += 1;
            // SAFETY: as the read above.
            unsafe { location.cast::<usize>().write_unaligned(copy) };
            return Ok(());
        }
        let alloc_size = self.offset_at(node + 1)?;
        let content = self.word(node + 2)?;
        // SAFETY: the block is valid for its variant's payload (the caller).
        let used = unsafe { self.used_size(block, content, alloc_size)? };
        let copy = (self.alloc)(alloc_size.max(1));
        // SAFETY: `used <= alloc_size` bytes of the source; `copy` is fresh
        // and `alloc_size` long.
        unsafe {
            core::ptr::copy_nonoverlapping(block, copy, used);
            core::ptr::write_bytes(copy.add(used), 0, alloc_size - used);
            location.cast::<*mut u8>().write_unaligned(copy);
        }
        self.blocks.insert(block as usize, copy as usize);
        self.tally.copied += 1;
        self.tally.bytes += used;
        if content != NO_NODE {
            // The copy is walked, so its own pointers are rewritten in place.
            self.stack.push((copy, self.node_at(node + 2)?));
        }
        Ok(())
    }

    /// How many bytes of a block to copy: a tagged block's own variant,
    /// never more than the block's size.
    unsafe fn used_size(
        &self,
        block: *const u8,
        content: u64,
        alloc_size: usize,
    ) -> Result<usize, MalformedLayout> {
        if content == NO_NODE {
            return Ok(alloc_size);
        }
        let content = usize::try_from(content).map_err(|_| MalformedLayout { word: 0 })?;
        if self.word(content)? != KIND_TAGGED {
            return Ok(alloc_size);
        }
        // SAFETY: a tagged block starts with its tag.
        let tag = unsafe { block.cast::<u32>().read_unaligned() } as usize;
        let variants = self.offset_at(content + 2)?;
        if tag >= variants {
            return Ok(alloc_size);
        }
        let payload = self.offset_at(content + 1)?;
        let size = self.offset_at(content + 3 + 2 * tag)?;
        Ok((payload + size).min(alloc_size))
    }
}
