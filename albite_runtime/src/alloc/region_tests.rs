//! ADR 7.10.26s AC 2: the region stack over an injected counting chunk source
//! and release, as `arena/tests.rs` drives `Arena` — no process globals, so
//! these tests need no mode lock.

use super::region::{RegionMisuse, RegionStack};
use crate::arena::Arena;
use std::cell::RefCell;

const CHUNK: usize = 256;
const ALIGN: usize = 16;

/// A chunk source and release that serve from libc and record every call.
struct CountingChunks {
    taken: RefCell<Vec<(usize, usize)>>,
    released: RefCell<Vec<(usize, usize)>>,
    /// Requests at or beyond this index fail (return null).
    fail_from: usize,
}

impl CountingChunks {
    fn new() -> Self {
        Self::failing_after(usize::MAX)
    }

    fn failing_after(count: usize) -> Self {
        Self {
            taken: RefCell::new(Vec::new()),
            released: RefCell::new(Vec::new()),
            fail_from: count,
        }
    }

    fn source(&self) -> impl FnMut(usize) -> *mut u8 + '_ {
        move |size| {
            if self.taken.borrow().len() >= self.fail_from {
                return std::ptr::null_mut();
            }
            // SAFETY: `malloc` contract; released through `release`.
            let base = unsafe { libc::malloc(size) }.cast::<u8>();
            self.taken.borrow_mut().push((base as usize, size));
            base
        }
    }

    fn release(&self) -> impl FnMut(*mut u8, usize) + '_ {
        move |base, len| {
            self.released.borrow_mut().push((base as usize, len));
            // SAFETY: every chunk came from `source`.
            unsafe { libc::free(base.cast()) };
        }
    }

    fn taken_count(&self) -> usize {
        self.taken.borrow().len()
    }
}

/// Allocate `size` bytes in the top region, which must exist.
fn alloc(stack: &mut RegionStack, chunks: &CountingChunks, size: usize) -> *mut u8 {
    stack
        .alloc_in_top(size, ALIGN, CHUNK, &mut chunks.source())
        .expect("a region is entered")
}

/// Detach and release every region still entered, so a test leaks nothing.
fn unwind(stack: &mut RegionStack, chunks: &CountingChunks) {
    while stack.depth() > 0 {
        stack.detach().expect("detach");
        stack.release(&mut chunks.release()).expect("release");
    }
}

#[test]
fn entering_takes_no_chunk_until_the_first_allocation() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    stack.enter();
    assert_eq!(stack.depth(), 1);
    assert_eq!(chunks.taken_count(), 0);
    assert!(stack.top().is_some_and(Arena::is_chunkless));
    let _ = alloc(&mut stack, &chunks, 24);
    assert_eq!(chunks.taken_count(), 1);
    assert!(stack.top().is_some_and(|top| !top.is_chunkless()));
    unwind(&mut stack, &chunks);
}

#[test]
fn a_default_stack_is_an_empty_new_one() {
    let stack = RegionStack::default();
    assert_eq!(stack.depth(), 0);
    assert!(stack.top().is_none());
    assert!(stack.detached().is_none());
}

#[test]
fn an_empty_stack_allocates_nothing_and_says_so() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    assert!(stack
        .alloc_in_top(24, ALIGN, CHUNK, &mut chunks.source())
        .is_none());
    // SAFETY: no region is entered, so nothing is read.
    let grown =
        unsafe { stack.grow_in_top(core::ptr::null_mut(), 0, 8, CHUNK, &mut chunks.source()) };
    assert!(grown.is_none());
    assert_eq!(chunks.taken_count(), 0);
}

#[test]
fn an_allocation_comes_from_the_top_region_and_a_nested_one_from_the_inner() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    stack.enter();
    let outer = alloc(&mut stack, &chunks, 24);
    stack.enter();
    let inner = alloc(&mut stack, &chunks, 24);
    assert_eq!(stack.depth(), 2);
    assert!(stack.top().is_some_and(|top| top.contains(inner)));
    assert!(stack.top().is_some_and(|top| !top.contains(outer)));
    stack.detach().expect("detach inner");
    assert!(stack.detached_contains(inner));
    assert!(!stack.detached_contains(outer));
    assert!(stack.top().is_some_and(|top| top.contains(outer)));
    stack.release(&mut chunks.release()).expect("release inner");
    unwind(&mut stack, &chunks);
}

#[test]
fn after_detach_allocation_targets_the_parent_and_the_detached_bytes_still_read() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    stack.enter();
    let parent_block = alloc(&mut stack, &chunks, 8);
    stack.enter();
    let block = alloc(&mut stack, &chunks, 8);
    // SAFETY: an 8-byte region block.
    unsafe { block.write_bytes(0x5a, 8) };
    stack.detach().expect("detach");
    let after = alloc(&mut stack, &chunks, 8);
    assert!(stack.top().is_some_and(|top| top.contains(after)));
    assert!(stack.top().is_some_and(|top| top.contains(parent_block)));
    assert!(!stack.detached_contains(after));
    // SAFETY: detached chunks are not released until `release`.
    assert!(unsafe { std::slice::from_raw_parts(block, 8) }
        .iter()
        .all(|&b| b == 0x5a));
    stack.release(&mut chunks.release()).expect("release");
    unwind(&mut stack, &chunks);
}

#[test]
fn release_returns_every_detached_chunk_the_first_included_and_no_other() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    stack.enter();
    let _parent = alloc(&mut stack, &chunks, 8);
    let parent_chunks = chunks.taken.borrow().clone();
    stack.enter();
    for _ in 0..3 {
        // Each 200-byte request overflows a 256-byte chunk: three chunks.
        let _ = alloc(&mut stack, &chunks, 200);
    }
    let region_chunks: Vec<_> = chunks.taken.borrow()[parent_chunks.len()..].to_vec();
    assert_eq!(region_chunks.len(), 3);
    stack.detach().expect("detach");
    stack.release(&mut chunks.release()).expect("release");
    assert_eq!(*chunks.released.borrow(), region_chunks);
    assert!(stack.detached().is_none());
    assert_eq!(stack.depth(), 1, "the parent is still entered");
    unwind(&mut stack, &chunks);
    assert_eq!(chunks.released.borrow()[3..], parent_chunks[..]);
}

#[test]
fn realloc_of_an_outside_pointer_copies_it_in_and_never_calls_platform_realloc() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    let mut outside = [0x11_u8; 16];
    stack.enter();
    let _ = alloc(&mut stack, &chunks, 8);
    let platform_reallocs = RefCell::new(0_usize);
    // The source spy: only a fresh chunk would call it, and a 64-byte grow
    // fits the 256-byte chunk — so no call at all is the proof nothing went
    // to a platform resize.
    let mut spy = |size: usize| {
        *platform_reallocs.borrow_mut() += 1;
        chunks.source()(size)
    };
    // SAFETY: `outside` is valid for 16 bytes and is not in the region.
    let grown = unsafe { stack.grow_in_top(outside.as_mut_ptr(), 16, 64, CHUNK, &mut spy) }
        .expect("a region is entered");
    assert_eq!(*platform_reallocs.borrow(), 0);
    assert_ne!(grown, outside.as_mut_ptr());
    assert!(stack.top().is_some_and(|top| top.contains(grown)));
    // SAFETY: 16 bytes were copied in.
    assert!(unsafe { std::slice::from_raw_parts(grown, 16) }
        .iter()
        .all(|&b| b == 0x11));
    unwind(&mut stack, &chunks);
}

#[test]
fn grow_of_an_outside_pointer_in_a_chunkless_region_allocates_and_copies() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    let mut outside = [0x22_u8; 8];
    stack.enter();
    // SAFETY: `outside` is valid for 8 bytes.
    let grown =
        unsafe { stack.grow_in_top(outside.as_mut_ptr(), 8, 32, CHUNK, &mut chunks.source()) }
            .expect("a region is entered");
    assert_eq!(chunks.taken_count(), 1, "the first chunk");
    assert!(stack.top().is_some_and(|top| top.contains(grown)));
    // SAFETY: 8 bytes were copied in.
    assert!(unsafe { std::slice::from_raw_parts(grown, 8) }
        .iter()
        .all(|&b| b == 0x22));
    unwind(&mut stack, &chunks);
}

#[test]
fn each_misuse_is_reported_by_the_pure_layer() {
    let chunks = CountingChunks::new();
    let mut stack = RegionStack::new();
    assert_eq!(stack.detach(), Err(RegionMisuse::DetachWithEmptyStack));
    assert_eq!(
        stack.release(&mut chunks.release()),
        Err(RegionMisuse::ReleaseWithEmptySlot)
    );
    stack.enter();
    stack.enter();
    stack.detach().expect("first detach");
    assert_eq!(stack.detach(), Err(RegionMisuse::DetachWithSlotFull));
    assert_eq!(stack.depth(), 1, "a refused detach leaves the stack alone");
    stack.release(&mut chunks.release()).expect("release");
    unwind(&mut stack, &chunks);
    for misuse in [
        RegionMisuse::DetachWithEmptyStack,
        RegionMisuse::DetachWithSlotFull,
        RegionMisuse::ReleaseWithEmptySlot,
    ] {
        assert!(misuse
            .message()
            .starts_with("tungsten runtime: region misuse ("));
    }
    assert_ne!(
        RegionMisuse::DetachWithSlotFull.message(),
        RegionMisuse::ReleaseWithEmptySlot.message()
    );
}

#[test]
fn a_failing_source_yields_null_and_leaves_the_stack_consistent() {
    let chunks = CountingChunks::failing_after(0);
    let mut stack = RegionStack::new();
    stack.enter();
    let ptr = alloc(&mut stack, &chunks, 24);
    assert!(ptr.is_null());
    assert_eq!(stack.depth(), 1);
    assert!(stack.top().is_some_and(Arena::is_chunkless));
    stack.detach().expect("detach");
    stack.release(&mut chunks.release()).expect("release");
    assert!(chunks.released.borrow().is_empty());
}

#[test]
fn contains_is_half_open_over_each_chunk() {
    use crate::arena::chunk_contains;
    assert!(chunk_contains(100, 16, 100));
    assert!(chunk_contains(100, 16, 115));
    assert!(!chunk_contains(100, 16, 116));
    assert!(!chunk_contains(100, 16, 99));
    assert!(!chunk_contains(100, 0, 100));
    let arena = Arena::new();
    assert!(!arena.contains(core::ptr::null()));
}
