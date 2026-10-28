//! Allocation profiler for compiled Tungsten programs.
//!
//! Provides per-function AND per-class attribution of cumulative allocated
//! bytes. When enabled via `--alloc-profile`, codegen emits a function-entry
//! hook that sets the current function name; every allocation site already
//! calls the class-tagged `__albite_alloc` (ADR 14.9.26b), which records
//! here once a profiled function entry has activated the profiler. At program
//! exit, a sorted report is printed. Interim snapshots are printed every
//! `ALBITE_ALLOC_PROFILE_INTERVAL_MB` (default 1024) so OOM-killed runs
//! still yield a ranking (ADR 2.7.26a P0 — the L3 check dies to SIGKILL,
//! which cannot be caught).
//!
//! See ADR 7.5.26b (base profiler) and ADR 2.7.26a §3 (class fidelity).

use core::cell::UnsafeCell;
use core::ffi::c_char;
use core::ptr;

mod report;

/// Maximum number of distinct functions we track.
/// Uses a fixed array to avoid allocating (which would recurse into the profiler).
const MAX_FUNCTIONS: usize = 4096;

// Allocation classes (ADR 2.7.26a §3 P0 fidelity requirement).
// MUST match `AllocClass` in albite_codegen/src/codegen/data/mod.rs.
/// Recursive-ADT node allocation (`fold_to_heap` / `mu_alloc`) whose value is
/// not an empty constructor — or any fold in a non-profiled build.
pub const CLASS_MU: u32 = 0;
/// Closure environment allocation (`env_alloc`).
pub const CLASS_ENV: u32 = 1;
/// Mutable reference cell (`ref.new`).
pub const CLASS_REF: u32 = 2;
/// String buffer (codegen `malloc_bytes` + `albite_core` string FFI).
pub const CLASS_STRING: u32 = 3;
/// Recursive-ADT fold whose value is a constructor with an empty payload —
/// stamped by profiled builds only (ADR 29.9.26h D1/D2). `mu + nullary` is
/// what `mu` counted before that ADR.
pub const CLASS_MU_NULLARY: u32 = 4;
/// Anything not otherwise classified. Kept LAST: `record` clamps an
/// out-of-range class to the final index, which must stay `other`.
pub const CLASS_OTHER: u32 = 5;
pub const NUM_CLASSES: usize = 6;

pub(crate) const CLASS_NAMES: [&str; NUM_CLASSES] = [
    "mu_alloc (ADT)",
    "env_alloc (closure)",
    "ref_new",
    "string",
    "mu_alloc nullary (ADT)",
    "other",
];

/// Short keys for one-line class summaries, in class order. Each is found by
/// `key=` substring in marker lines (`selfcompiled-profile`), so no key may
/// contain another.
pub(crate) const CLASS_SHORT: [&str; NUM_CLASSES] = ["mu", "env", "ref", "str", "nullary", "other"];

/// Whether an allocation of `class` is a heap fold, and so feeds the
/// `mu_alloc` size histogram. Both fold classes do, so the histogram stays
/// comparable with profiles taken before the nullary split (ADR 29.9.26h D7).
pub(crate) fn is_fold_class(class: u32) -> bool {
    class == CLASS_MU || class == CLASS_MU_NULLARY
}

/// Size-bucket upper bounds (bytes) for the `mu_alloc` histogram.
/// Small buckets ≈ list spines / small nodes; large ≈ by-value payload copies.
pub(crate) const MU_BUCKET_BOUNDS: [u64; 5] = [16, 32, 64, 128, 256];
pub(crate) const NUM_MU_BUCKETS: usize = 6; // 5 bounded + 1 overflow

/// Default interim-dump interval: 1 GiB of cumulative allocation.
const DEFAULT_DUMP_INTERVAL_BYTES: u64 = 1024 * 1024 * 1024;

/// A single profiler entry: function name pointer + cumulative bytes.
struct ProfileEntry {
    name: *const c_char,
    bytes: u64,
    count: u64,
    bytes_by_class: [u64; NUM_CLASSES],
}

/// Wrapper for global profiler state.
///
/// Tungsten programs are single-threaded, so no synchronization is needed.
/// `UnsafeCell` avoids the Rust 2024 `static mut` deprecation while making
/// the single-writer invariant explicit.
struct ProfilerCell(UnsafeCell<Profiler>);

// SAFETY: Tungsten programs are single-threaded. No concurrent access.
unsafe impl Sync for ProfilerCell {}

static PROFILER: ProfilerCell = ProfilerCell(UnsafeCell::new(Profiler::new()));

struct Profiler {
    current_fn: *const c_char,
    filter_fn: *const c_char,
    entries: [ProfileEntry; MAX_FUNCTIONS],
    entry_count: usize,
    /// Cache of the last-touched entry index — allocations cluster by
    /// function, so this makes the common case O(1) instead of a linear scan.
    last_entry: usize,
    total_bytes: u64,
    total_count: u64,
    class_bytes: [u64; NUM_CLASSES],
    class_count: [u64; NUM_CLASSES],
    mu_size_hist: [u64; NUM_MU_BUCKETS],
    /// True once any profiler hook has fired — gates external (Rust-side)
    /// recording so non-profiled builds pay only a branch.
    active: bool,
    /// Cumulative-bytes threshold for the next interim dump (0 = disabled).
    next_dump_bytes: u64,
    dump_interval: u64,
    dump_interval_initialized: bool,
}

impl Profiler {
    #[allow(clippy::large_stack_arrays)] // Reason: a const fn evaluated once into the PROFILER static; it never runs on a stack (ADR 30.9.26l D6)
    const fn new() -> Self {
        const EMPTY: ProfileEntry = ProfileEntry {
            name: ptr::null(),
            bytes: 0,
            count: 0,
            bytes_by_class: [0; NUM_CLASSES],
        };
        Self {
            current_fn: ptr::null(),
            filter_fn: ptr::null(),
            entries: [EMPTY; MAX_FUNCTIONS],
            entry_count: 0,
            last_entry: 0,
            total_bytes: 0,
            total_count: 0,
            class_bytes: [0; NUM_CLASSES],
            class_count: [0; NUM_CLASSES],
            mu_size_hist: [0; NUM_MU_BUCKETS],
            active: false,
            next_dump_bytes: 0,
            dump_interval: 0,
            dump_interval_initialized: false,
        }
    }

    /// Find or create an entry for the given function name.
    /// Comparison is by pointer equality (all names are string literals in .rodata).
    fn find_or_create(&mut self, name: *const c_char) -> Option<usize> {
        // Fast path: same function as the previous allocation.
        if self.entry_count > 0 && self.entries[self.last_entry].name == name {
            return Some(self.last_entry);
        }
        for i in 0..self.entry_count {
            if self.entries[i].name == name {
                self.last_entry = i;
                return Some(i);
            }
        }
        if self.entry_count < MAX_FUNCTIONS {
            let idx = self.entry_count;
            self.entries[idx].name = name;
            self.entry_count += 1;
            self.last_entry = idx;
            Some(idx)
        } else {
            None
        }
    }

    /// Record an allocation of `size` bytes in `class` under the current function.
    fn record(&mut self, size: u64, class: u32) {
        self.active = true;
        self.total_bytes += size;
        self.total_count += 1;

        let class_idx = (class as usize).min(NUM_CLASSES - 1);
        self.class_bytes[class_idx] += size;
        self.class_count[class_idx] += 1;
        if is_fold_class(class) {
            self.mu_size_hist[mu_bucket(size)] += 1;
        }

        if !self.current_fn.is_null() {
            if let Some(idx) = self.find_or_create(self.current_fn) {
                let entry = &mut self.entries[idx];
                entry.bytes += size;
                entry.count += 1;
                entry.bytes_by_class[class_idx] += size;
            }
        }

        self.maybe_interim_dump();
    }

    /// Print an interim snapshot each time cumulative allocation crosses the
    /// configured interval. Uses the Rust allocator for formatting — the
    /// profiled malloc is only reachable from Tungsten-generated code, so
    /// this cannot recurse.
    fn maybe_interim_dump(&mut self) {
        if !self.dump_interval_initialized {
            self.dump_interval_initialized = true;
            self.dump_interval = dump_interval_from_env();
            self.next_dump_bytes = self.dump_interval;
        }
        if self.dump_interval == 0 {
            return;
        }
        if self.total_bytes >= self.next_dump_bytes {
            report::print_interim(self);
            while self.next_dump_bytes <= self.total_bytes {
                self.next_dump_bytes += self.dump_interval;
            }
        }
    }
}

/// Bucket index for a `mu_alloc` size.
pub(crate) fn mu_bucket(size: u64) -> usize {
    for (i, bound) in MU_BUCKET_BOUNDS.iter().enumerate() {
        if size <= *bound {
            return i;
        }
    }
    NUM_MU_BUCKETS - 1
}

/// Read the interim-dump interval from the environment (MB, default 1024).
/// `0` disables interim dumps.
fn dump_interval_from_env() -> u64 {
    match crate::settings::read("ALLOC_PROFILE_INTERVAL_MB") {
        Some(v) => match v.trim().parse::<u64>() {
            Ok(mb) => mb * 1024 * 1024,
            Err(_) => DEFAULT_DUMP_INTERVAL_BYTES,
        },
        None => DEFAULT_DUMP_INTERVAL_BYTES,
    }
}

/// Set the current function name for allocation attribution.
///
/// Called by codegen-emitted hooks at the start of each Tungsten function
/// when `--alloc-profile` is enabled.
///
/// **Activation side effect:** the first call to this hook is what flips the
/// profiler to active — there is no separate activate API. Profiled builds
/// activate implicitly on their first function entry; tests that need the
/// profiler on (e.g. arena retention accounting, ADR 2.7.26a §3.4) call this
/// with a static name for the same effect. Query via
/// [`alloc_profile_is_active`]. There is deliberately no deactivate: the
/// profiler stays on for the process lifetime once a profiled function runs.
///
/// # Safety
///
/// `name` must be a valid, null-terminated C string with static lifetime
/// (codegen emits these as global string constants).
#[no_mangle]
pub unsafe extern "C" fn __albite_alloc_profile_set_fn(name: *const c_char) {
    unsafe {
        let profiler = &mut *PROFILER.0.get();
        profiler.current_fn = name;
        profiler.active = true;
    }
}

/// Set a filter so only the named function appears in the report.
///
/// # Safety
///
/// `name` must be a valid, null-terminated C string with static lifetime.
#[no_mangle]
pub unsafe extern "C" fn __albite_alloc_profile_set_filter(name: *const c_char) {
    unsafe {
        (*PROFILER.0.get()).filter_fn = name;
    }
}

/// Whether the allocation profiler has been activated by a profiled build.
///
/// Activation happens as a side effect of the first
/// [`__albite_alloc_profile_set_fn`] call (codegen emits one per function
/// entry in `--alloc-profile` builds); see that function's doc.
///
/// Lets `albite_core` gate its own (Rust-allocator-side) attribution work
/// — e.g. arena deep-size accounting (ADR 2.7.26a §3.4) — on profiler
/// activity, so non-profiled runs pay only this branch.
#[inline]
pub fn alloc_profile_is_active() -> bool {
    unsafe { (*PROFILER.0.get()).active }
}

/// Record an allocation made outside the profiled malloc path (e.g. libc
/// mallocs inside `albite_core` string FFI). No-op unless the profiler
/// is active, so non-profiled builds pay only a branch.
#[inline]
pub fn alloc_profile_record_external(size: u64, class: u32) {
    unsafe {
        let profiler = &mut *PROFILER.0.get();
        if profiler.active {
            profiler.record(size, class);
        }
    }
}

/// Emit a phase/module marker line with a snapshot of cumulative totals.
///
/// Called (via `tg_alloc_profile_marker` in `albite_core`) from the L2
/// compiler driver at phase boundaries so allocation slope correlates to
/// compiler structure (ADR 2.7.26a §3 P0). No-op when the profiler is
/// inactive (non-profiled builds).
///
/// # Safety
///
/// `name` must be a valid, null-terminated C string (any lifetime — it is
/// read immediately and not retained).
#[no_mangle]
pub unsafe extern "C" fn __albite_alloc_profile_marker(name: *const c_char) {
    unsafe {
        let profiler = &*PROFILER.0.get();
        if !profiler.active || name.is_null() {
            return;
        }
        let label = core::ffi::CStr::from_ptr(name)
            .to_str()
            .unwrap_or("<invalid utf8>");
        report::print_marker(profiler, label);
    }
}

/// Print the allocation profile report to stderr.
///
/// Called at the end of `__albite_inner_main` when `--alloc-profile` is enabled.
/// If a filter was set via `__albite_alloc_profile_set_filter`, only that
/// function is shown in the report.
#[no_mangle]
pub extern "C" fn __albite_alloc_profile_report() {
    unsafe {
        report::print_report(&*PROFILER.0.get());
    }
}

#[cfg(test)]
mod tests;
