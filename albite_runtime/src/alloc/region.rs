//! The region stack (ADR 7.10.26s D2, G2 of region-scoped deallocation).
//!
//! Each thread holds a [`RegionStack`]: a stack of [`Arena`]s, one per entered
//! region, plus one **detached** slot. While the stack is non-empty every
//! region-aware allocation takes from the top region; detach pops the top into
//! the slot, so allocation targets the parent again while the detached chunks
//! stay readable for G3's copy; release returns every one of the slot's chunks.
//!
//! One slot is enough: G7's exits are strictly nested, and the copy between
//! detach and release enters no region. Detaching while the slot is full,
//! detaching an empty stack, or releasing an empty slot is a [`RegionMisuse`],
//! which this pure layer *reports* and the C exports turn into one stderr line
//! and `abort()` (26a §9).
//!
//! Like `Arena`, the stack owns no allocator: chunk source and release are
//! injected, so its tests drive every path with counting closures and no
//! process globals.

use crate::arena::Arena;

/// What a misused region export did — a bug in generated code, never a
/// recoverable condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionMisuse {
    /// `detach` with no region entered.
    DetachWithEmptyStack,
    /// `detach` while the previous detached region has not been released.
    DetachWithSlotFull,
    /// `release` with nothing detached.
    ReleaseWithEmptySlot,
}

impl RegionMisuse {
    /// The stderr line the export prints before aborting.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::DetachWithEmptyStack => {
                "tungsten runtime: region misuse (detach with no region entered)"
            }
            Self::DetachWithSlotFull => {
                "tungsten runtime: region misuse (detach while a detached region is unreleased)"
            }
            Self::ReleaseWithEmptySlot => {
                "tungsten runtime: region misuse (release with no region detached)"
            }
        }
    }
}

/// A thread's entered regions and its one detached region.
///
/// `const`-constructible so it can sit in a `const {}`-initialised
/// thread-local beside the thread arena, for the same reason.
#[derive(Debug)]
pub struct RegionStack {
    entered: Vec<Arena>,
    detached: Option<Arena>,
}

impl Default for RegionStack {
    fn default() -> Self {
        Self::new()
    }
}

impl RegionStack {
    /// No region entered, nothing detached.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entered: Vec::new(),
            detached: None,
        }
    }

    /// How many regions are entered.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.entered.len()
    }

    /// Push a region. Its arena holds no chunk until its first allocation, so
    /// a region that allocates nothing costs no chunk.
    pub fn enter(&mut self) {
        self.entered.push(Arena::new());
    }

    /// Pop the top region into the detached slot.
    ///
    /// # Errors
    ///
    /// [`RegionMisuse::DetachWithSlotFull`] when the slot is occupied (checked
    /// first: the stack is left untouched), [`RegionMisuse::DetachWithEmptyStack`]
    /// when no region is entered.
    pub fn detach(&mut self) -> Result<(), RegionMisuse> {
        if self.detached.is_some() {
            return Err(RegionMisuse::DetachWithSlotFull);
        }
        let top = self
            .entered
            .pop()
            .ok_or(RegionMisuse::DetachWithEmptyStack)?;
        self.detached = Some(top);
        Ok(())
    }

    /// Return every chunk of the detached region to `release` and empty the
    /// slot.
    ///
    /// # Errors
    ///
    /// [`RegionMisuse::ReleaseWithEmptySlot`] when nothing is detached.
    pub fn release(
        &mut self,
        release: &mut impl FnMut(*mut u8, usize),
    ) -> Result<(), RegionMisuse> {
        let mut region = self
            .detached
            .take()
            .ok_or(RegionMisuse::ReleaseWithEmptySlot)?;
        region.release_all(release);
        Ok(())
    }

    /// Allocate from the top region, or `None` when no region is entered —
    /// the caller then allocates as its mode says. `Some(null)` only when the
    /// source fails; the stack is unchanged either way.
    pub fn alloc_in_top(
        &mut self,
        size: usize,
        align: usize,
        chunk_size: usize,
        source: &mut impl FnMut(usize) -> *mut u8,
    ) -> Option<*mut u8> {
        let top = self.entered.last_mut()?;
        Some(top.alloc_with(size, align, chunk_size, source))
    }

    /// Grow `ptr` in the top region, or `None` when no region is entered.
    ///
    /// A pointer from outside the region fails `can_grow_in_place` against
    /// the region's cursor, so `grow_last` copies it in and never hands it to
    /// the platform's `realloc`.
    ///
    /// # Safety
    ///
    /// `ptr` must be null or valid for reads of `old` bytes, and — when it came
    /// from the top region — must satisfy [`Arena::grow_last`]'s contract.
    pub unsafe fn grow_in_top(
        &mut self,
        ptr: *mut u8,
        old: usize,
        new: usize,
        chunk_size: usize,
        source: &mut impl FnMut(usize) -> *mut u8,
    ) -> Option<*mut u8> {
        let top = self.entered.last_mut()?;
        // SAFETY: the caller's contract on `ptr` and `old`.
        Some(unsafe { top.grow_last(ptr, old, new, chunk_size, source) })
    }

    /// The top region, if any — for tests and the copy-out's queries.
    #[must_use]
    pub fn top(&self) -> Option<&Arena> {
        self.entered.last()
    }

    /// The detached region, if any.
    #[must_use]
    pub fn detached(&self) -> Option<&Arena> {
        self.detached.as_ref()
    }

    /// The detached region's chunks as `(base, len)`, for the copy-out's
    /// index (ADR 13.10.26q D3); empty with the slot empty.
    #[must_use]
    pub fn detached_chunk_spans(&self) -> Vec<(usize, usize)> {
        self.detached
            .as_ref()
            .map(|region| region.chunk_spans().collect())
            .unwrap_or_default()
    }

    /// Whether `ptr` lies in the detached region's chunks; `false` with the
    /// slot empty.
    #[must_use]
    pub fn detached_contains(&self, ptr: *const u8) -> bool {
        self.detached
            .as_ref()
            .is_some_and(|region| region.contains(ptr))
    }
}
