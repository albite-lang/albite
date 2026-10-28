//! The arena's statistics export and the profiler's legacy allocation aliases
//! (ADR 14.9.26b, 2.7.26a), moved unchanged from `alloc/mod.rs` to keep it
//! under the file-size threshold (ADR 7.10.26s).

use core::ffi::c_void;

use crate::alloc_profile::CLASS_OTHER;
use crate::arena::ArenaStats;

use super::{__albite_alloc, arena_mode, with_thread_arena, ArenaMode};

/// The C-ABI shape of [`ArenaStats`] plus the mode, for tooling.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArenaStatsOut {
    pub mode: u32,
    pub chunks: u64,
    pub reserved: u64,
    pub used: u64,
    pub high_water: u64,
}

impl ArenaStatsOut {
    fn from_stats(mode: ArenaMode, stats: ArenaStats) -> Self {
        Self {
            mode: mode as u32,
            chunks: stats.chunks as u64,
            reserved: stats.reserved as u64,
            used: stats.used as u64,
            high_water: stats.high_water as u64,
        }
    }
}

/// The calling thread's arena statistics and the process mode.
#[must_use]
pub fn current_arena_stats() -> ArenaStatsOut {
    ArenaStatsOut::from_stats(arena_mode(), with_thread_arena(|arena| arena.stats()))
}

/// Write the calling thread's arena statistics to `out`.
///
/// # Safety
///
/// `out` must be null (a no-op) or valid for a write of `ArenaStatsOut`.
#[no_mangle]
pub unsafe extern "C" fn __albite_arena_stats(out: *mut ArenaStatsOut) {
    if !out.is_null() {
        // SAFETY: the caller's contract on `out`.
        unsafe { out.write(current_arena_stats()) };
    }
}

/// Class-tagged profiling malloc (ADR 2.7.26a), kept as a thin alias of
/// [`__albite_alloc`] for ABI continuity. It no longer self-activates the
/// profiler — recording is gated on an activated profiler, as every other
/// route through the symbol is.
///
/// # Safety
///
/// Same contract as [`__albite_alloc`].
#[no_mangle]
pub unsafe extern "C" fn __albite_alloc_profile_malloc_class(size: u64, class: u32) -> *mut c_void {
    unsafe { __albite_alloc(size, class) }
}

/// Legacy class-blind profiling malloc: records as `other`.
///
/// # Safety
///
/// Same contract as [`__albite_alloc`].
#[no_mangle]
pub unsafe extern "C" fn __albite_alloc_profile_malloc(size: u64) -> *mut c_void {
    unsafe { __albite_alloc(size, CLASS_OTHER) }
}
