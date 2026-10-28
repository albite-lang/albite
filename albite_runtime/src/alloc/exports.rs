//! The region exports (ADR 7.10.26s D2–D4): enter, detach and release as thin
//! wrappers binding [`RegionStack`](super::region::RegionStack) to the thread
//! and to `malloc(3)`/`free(3)`; the region-bypassing alloc/realloc pair; and
//! the prologue a region-using program calls. Split from `alloc/mod.rs` to keep
//! it under the file-size threshold.
//!
//! `B1.2` wires enter, detach and release; `B1.3` adds the copy-out between
//! the last two (ADR 13.10.26q).

use core::ffi::c_void;
use core::sync::atomic::Ordering;

use crate::alloc_profile::{alloc_profile_is_active, alloc_profile_record_external, CLASS_OTHER};

use super::copy_out::{copy_out_with, ChunkIndex};
use super::region::RegionMisuse;
use super::setting::{
    apply_regions_setting, parse_arena_setting, read_region_poison, region_poison_enabled,
    POISON_BYTE,
};
use super::{
    abort_out_of_memory, alloc_outside_regions, allocation_failed, arena_mode, platform_free,
    realloc_outside_regions, with_thread_regions, REGIONS_ENTERED,
};

/// One stderr line, then abort: a misused region export is a code-generator
/// bug, and the v2.0 plan forbids unwinding (§9).
#[cold]
#[inline(never)]
fn abort_region_misuse(misuse: RegionMisuse) -> ! {
    eprintln!("{}", misuse.message());
    std::process::abort()
}

/// Push a region on the calling thread's stack. It takes no chunk until its
/// first allocation. The first call in the process also sets the flag that
/// makes `__albite_alloc` consult the stack at all (D6's lever).
#[no_mangle]
pub extern "C" fn __albite_region_enter() {
    REGIONS_ENTERED.store(true, Ordering::Relaxed);
    with_thread_regions(super::region::RegionStack::enter);
}

/// Pop the top region into the detached slot: allocation targets the parent
/// again while the detached chunks stay readable for the copy-out (G7:
/// detach, copy, release). Misuse aborts.
#[no_mangle]
pub extern "C" fn __albite_region_detach() {
    if let Err(misuse) = with_thread_regions(super::region::RegionStack::detach) {
        abort_region_misuse(misuse);
    }
}

/// Return every chunk of the detached region to the platform and empty the
/// slot. Misuse aborts. Under `ALBITE_REGION_POISON=1` each chunk is filled
/// with [`POISON_BYTE`] first, so a missing copy-out reads garbage
/// deterministically instead of the bytes `free(3)` happened to leave (ADR
/// 13.10.26q D3).
#[no_mangle]
pub extern "C" fn __albite_region_release() {
    let poison = region_poison_enabled();
    let released = with_thread_regions(|regions| {
        regions.release(&mut |base, len| {
            if poison {
                // SAFETY: the chunk is `len` bytes at `base` and is about to be
                // freed; nothing reads it again through the region.
                unsafe { core::ptr::write_bytes(base, POISON_BYTE, len) };
            }
            platform_free(base, len);
        })
    });
    if let Err(misuse) = released {
        abort_region_misuse(misuse);
    }
}

/// Copy everything the value in `slot` reaches inside the detached region
/// into the parent allocator, rewriting the pointers in place (ADR 13.10.26q
/// D3). Called between detach and release; `layout` is a static
/// `__tg_region_layout.<fp>` constant whose word 0 is its own length. A
/// malformed layout is a code-generator bug: one stderr line, then abort.
///
/// # Safety
///
/// `slot` holds a value of the type `layout` describes, and the detached
/// region has not been released.
#[no_mangle]
#[allow(clippy::cast_possible_truncation)] // Reason: lossless on the 64-bit link targets, as `__albite_alloc` (ADR 30.9.26l D6)
pub unsafe extern "C" fn __albite_region_copy_out(slot: *mut c_void, layout: *const u64) {
    // SAFETY: word 0 of a layout constant is its length in words.
    let words = unsafe { core::slice::from_raw_parts(layout, *layout as usize) };
    let index = with_thread_regions(|regions| ChunkIndex::new(regions.detached_chunk_spans()));
    // The parent: after detach, `__albite_alloc` targets the enclosing
    // region, the thread arena or `malloc(3)`. Called outside the region
    // borrow above, which it would re-enter.
    let mut alloc_in_parent = |size: usize| {
        // SAFETY: `__albite_alloc`'s contract; the walk writes every byte.
        unsafe { super::__albite_alloc(size as u64, CLASS_OTHER) }.cast::<u8>()
    };
    // SAFETY: the caller's contract on `slot`; the region is detached, not
    // released, so every block it reaches is readable.
    let copied = unsafe {
        copy_out_with(
            slot.cast(),
            words,
            &|addr| index.contains(addr),
            &mut alloc_in_parent,
        )
    };
    if let Err(malformed) = copied {
        eprintln!(
            "tungsten runtime: malformed region copy layout (word {})",
            malformed.word
        );
        std::process::abort();
    }
}

/// Whether `ptr` lies in the calling thread's detached region — a Rust
/// query, not a C export, for other crates' tests (AC 4) and the copy-out
/// (`B1.3`).
#[must_use]
pub fn detached_region_contains(ptr: *const u8) -> bool {
    with_thread_regions(|regions| regions.detached_contains(ptr))
}

/// Allocate ignoring the region stack (ADR 7.10.26s D3): the thread arena in
/// mode `Bump`, otherwise `malloc(3)`. Records against an active profiler as
/// `__albite_alloc` does.
///
/// # Safety
///
/// As `__albite_alloc`.
#[no_mangle]
#[allow(clippy::cast_possible_truncation)] // Reason: lossless on the 64-bit link targets, as `__albite_alloc` (ADR 30.9.26l D6)
pub unsafe extern "C" fn __albite_alloc_parent(size: u64, class: u32) -> *mut c_void {
    let size_bytes = size as usize;
    let ptr = alloc_outside_regions(arena_mode(), size_bytes);
    if allocation_failed(ptr, size_bytes) {
        abort_out_of_memory("alloc", size_bytes);
    }
    if alloc_profile_is_active() {
        alloc_profile_record_external(size, class);
    }
    ptr.cast()
}

/// Resize a block from [`__albite_alloc_parent`] ignoring the region stack:
/// `Arena::grow_last` on the thread arena in mode `Bump`, otherwise
/// `realloc(3)`. So a buffer grown while a region is entered stays outside it
/// — the reverse route G4 closes.
///
/// # Safety
///
/// `ptr` must be null or a block from the parent pair on this thread of at
/// least `old` bytes, and is invalidated by this call.
#[no_mangle]
#[allow(clippy::cast_possible_truncation)] // Reason: lossless on the 64-bit link targets, as `__albite_realloc` (ADR 30.9.26l D6)
pub unsafe extern "C" fn __albite_realloc_parent(
    ptr: *mut c_void,
    old: u64,
    new: u64,
) -> *mut c_void {
    let (old, new) = (old as usize, new as usize);
    // SAFETY: the caller's contract on `ptr` and `old`.
    let grown = unsafe { realloc_outside_regions(ptr.cast(), old, new) };
    if allocation_failed(grown, new) {
        abort_out_of_memory("realloc", new);
    }
    grown.cast()
}

/// Read `ALBITE_ARENA` for a program that uses regions (ADR 7.10.26s D4):
/// `bump[:n]` selects `Bump`; unset, `off` and an unrecognised value select
/// `Scoped`, so every allocation reaches the symbol that consults the stack.
#[no_mangle]
pub extern "C" fn __albite_arena_init_regions() {
    let value = crate::settings::read("ARENA");
    apply_regions_setting(parse_arena_setting(value.as_deref()));
    read_region_poison(crate::settings::read("REGION_POISON").as_deref());
}
