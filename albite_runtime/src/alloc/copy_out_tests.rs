//! Tests for the region copy-out (ADR 13.10.26q AC 2).
//!
//! Each test builds a value in a real [`RegionStack`] region over an injected
//! chunk source, detaches it, copies it out into an injected parent, then
//! releases the region with a closure that **poisons** each chunk (and keeps
//! it mapped, so a missed copy reads [`POISON_BYTE`] rather than freed
//! memory). Reading the value back intact is the evidence that the copy
//! moved everything the value reaches inside the region.

use super::copy_out::{
    copy_out_with, ChunkIndex, CopyTally, MalformedLayout, KIND_BOX, KIND_STRING, KIND_STRUCT,
    KIND_TAGGED, NO_NODE,
};
use super::region::RegionStack;
use super::{region_poison_requested, POISON_BYTE, REGION_CHUNK_BYTES};

/// The parent's fill: a byte the copy should have overwritten or zeroed
/// shows as this, never as a lucky zero.
pub(super) const PARENT_FILL: u8 = 0xEE;

/// Sixteen bytes, sixteen-aligned: the unit a [`Pool`] block is made of.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct Unit([u8; 16]);

/// 16-aligned blocks starting as one fill byte, owned (and freed) by the pool
/// — a stand-in for both the chunk source and the parent allocator.
#[derive(Default)]
struct Pool {
    fill: u8,
    blocks: Vec<Vec<Unit>>,
}

impl Pool {
    fn filled(fill: u8) -> Self {
        Self {
            fill,
            blocks: Vec::new(),
        }
    }

    fn take(&mut self, size: usize) -> *mut u8 {
        let mut block = vec![Unit([self.fill; 16]); size.max(1).div_ceil(16)];
        // Moving the `Vec` into the pool keeps its buffer where it is.
        let ptr = block.as_mut_ptr().cast::<u8>();
        self.blocks.push(block);
        ptr
    }
}

/// One region over a [`Pool`] of chunks, plus the parent pool.
pub(super) struct Fixture {
    pub(super) regions: RegionStack,
    chunks: Pool,
    parent: Pool,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let mut regions = RegionStack::new();
        regions.enter();
        Self {
            regions,
            chunks: Pool::default(),
            parent: Pool::filled(PARENT_FILL),
        }
    }

    /// Allocate `size` bytes in the open region.
    pub(super) fn in_region(&mut self, size: usize) -> *mut u8 {
        let chunks = &mut self.chunks;
        self.regions
            .alloc_in_top(size, 16, REGION_CHUNK_BYTES, &mut |want| chunks.take(want))
            .expect("a region is entered")
    }

    pub(super) fn string_in_region(&mut self, text: &str) -> (*mut u8, usize) {
        let ptr = self.in_region(text.len());
        // SAFETY: `text.len()` fresh bytes.
        unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), ptr, text.len()) };
        (ptr, text.len())
    }

    /// Detach, copy `slot` out through `layout`, then release with poison.
    pub(super) fn copy_then_release(&mut self, slot: *mut u8, layout: &[u64]) -> CopyTally {
        self.regions.detach().unwrap();
        let index = ChunkIndex::new(self.regions.detached_chunk_spans());
        let parent = &mut self.parent;
        // SAFETY: every fixture builds `slot` to match `layout`.
        let tally = unsafe {
            copy_out_with(slot, layout, &|addr| index.contains(addr), &mut |size| {
                parent.take(size)
            })
        }
        .expect("well-formed layout");
        self.release_with_poison();
        tally
    }

    /// The region released with every chunk poisoned and still mapped.
    pub(super) fn release_with_poison(&mut self) {
        self.regions
            .release(&mut |base, len| {
                // SAFETY: the chunk is `len` bytes at `base`, owned by the pool.
                unsafe { core::ptr::write_bytes(base, POISON_BYTE, len) };
            })
            .unwrap();
    }
}

/// A `{ ptr, i64 }` string slot.
pub(super) fn string_slot(ptr: *mut u8, len: usize) -> [u64; 2] {
    [ptr as u64, len as u64]
}

/// Read a string slot's bytes.
pub(super) fn read_string(slot: *const u8) -> String {
    // SAFETY: a `{ ptr, i64 }` the test built.
    unsafe {
        let ptr = slot.cast::<*const u8>().read_unaligned();
        let len = slot.add(8).cast::<usize>().read_unaligned();
        String::from_utf8_lossy(core::slice::from_raw_parts(ptr, len)).into_owned()
    }
}

/// `[len, root…]` with the length word filled in.
pub(super) fn layout(body: &[u64]) -> Vec<u64> {
    let mut words = vec![body.len() as u64 + 1];
    words.extend_from_slice(body);
    words
}

/// `List<Nat>` as the code generator lowers it: a box of a 20-byte
/// `{ i32, [16 x i8] }` blob whose `Cons` payload is `(i64, ptr)` at 4.
fn list_of_nat_layout() -> Vec<u64> {
    layout(&[
        KIND_BOX,
        20,
        4, // word 1: the list pointer
        KIND_TAGGED,
        4,
        2,
        0,
        NO_NODE,
        16,
        11, // word 4: Nil | Cons
        KIND_STRUCT,
        1,
        8,
        1, // word 11: Cons's tail at payload offset 8
    ])
}

/// The shared nullary `Nil` cell, outside every region.
static NIL_CELL: [u32; 5] = [0; 5];

/// Build `n, n-1, …, 1` in the region, ending at the shared `Nil` cell.
fn build_list(fixture: &mut Fixture, n: u64) -> *mut u8 {
    let mut tail = NIL_CELL.as_ptr().cast::<u8>().cast_mut();
    for value in 1..=n {
        let block = fixture.in_region(20);
        // SAFETY: a fresh 20-byte block.
        unsafe {
            block.cast::<u32>().write_unaligned(1);
            block.add(4).cast::<u64>().write_unaligned(value);
            block.add(12).cast::<*mut u8>().write_unaligned(tail);
        }
        tail = block;
    }
    tail
}

/// Sum a list, failing on a tag that is neither `Nil` nor `Cons` (poison).
fn sum_list(mut node: *const u8) -> u64 {
    let mut total = 0;
    loop {
        // SAFETY: a list block the test built, or its copy.
        let tag = unsafe { node.cast::<u32>().read_unaligned() };
        match tag {
            0 => return total,
            1 => unsafe {
                total += node.add(4).cast::<u64>().read_unaligned();
                node = node.add(12).cast::<*const u8>().read_unaligned();
            },
            other => panic!("read a poisoned block (tag {other:#x})"),
        }
    }
}

// 13.10.26q AC2: a String in the region is copied and reads back intact
// after a poisoned release.
#[test]
fn a_region_string_is_copied_and_survives_release() {
    let mut fixture = Fixture::new();
    let (ptr, len) = fixture.string_in_region("abcdef");
    let mut slot = string_slot(ptr, len);
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &layout(&[KIND_STRING]));
    assert_eq!(read_string(slot.as_ptr().cast()), "abcdef");
    assert_ne!(slot[0], ptr as u64, "the pointer moved");
    assert_eq!(
        tally,
        CopyTally {
            copied: 1,
            bytes: 6,
            shared: 0
        }
    );
}

// 13.10.26q AC2: a pointer outside the detached region (a literal) is kept,
// unmoved, and nothing is copied.
#[test]
fn a_pointer_outside_the_region_is_kept() {
    let mut fixture = Fixture::new();
    let literal = "literal";
    let mut slot = string_slot(literal.as_ptr().cast_mut(), literal.len());
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &layout(&[KIND_STRING]));
    assert_eq!(slot[0], literal.as_ptr() as u64);
    assert_eq!(tally, CopyTally::default());
}

// 13.10.26q AC2: a list in the region is copied node by node; the shared
// Nil cell outside it is kept, so the copy ends at the same cell.
#[test]
fn a_region_list_is_copied_and_ends_at_the_shared_cell() {
    let mut fixture = Fixture::new();
    let head = build_list(&mut fixture, 10);
    let mut slot = [head as u64];
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &list_of_nat_layout());
    assert_eq!(sum_list(slot[0] as *const u8), 55);
    assert_eq!(tally.copied, 10);
    assert_eq!(tally.bytes, 10 * 20);
}

// 13.10.26q AC2: a block holding a short variant is copied at that variant's
// size, never the largest's.
#[test]
fn a_short_variant_is_copied_at_its_own_size() {
    let mut fixture = Fixture::new();
    let nil = fixture.in_region(4);
    let mut slot = [nil as u64];
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &list_of_nat_layout());
    assert_eq!(tally.bytes, 4, "Nil's tag only");
    assert_eq!(sum_list(slot[0] as *const u8), 0);
}

// 13.10.26q AC2: a value reachable twice is copied once, and both
// references point at the one copy — a list and a string each.
#[test]
fn a_value_reachable_twice_is_copied_once() {
    let mut fixture = Fixture::new();
    let head = build_list(&mut fixture, 3);
    let (text, len) = fixture.string_in_region("xy");
    // (List, List, String, String) as a by-value struct.
    let mut slot = [
        head as u64,
        head as u64,
        text as u64,
        len as u64,
        text as u64,
        len as u64,
    ];
    let words = layout(&[
        KIND_STRUCT,
        4,
        0,
        11,
        8,
        11,
        16,
        14,
        32,
        14, // word 1
        KIND_BOX,
        20,
        15,          // word 11
        KIND_STRING, // word 14
        KIND_TAGGED,
        4,
        2,
        0,
        NO_NODE,
        16,
        22, // word 15
        KIND_STRUCT,
        1,
        8,
        11, // word 22
    ]);
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &words);
    assert_eq!(slot[0], slot[1], "one list copy");
    assert_eq!(slot[2], slot[4], "one string copy");
    assert_eq!(sum_list(slot[0] as *const u8), 6);
    assert_eq!(read_string(slot[4..].as_ptr().cast()), "xy");
    assert_eq!(tally.copied, 4, "three list nodes and one string");
    assert_eq!(tally.shared, 2);
}

// 13.10.26q AC2: a tagged value by value (`Result<Nat, String>`'s `Err`)
// copies its arm's string; the `Ok` arm copies nothing.
#[test]
fn a_tagged_value_copies_only_its_arm() {
    let words = layout(&[KIND_TAGGED, 4, 2, 8, NO_NODE, 16, 8, KIND_STRING]);
    let mut fixture = Fixture::new();
    let (text, len) = fixture.string_in_region("odd!");
    let mut slot = [0u8; 24];
    slot[..4].copy_from_slice(&1u32.to_ne_bytes());
    slot[4..12].copy_from_slice(&(text as u64).to_ne_bytes());
    slot[12..20].copy_from_slice(&(len as u64).to_ne_bytes());
    fixture.copy_then_release(slot.as_mut_ptr(), &words);
    assert_eq!(read_string(slot[4..].as_ptr()), "odd!");

    let mut fixture = Fixture::new();
    let _garbage = fixture.string_in_region("ignored");
    let mut ok_slot = [0u8; 24];
    ok_slot[4..12].copy_from_slice(&7u64.to_ne_bytes());
    let tally = fixture.copy_then_release(ok_slot.as_mut_ptr(), &words);
    assert_eq!(tally, CopyTally::default());
}

// 13.10.26q AC2: a 10⁶-node list copies on a thread whose stack is far too
// small for one frame per node — the walk does not recurse.
#[test]
fn a_million_node_list_copies_without_growing_the_stack() {
    let summed = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut fixture = Fixture::new();
            let head = build_list(&mut fixture, 1_000_000);
            let mut slot = [head as u64];
            let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &list_of_nat_layout());
            (sum_list(slot[0] as *const u8), tally.copied)
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(summed, (500_000_500_000, 1_000_000));
}

// 13.10.26q AC2: a layout naming a node past its end, or an unknown kind, is
// refused rather than read.
#[test]
fn a_malformed_layout_is_refused() {
    let mut slot = [0u64; 2];
    let never = |_: usize| false;
    let mut no_alloc = |_: usize| -> *mut u8 { panic!("nothing to copy") };
    // SAFETY: the walk refuses before it reads `slot`'s pointers.
    let unknown =
        unsafe { copy_out_with(slot.as_mut_ptr().cast(), &[2, 99], &never, &mut no_alloc) };
    assert_eq!(unknown, Err(MalformedLayout { word: 1 }));
    let dangling = layout(&[KIND_STRUCT, 1, 0, 40]);
    // SAFETY: as above.
    let dangling =
        unsafe { copy_out_with(slot.as_mut_ptr().cast(), &dangling, &never, &mut no_alloc) };
    assert_eq!(dangling, Err(MalformedLayout { word: 4 }));
}

// 13.10.26q AC2: the chunk index answers membership at both edges of every
// chunk, whatever order the chunks arrive in.
#[test]
fn the_chunk_index_finds_each_chunk_by_its_edges() {
    let index = ChunkIndex::new([(300, 50), (100, 100)]);
    assert!(!index.contains(99));
    assert!(index.contains(100));
    assert!(index.contains(199));
    assert!(!index.contains(200));
    assert!(index.contains(300));
    assert!(index.contains(349));
    assert!(!index.contains(350));
    assert!(!ChunkIndex::default().contains(100));
}

// 13.10.26q AC2: the parent pool starts every block as its fill — the
// contract that lets a missing zero-fill show in the edge tests.
#[test]
fn the_parent_pool_starts_each_block_as_its_fill() {
    let mut pool = Pool::filled(PARENT_FILL);
    let block = pool.take(3);
    // SAFETY: a block is whole 16-byte units.
    let bytes = unsafe { core::slice::from_raw_parts(block, 16) };
    assert!(bytes.iter().all(|&byte| byte == PARENT_FILL));
}

// 13.10.26q D3: ALBITE_REGION_POISON is on for exactly `1`.
#[test]
fn region_poison_is_requested_by_one_only() {
    assert!(region_poison_requested(Some("1")));
    assert!(region_poison_requested(Some(" 1 ")));
    assert!(!region_poison_requested(Some("0")));
    assert!(!region_poison_requested(Some("yes")));
    assert!(!region_poison_requested(None));
}
