//! Runtime support library for compiled Tungsten programs.
//!
//! Provides:
//! - Stack overflow signal handler (SIGSEGV/SIGBUS detection with diagnostics)
//! - `__albite_run_on_stack`, which runs the program body on a thread whose
//!   stack the runtime sizes (ADR 19.10.26c)
//! - The one allocation symbol generated code calls, `__albite_alloc`, with
//!   a bump-pointer arena behind it selected by `ALBITE_ARENA` at start-up
//!   (ADR 14.9.26b) and the class-tagged allocation profiler (ADR 2.7.26a)
//!   as a mode inside it
//! - [`settings`], the one reader of `ALBITE_*` settings, with the legacy
//!   `TUNGSTEN_*` spelling as a fallback (ADR 20.10.26e)
//!
//! This crate is compiled into `albite_core`'s cdylib and linked into every
//! compiled Tungsten program. The signal handler is installed and the arena
//! mode read from the `main()` prologue via `__albite_install_signal_handlers`
//! and `__albite_arena_init`.
//!
//! See ADR 18.4.26g §5 for design rationale.
//!
//! Lint policy: ADR 18.5.26h triaged the crate-level list; ADR 30.9.26l paid
//! it down to `unsafe_code` alone. Every other exemption is a single-line
//! per-item `#[allow]` with a `// Reason:`, pinned by
//! `tests/crate_lint_policy.rs`.

#![allow(unsafe_code)] // Reason: the crate IS the unsafe boundary — malloc, mmap, sigaction and unsafe extern "C" symbols in every working module (ADR 30.9.26l D3)

mod alloc;
mod alloc_profile;
mod arena;
pub mod settings;
mod signal_handler;
mod stack;

// Re-export the public entry points.
// These symbols are called from compiled Tungsten main() prologues.
pub use alloc::copy_out;
pub use alloc::region::{RegionMisuse, RegionStack};
pub use alloc::{
    __albite_alloc, __albite_alloc_parent, __albite_alloc_profile_malloc,
    __albite_alloc_profile_malloc_class, __albite_arena_init, __albite_arena_init_regions,
    __albite_arena_reset, __albite_arena_stats, __albite_realloc, __albite_realloc_amortised,
    __albite_realloc_parent, __albite_region_copy_out, __albite_region_detach,
    __albite_region_enter, __albite_region_release, apply_arena_setting, apply_regions_setting,
    arena_mode, current_arena_stats, detached_region_contains, parse_arena_setting,
    region_poison_requested, regions_setting, ArenaMode, ArenaSetting, ArenaStatsOut,
    DEFAULT_CHUNK_BYTES, MODE_BUMP, MODE_OFF, MODE_SCOPED, POISON_BYTE, REGION_CHUNK_BYTES,
    UNRECOGNISED_REGIONS_LINE,
};
pub use alloc_profile::{
    __albite_alloc_profile_marker, __albite_alloc_profile_report,
    __albite_alloc_profile_set_filter, __albite_alloc_profile_set_fn, alloc_profile_is_active,
    alloc_profile_record_external, CLASS_ENV, CLASS_MU, CLASS_MU_NULLARY, CLASS_OTHER, CLASS_REF,
    CLASS_STRING,
};
pub use arena::{Arena, ArenaStats};
pub use signal_handler::__albite_install_signal_handlers;
pub use stack::{
    __albite_run_on_stack, run_on_stack_with, stack_plan, ProgramBody, StackRequest,
    StackSizeError, DEFAULT_STACK_BYTES, MAX_STACK_BYTES, MIN_STACK_BYTES, STACK_GUARD_BYTES,
    STACK_SIZE_ENV,
};
