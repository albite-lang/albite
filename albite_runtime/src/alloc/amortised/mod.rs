//! The core's owned grow: a block that must move takes room for the next
//! grows (ADR 16.10.26j D3).
//!
//! `__albite_realloc_amortised(ptr, old, new)` is `__albite_realloc`
//! everywhere but the one arm that calls `realloc(3)` — mode `Off` or
//! `Scoped` with no region on the thread. There it reads the block's usable
//! size from the allocator's own chunk header and
//!
//! - returns `ptr` and makes no call when the block already holds `new`;
//! - otherwise asks for `max(new, F × usable)`, saturating, where `F` is
//!   [`AMORTISED_GROWTH_FACTOR`];
//! - and when that fails, asks again for exactly `new` before the caller
//!   aborts, so out of memory is no worse than an exact request.
//!
//! The bump arena and the region stack never take this path — they read `old`
//! as the block's true size and grow in place at their cursor — so both share
//! one region-then-mode dispatcher with `__albite_realloc`
//! ([`super::realloc_with`]) and differ only in the leaf.
//!
//! Writing into the usable slack without a `realloc` is what
//! `malloc_usable_size(3)` calls unsupported; ADR 16.10.26j D3 records why it
//! is accepted here (the slack is the chunk's own payload, no compiler sees the
//! request size, and ASan/valgrind degrade the usable size to the request).
//!
//! Only the core's `++` grow calls it (`albite_codegen`'s `bytes/grow.rs`).
//! Rust's `tg_string_concat_owned` keeps exact requests: it is the `off` side
//! of every budgeted pair.

use core::ffi::c_void;

use super::{platform_realloc, realloc_with};

/// A growth factor as the integer ratio `numerator / denominator`, so the
/// Rust policy and `benchmarks/strings/owned_grow_replay.c` round alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrowthFactor {
    pub numerator: usize,
    pub denominator: usize,
}

/// `F`: P0.2's replay qualified 2, 5/2 and 3 under D4's allocator-work rule;
/// 2 missed the owned shape's 1.10× budget and 5/2, D8's remedy, met it
/// (`benchmarks/results/16.10.26j.owned-concat-moves.md`).
pub const AMORTISED_GROWTH_FACTOR: GrowthFactor = GrowthFactor {
    numerator: 5,
    denominator: 2,
};

/// `have × F`, saturating at `usize::MAX` — the replay's `scaled`.
fn scaled(have: usize, factor: GrowthFactor) -> usize {
    have.checked_mul(factor.numerator)
        .map_or(usize::MAX, |product| product / factor.denominator)
}

/// The policy: `None` when a block of usable size `have` already holds `new`
/// bytes, otherwise the size to ask `realloc` for.
#[must_use]
pub fn amortised_request(have: usize, new: usize) -> Option<usize> {
    (new > have).then(|| new.max(scaled(have, AMORTISED_GROWTH_FACTOR)))
}

/// The `Off`/`Scoped` leaf over an injected allocator, so the fits branch,
/// the moving branch, a null block and the out-of-memory retry are tested
/// without depending on a platform's answer.
///
/// A null `block` is an allocation, as `realloc(3)` treats it, and its usable
/// size is never asked: macOS does not document `malloc_size(NULL)`.
///
/// # Safety
///
/// `block` is null or a live block the injected `realloc` accepts, and
/// `usable_size` answers for it.
pub(super) unsafe fn amortised_leaf(
    block: *mut u8,
    new: usize,
    usable_size: &mut impl FnMut(*mut u8) -> usize,
    realloc: &mut impl FnMut(*mut u8, usize) -> *mut u8,
) -> *mut u8 {
    if block.is_null() {
        return realloc(block, new);
    }
    // The one known non-`malloc` pointer that can reach here is a detached
    // region's block through a G2 violation; reading its "chunk header" would
    // let a grow write past it silently, where `realloc(3)` would abort.
    debug_assert!(
        !super::detached_region_contains(block),
        "an amortised grow of a detached region's block"
    );
    let Some(request) = amortised_request(usable_size(block), new) else {
        return block;
    };
    let grown = realloc(block, request);
    // A failed `realloc` leaves `block` valid and unchanged, so an exact
    // request is still possible.
    if grown.is_null() && request != new {
        realloc(block, new)
    } else {
        grown
    }
}

/// The allocator's usable size of a live `malloc` block.
#[cfg(target_os = "linux")]
fn platform_usable_size(block: *mut u8) -> usize {
    // SAFETY: `block` is a live, non-null `malloc` block (`amortised_leaf`).
    unsafe { libc::malloc_usable_size(block.cast()) }
}

#[cfg(target_vendor = "apple")]
fn platform_usable_size(block: *mut u8) -> usize {
    // SAFETY: `block` is a live, non-null `malloc` block (`amortised_leaf`).
    unsafe { libc::malloc_size(block.cast_const().cast()) }
}

/// The leaf on a platform with a usable-size call.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn platform_amortised_leaf(block: *mut u8, new: usize) -> *mut u8 {
    // SAFETY: the export's caller contract — `block` is null or a live block
    // from the C allocator, which `platform_usable_size` answers for.
    unsafe { amortised_leaf(block, new, &mut platform_usable_size, &mut platform_realloc) }
}

/// Every other target (`wasm32` included) keeps exact growth.
#[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
fn platform_amortised_leaf(block: *mut u8, new: usize) -> *mut u8 {
    platform_realloc(block, new)
}

/// Resize a block for the core's owned `++` grow: `__albite_realloc`, except
/// that a `realloc(3)` grow keeps room for the next ones (module doc).
///
/// # Safety
///
/// As [`super::__albite_realloc`]: `ptr` is null or a block from
/// `__albite_alloc`/`__albite_realloc`/this function on this thread of at
/// least `old` bytes, and is invalidated by this call.
#[no_mangle]
pub unsafe extern "C" fn __albite_realloc_amortised(
    ptr: *mut c_void,
    old: u64,
    new: u64,
) -> *mut c_void {
    // SAFETY: the caller's contract, forwarded.
    unsafe { realloc_with(ptr, old, new, &mut platform_amortised_leaf) }
}

#[cfg(test)]
mod tests;
