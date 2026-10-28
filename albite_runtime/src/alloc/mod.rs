//! The one allocation symbol compiled programs call (ADR 14.9.26b).
//!
//! Every heap request a compiled Tungsten program makes — closure environment,
//! μ/ADT node, ref cell, string buffer, and the string runtime's C-allocator
//! shim — reaches `__albite_alloc(size, class)`. What it does is decided by a
//! **process-global mode**, read once from `ALBITE_ARENA` by
//! `__albite_arena_init()` in the program prologue (or, in a program that
//! uses regions, `__albite_arena_init_regions()` — ADR 7.10.26s D4):
//!
//! | `ALBITE_ARENA`      | mode     | `__albite_alloc`       | `__albite_realloc`   |
//! |-----------------------|----------|--------------------------|------------------------|
//! | unset / `off`         | `Off`    | `malloc(3)`              | `realloc(3)`           |
//! | `bump` / `bump:<MiB>` | `Bump`   | thread-local bump arena  | `Arena::grow_last`     |
//! | (regions init only)   | `Scoped` | `malloc(3)`              | `realloc(3)`           |
//! `__albite_realloc_amortised` (ADR 16.10.26j, [`amortised`]) is the last
//! column, except that its `realloc(3)` keeps room for the next grows.
//!
//! The mode lives in [`ARENA_MODE`], exported as `__albite_arena_mode`, which generated
//! code also reads at each allocation site (ADR 18.9.26c) so that mode `off`
//! calls `malloc(3)` directly and never enters `__albite_alloc`. `Scoped`
//! exists so a region-using program always reaches the symbol.
//!
//! **The region stack** (ADR 7.10.26s D2, [`region`]) overrides the table: while
//! the calling thread has a region entered, `__albite_alloc` takes from the
//! top region and `__albite_realloc` grows there, in every mode. The
//! region-bypassing pair, `__albite_alloc_parent`/`__albite_realloc_parent`,
//! ignores the stack: a handle's memory never lives in a region (G4).
//!
//! A process in which init never ran — the bootstrap compiler, which links
//! this crate but has no Tungsten prologue — is in mode `Off`, byte-identical
//! to the pre-ADR `malloc` path.
//!
//! The mode is global and only the *arenas* are thread-local, on purpose: a
//! per-thread mode would let a buffer allocated in bump mode on one thread be
//! grown on another through libc `realloc`, which is undefined behaviour. With
//! regions the question is scoped rather than total: **a pointer allocated in a
//! region never outlives it** (G2) — the copy-out (`B1.3`) moves what escapes.
//!
//! Profiler recording (ADR 2.7.26a) is a mode *inside* the symbol, not a
//! different callee, and is gated on [`alloc_profile_is_active`] because
//! `Profiler::record` activates the profiler as a side effect, and activation
//! is what turns on the external-record path and the 1 GiB interim dumps.
//!
//! Out of memory in either mode is one stderr line and `std::process::abort()`:
//! generated code never null-checks the return, and the v2.0 plan forbids
//! unwinding out of the allocator (§9).

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::alloc_profile::{alloc_profile_is_active, alloc_profile_record_external};
use crate::arena::Arena;

mod amortised;
pub mod copy_out;
mod exports;
pub mod region;
mod setting;
mod stats;

pub use amortised::__albite_realloc_amortised;
pub use exports::{
    __albite_alloc_parent, __albite_arena_init_regions, __albite_realloc_parent,
    __albite_region_copy_out, __albite_region_detach, __albite_region_enter,
    __albite_region_release, detached_region_contains,
};
pub use setting::{
    apply_arena_setting, apply_regions_setting, parse_arena_setting, region_poison_requested,
    regions_setting, ArenaSetting, POISON_BYTE, UNRECOGNISED_REGIONS_LINE,
};
pub use stats::{
    __albite_alloc_profile_malloc, __albite_alloc_profile_malloc_class, __albite_arena_stats,
    current_arena_stats, ArenaStatsOut,
};

use region::RegionStack;

/// The allocation mode, chosen once per process.
///
/// Discriminants cross the FFI boundary in `__albite_arena_stats` and MUST
/// match `ArenaMode` in `albite_codegen/src/codegen/data/alloc/mod.rs`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaMode {
    /// Every request goes to the platform `malloc(3)` — today's behaviour.
    Off = 0,
    /// Every request bumps the calling thread's arena.
    Bump = 1,
    /// Always call the symbol; with the region stack empty it calls
    /// `malloc(3)` (ADR 7.10.26s D4, G2a option b).
    Scoped = 2,
}

pub const MODE_OFF: u32 = ArenaMode::Off as u32;
pub const MODE_BUMP: u32 = ArenaMode::Bump as u32;
pub const MODE_SCOPED: u32 = ArenaMode::Scoped as u32;

/// Chunk size when `ALBITE_ARENA=bump` names none: 4 MiB. Large enough
/// that a self-compile's ~1 GB of ~47-byte requests takes a few hundred
/// chunks, small enough that a hello-world reserves little.
pub const DEFAULT_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Chunk size of a region's arena (ADR 7.10.26s D5): measured by
/// `examples/region_chunk_probe.rs` and recorded in
/// `benchmarks/results/7.10.26s.region-runtime-cost.md`. Small regions must
/// not pay the thread arena's 4 MiB, and at 4096 allocations 16 KiB was within
/// 1.4% of the best size; `B1.3`'s RSS measurement may revise it.
pub const REGION_CHUNK_BYTES: usize = 16 * 1024;

/// Alignment every arena block gets — the C allocator's guarantee, which
/// generated code relies on for `{ ptr, i64 }` and 16-byte closure pairs.
const BLOCK_ALIGN: usize = 16;

/// The process-global mode, exported so generated code can branch on it at
/// each allocation site (ADR 18.9.26c): mode `off` then calls `malloc(3)`
/// directly instead of paying a call level through `__albite_alloc`.
///
/// Generated code reads it with a plain `load i32`: `AtomicU32` is four bytes,
/// four-aligned, and the prologue's store precedes the first allocation on the
/// same thread, with nothing storing afterwards. Exported under the symbol
/// name codegen declares, `__albite_arena_mode`.
#[export_name = "__albite_arena_mode"]
pub static ARENA_MODE: AtomicU32 = AtomicU32::new(MODE_OFF);
static CHUNK_BYTES: AtomicUsize = AtomicUsize::new(DEFAULT_CHUNK_BYTES);

/// Set by the first `__albite_region_enter` in the process and never
/// cleared: D6's lever (ADR 7.10.26s). One relaxed load lets a process that
/// never enters a region skip the thread-local stack on every allocation,
/// which measured +11% on `closure_chain` in mode `bump` without it. A thread
/// that entered a region stored the flag itself, so it always sees it.
static REGIONS_ENTERED: AtomicBool = AtomicBool::new(false);

/// One arena per thread. `const`-initialised: an initialiser that allocated
/// would recurse through the allocator it backs.
struct ArenaCell(UnsafeCell<Arena>);

/// One region stack per thread, beside the arena and `const`-initialised for
/// the same reason (ADR 7.10.26s D2).
struct RegionCell(UnsafeCell<RegionStack>);

thread_local! {
    static CURRENT_ARENA: ArenaCell = const { ArenaCell(UnsafeCell::new(Arena::new())) };
    static CURRENT_REGIONS: RegionCell = const { RegionCell(UnsafeCell::new(RegionStack::new())) };
}

/// The current process-global mode.
#[must_use]
#[inline]
pub fn arena_mode() -> ArenaMode {
    match ARENA_MODE.load(Ordering::Relaxed) {
        MODE_BUMP => ArenaMode::Bump,
        MODE_SCOPED => ArenaMode::Scoped,
        _ => ArenaMode::Off,
    }
}

/// Read `ALBITE_ARENA` (or the legacy `TUNGSTEN_ARENA`) once and set the process-global mode.
///
/// Emitted unconditionally in every compiled program's `__albite_inner_main`
/// prologue (the `tg_init_args_c` precedent). Never called by the bootstrap,
/// which therefore stays `Off` even with the variable exported.
#[no_mangle]
pub extern "C" fn __albite_arena_init() {
    let value = crate::settings::read("ARENA");
    apply_arena_setting(parse_arena_setting(value.as_deref()));
}

/// The `malloc(3)` mode `Off` delegates to, on targets that have one.
#[cfg(unix)]
fn platform_malloc(size: usize) -> *mut u8 {
    // SAFETY: `malloc` contract.
    unsafe { libc::malloc(size) }.cast::<u8>()
}

/// `wasm32-unknown-unknown` has no C allocator (ADR 28.7.26a); null is the
/// honest answer, and nothing reaches this arm — its only caller is a
/// codegen-emitted site, and codegen needs LLVM.
#[cfg(not(unix))]
fn platform_malloc(_size: usize) -> *mut u8 {
    core::ptr::null_mut()
}

#[cfg(unix)]
fn platform_realloc(ptr: *mut u8, size: usize) -> *mut u8 {
    // SAFETY: `realloc` contract — `ptr` came from `platform_malloc` or is null.
    unsafe { libc::realloc(ptr.cast(), size) }.cast::<u8>()
}

#[cfg(not(unix))]
fn platform_realloc(_ptr: *mut u8, _size: usize) -> *mut u8 {
    core::ptr::null_mut()
}

/// Return a chunk to the platform: every arena chunk came from
/// [`platform_malloc`].
fn platform_free(base: *mut u8, _len: usize) {
    // SAFETY: every chunk came from `platform_malloc`.
    #[cfg(unix)]
    unsafe {
        libc::free(base.cast());
    }
    #[cfg(not(unix))]
    let _ = base;
}

/// Run `f` on the calling thread's arena.
fn with_thread_arena<R>(f: impl FnOnce(&mut Arena) -> R) -> R {
    // SAFETY: the cell is thread-local and `f` does not re-enter it — the
    // chunk source is the platform allocator, never this symbol.
    CURRENT_ARENA.with(|cell| f(unsafe { &mut *cell.0.get() }))
}

/// Run `f` on the calling thread's region stack.
fn with_thread_regions<R>(f: impl FnOnce(&mut RegionStack) -> R) -> R {
    // SAFETY: as `with_thread_arena` — thread-local, and `f` never re-enters
    // it: chunks come from the platform allocator and the stack's own `Vec`
    // from Rust's global allocator, never `__albite_alloc`.
    CURRENT_REGIONS.with(|cell| f(unsafe { &mut *cell.0.get() }))
}

/// One stderr line, then abort: the v2.0 plan's no-unwinding invariant (§9).
#[cold]
#[inline(never)]
fn abort_out_of_memory(what: &str, bytes: usize) -> ! {
    eprintln!("tungsten runtime: out of memory ({what} of {bytes} bytes)");
    std::process::abort()
}

/// Allocate `size` bytes from the top region when one is entered, else in
/// the current mode, or abort.
///
/// Inlined into the exported symbol on purpose: every generated allocation
/// pays this path, and measured on `closure_chain` (6.9 M allocations) each
/// extra call level in mode `Off` cost ~1–2 ns per allocation on top of
/// `malloc(3)` — AC 4's branch-cost probe.
#[inline(always)] // Reason: measured; `#[inline]` alone left the call level in the release build
#[allow(clippy::inline_always)] // Reason: measured per-allocation call-level cost; see the `#[inline(always)]` above
fn alloc_in_mode(mode: ArenaMode, size: usize) -> *mut u8 {
    let in_region = if REGIONS_ENTERED.load(Ordering::Relaxed) {
        with_thread_regions(|regions| {
            regions.alloc_in_top(size, BLOCK_ALIGN, REGION_CHUNK_BYTES, &mut platform_malloc)
        })
    } else {
        None
    };
    let ptr = in_region.unwrap_or_else(|| alloc_outside_regions(mode, size));
    if allocation_failed(ptr, size) {
        abort_out_of_memory("alloc", size);
    }
    ptr
}

/// Allocate as the mode says, ignoring the region stack — the parent
/// allocator, which `__albite_alloc_parent` reaches directly.
#[inline(always)] // Reason: on every allocation's path; see `alloc_in_mode`
#[allow(clippy::inline_always)] // Reason: see the `#[inline(always)]` above
fn alloc_outside_regions(mode: ArenaMode, size: usize) -> *mut u8 {
    match mode {
        ArenaMode::Off | ArenaMode::Scoped => platform_malloc(size),
        ArenaMode::Bump => with_thread_arena(|arena| {
            let chunk = CHUNK_BYTES.load(Ordering::Relaxed);
            arena.alloc_with(size, BLOCK_ALIGN, chunk, &mut platform_malloc)
        }),
    }
}

/// The allocation symbol every generated allocation site calls.
///
/// The request is recorded against the profiler only when a profiled build
/// has activated it (`alloc_profile_record_external` is that gate), so the
/// aliased profiler symbol no longer self-activates (ADR 14.9.26b §2.2).
///
/// # Safety
///
/// Same contract as `malloc(3)`: the memory is uninitialised. `class` is one of
/// the `CLASS_*` constants (out-of-range values record as `CLASS_OTHER`). In
/// mode `Bump`, or with a region entered, the block must never be passed to
/// `free(3)`; nothing generated does so today.
#[no_mangle]
#[allow(clippy::cast_possible_truncation)] // Reason: u64 -> usize is lossless on the 64-bit targets that link compiled programs; wasm32 compiles but never calls this symbol (ADR 30.9.26l D6)
pub unsafe extern "C" fn __albite_alloc(size: u64, class: u32) -> *mut c_void {
    let ptr = alloc_in_mode(arena_mode(), size as usize);
    if alloc_profile_is_active() {
        alloc_profile_record_external(size, class);
    }
    ptr.cast()
}

/// Resize a block `__albite_alloc` returned from `old` to `new` bytes.
///
/// With a region entered, it grows in the top region (`grow_last`): a block
/// from outside is copied in, never handed to libc. Otherwise mode `Off` and
/// `Scoped` are `realloc(3)`, and mode `Bump` is `Arena::grow_last` on the
/// thread arena: in place when the block is its most recent allocation, else
/// a fresh block plus a copy — so an arena pointer is never handed to libc.
///
/// # Safety
///
/// `ptr` must be null or a block from `__albite_alloc`/`__albite_realloc`
/// on this thread of at least `old` bytes, and is invalidated by this call.
#[no_mangle]
pub unsafe extern "C" fn __albite_realloc(ptr: *mut c_void, old: u64, new: u64) -> *mut c_void {
    // SAFETY: the caller's contract, forwarded.
    unsafe { realloc_with(ptr, old, new, &mut platform_realloc) }
}

/// The region-then-mode branch `__albite_realloc` and
/// `__albite_realloc_amortised` share, so they cannot drift (ADR 16.10.26j
/// D3): only mode `Off`/`Scoped`'s `leaf` differs — `realloc(3)` or amortised.
///
/// # Safety
///
/// As [`__albite_realloc`]; `leaf` must accept any block `malloc` returned.
#[allow(clippy::cast_possible_truncation)] // Reason: lossless on 64-bit link targets; saturating `old` would turn an unreachable truncation into a reachable over-read in bump mode (ADR 30.9.26l D6)
unsafe fn realloc_with(
    ptr: *mut c_void,
    old: u64,
    new: u64,
    leaf: &mut impl FnMut(*mut u8, usize) -> *mut u8,
) -> *mut c_void {
    let (old, new) = (old as usize, new as usize);
    let block = ptr.cast::<u8>();
    let in_region = if REGIONS_ENTERED.load(Ordering::Relaxed) {
        with_thread_regions(|regions| {
            // SAFETY: the caller's contract on `ptr` and `old`.
            unsafe {
                regions.grow_in_top(block, old, new, REGION_CHUNK_BYTES, &mut platform_malloc)
            }
        })
    } else {
        None
    };
    // SAFETY: the caller's contract on `ptr` and `old`.
    let grown =
        in_region.unwrap_or_else(|| unsafe { realloc_outside_regions_with(block, old, new, leaf) });
    if allocation_failed(grown, new) {
        abort_out_of_memory("realloc", new);
    }
    grown.cast()
}

/// Resize as the mode says, ignoring the region stack.
///
/// # Safety
///
/// As [`__albite_realloc`]; the block must not lie in a region.
unsafe fn realloc_outside_regions(block: *mut u8, old: usize, new: usize) -> *mut u8 {
    // SAFETY: the caller's contract, forwarded.
    unsafe { realloc_outside_regions_with(block, old, new, &mut platform_realloc) }
}

/// As [`realloc_outside_regions`], with mode `Off`/`Scoped`'s leaf passed in.
unsafe fn realloc_outside_regions_with(
    block: *mut u8,
    old: usize,
    new: usize,
    leaf: &mut impl FnMut(*mut u8, usize) -> *mut u8,
) -> *mut u8 {
    match arena_mode() {
        ArenaMode::Off | ArenaMode::Scoped => leaf(block, new),
        // A null block is an allocation — `grow_last` treats it so, as
        // `realloc(3)` does.
        ArenaMode::Bump => with_thread_arena(|arena| {
            let chunk = CHUNK_BYTES.load(Ordering::Relaxed);
            // SAFETY: the caller's contract on `ptr` and `old`.
            unsafe { arena.grow_last(block, old, new, chunk, &mut platform_malloc) }
        }),
    }
}

/// Whether a request must abort: the allocator returned null for a non-zero
/// request. A null for zero bytes is a legal `malloc(0)` answer and is handed
/// back as-is.
fn allocation_failed(ptr: *mut u8, size: usize) -> bool {
    ptr.is_null() && size > 0
}

/// Return the calling thread's arena chunks (all but the first) to the
/// platform and rewind. Exported for B1 and wired nowhere: B1 never resets the
/// thread's arena (G7) — a region frees through `__albite_region_release` —
/// and calling it while any block is live is unsound.
#[no_mangle]
pub extern "C" fn __albite_arena_reset() {
    with_thread_arena(|arena| arena.reset(&mut platform_free));
}

#[cfg(test)]
mod copy_out_edge_tests;
#[cfg(test)]
mod copy_out_tests;
#[cfg(test)]
mod region_tests;
#[cfg(test)]
mod tests;
