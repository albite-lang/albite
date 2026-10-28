//! Per-module elaboration loop (ADR 5.5.26b, 5.5.26c).
//!
//! Elaborates each module in the `ParsedModule` tree independently in
//! post-order (children before parents). Cross-module definitions from
//! completed modules are injected into subsequent modules' environments.
//!
//! Two-phase architecture (ADR 5.5.26c):
//!   Stub Registration — walk ALL modules, register type + constructor stubs globally
//!   Signature Collection — collect ALL function signatures globally (combined AST)
//!   Body Elaboration — elaborate each module's value bodies in post-order

mod accumulator;
pub(super) mod cache;
pub(super) mod fresh_encodings;
pub(crate) mod inspect;
mod phases;
mod profile;
pub(crate) mod stubs;
#[cfg(test)]
mod tests;
mod walk;

use std::path::Path;
use std::time::Instant;

use crate::cache::elab_cache;
use crate::cache::elab_cache::writer::{self, BackgroundWriter};
use crate::elaborate::kernel::KernelMode;
use crate::elaborate::{termination, ElabError, ElabOutput};

use accumulator::ModuleTreeAccumulator;
use walk::TreeWalkState;

use super::modules::{self, ParsedModule};
use super::output::TraceOptions;
use super::pipeline::BuildCtx;

/// Output of per-module elaboration, extending `ElabOutput` with module partitioning data.
pub(super) struct ModuleTreeOutput {
    pub(super) elab: ElabOutput,
    /// Per-module definition groups for codegen unit partitioning (ADR 7.5.26h).
    /// Each entry is (module_path, source_file, defs).
    pub(super) module_defs: Vec<(
        Vec<String>,
        std::path::PathBuf,
        Vec<crate::elaborate::CoreDef>,
    )>,
    /// Per-module value import targets, keyed by canonical module path
    /// (ADR 12.7.26a §2.1).
    pub(super) module_import_targets:
        std::collections::BTreeMap<Vec<String>, crate::elaborate::ValueImportTargets>,
    /// Def count from cache hits (bodies not re-elaborated).
    pub(super) cached_def_count: usize,
    /// Accumulated whole-project exports after Body Elaboration (ADR 21.7.26j).
    /// Every module's real type/value/constructor definitions, *moved* out of
    /// the accumulator (not cloned — they were about to be dropped), so the
    /// live-elaborator normalization check can seed a fresh whole-project
    /// `Elaborator`. The compile path ignores this field.
    pub(super) exports: crate::elaborate::ModuleExports,
    /// Whether this output came from the cold retry without signature-cache
    /// hits (ADR 28.9.26d D12).
    pub(super) conversion_retried: bool,
    /// Term-mode theorems the kernel register exempted, by class (ADR 2.10.26d D11).
    pub(super) kernel_exempt: crate::elaborate::kernel::ExemptTally,
}

/// A failed pass of [`elaborate_module_tree_once`], with what the cold-retry
/// decision needs (ADR 28.9.26d D12).
struct TreeFailure {
    errors: Vec<ElabError>,
    failed_obligations: usize,
    signature_hit_modules: usize,
}

impl From<Vec<ElabError>> for TreeFailure {
    /// A failure before discharge: nothing δ could have changed.
    fn from(errors: Vec<ElabError>) -> Self {
        TreeFailure {
            errors,
            failed_obligations: 0,
            signature_hit_modules: 0,
        }
    }
}

/// Whether a failed warm pass is re-run without signature-cache hits
/// (ADR 28.9.26d D12): only when a conversion or kernel obligation failed
/// (ADR 2.10.26c D5) **and** some module's bodies were absent from the δ
/// environment, or its definitions from the kernel's, because it was a hit.
/// A missing body can only make an obligation fail, never pass, so the cold
/// verdict is the true one.
pub(super) fn needs_cold_retry(failed_obligations: usize, signature_hit_modules: usize) -> bool {
    failed_obligations > 0 && signature_hit_modules > 0
}

/// The retry's verbose line, so a reader can tell the two passes apart.
pub(super) fn cold_retry_notice(failed_obligations: usize, signature_hit_modules: usize) -> String {
    format!(
        "conversion: retrying without signature-cache hits ({failed_obligations} failed obligation(s), {signature_hit_modules} hit module(s))"
    )
}

/// Elaborate a module tree per-module in post-order, re-running it once cold
/// when a δ proof failed only for want of a cached module's bodies
/// (ADR 28.9.26d D12). The first pass's output is dropped whole, so nothing is
/// reported twice.
pub(super) fn elaborate_module_tree(
    module_tree: &ParsedModule,
    _source_path: &Path,
    verbose: bool,
    build: &BuildCtx<'_>,
    trace: &TraceOptions,
) -> Result<ModuleTreeOutput, Vec<ElabError>> {
    // Fix the Core-name table on the shared registry before any per-module
    // clone is taken, so each clone carries it rather than recomputing it
    // (ADR 4.10.26j D1).
    build.module_info.core_names();
    let failure = match elaborate_module_tree_once(module_tree, verbose, build, trace, true) {
        Ok(output) => return Ok(output),
        Err(failure) => failure,
    };
    if !needs_cold_retry(failure.failed_obligations, failure.signature_hit_modules) {
        return Err(failure.errors);
    }
    if verbose {
        eprintln!(
            "{}",
            cold_retry_notice(failure.failed_obligations, failure.signature_hit_modules)
        );
    }
    // The retry starts from the state a cold run starts from: the first pass's
    // comparator synthesis has read and filled the request registry, which
    // `prepare_project` clears once per invocation.
    crate::comparator::requests::clear();
    let mut output = elaborate_module_tree_once(module_tree, verbose, build, trace, false)
        .map_err(|failure| failure.errors)?;
    output.conversion_retried = true;
    Ok(output)
}

/// Elaborate a module tree per-module in post-order (ADR 5.5.26b §3, 5.5.26c).
///
/// Two-phase approach:
///   Stub Registration: Walk all modules and register type + constructor stubs globally,
///            so cross-branch imports resolve before any body elaboration.
///   Body Elaboration: Elaborate each module's value bodies in post-order, injecting
///            full defs from completed siblings.
///
/// `signature_hits: false` skips the bodyless signature-cache tier.
fn elaborate_module_tree_once(
    module_tree: &ParsedModule,
    verbose: bool,
    build: &BuildCtx<'_>,
    trace: &TraceOptions,
    signature_hits: bool,
) -> Result<ModuleTreeOutput, TreeFailure> {
    let mut acc = ModuleTreeAccumulator::new();
    // Read once, here on the caller thread, before the pooled walk (ADR 2.10.26d D2).
    let kernel_mode = KernelMode::from_window(termination::reporting_window_open());
    let profiling = profile::is_enabled();
    let mut elab_profile = profile::ElabProfile::new();

    // Stub Registration: collect type + constructor stubs from ALL modules (ADR 5.5.26c §2.2)
    let stub_registration_start = Instant::now();
    stubs::collect_all_type_and_constructor_stubs(module_tree, &mut acc.exports);
    let stub_registration_elapsed = stub_registration_start.elapsed();
    elab_profile.stub_registration = stub_registration_elapsed;
    phases::log_stub_registration(verbose, trace, &acc.exports);

    // Signature Collection: collect function signatures globally.
    // Build a combined AST of ALL items from ALL modules and run the
    // collection pass using Stub Registration type stubs. This gives every module
    // access to all function types for cross-branch value imports.
    let signature_collection_start = Instant::now();
    phases::run_signature_collection(module_tree, build, &mut acc, verbose);
    let signature_collection_elapsed = signature_collection_start.elapsed();
    phases::log_signature_collection(verbose, &acc.exports);

    // Compute exports hash once for Body Elaboration cache keys (ADR 10.5.26l §2.1).
    // This captures the full Signature Collection environment state. Any upstream change
    // produces a different hash, conservatively invalidating all module caches.
    let exports_hash = elab_cache::hash_exports(&acc.exports);

    elab_profile.signature_collection = signature_collection_elapsed;

    // Body Elaboration: elaborate each module's bodies in post-order
    let full_output_cache =
        albite_runtime::settings::read("ELAB_CACHE_FULL").is_some_and(|v| v == "1");
    let body_elaboration_start = Instant::now();
    let flags = ElabCtxFlags {
        verbose,
        profiling,
        full_output_cache,
        signature_hits,
    };
    if let Err(mut errors) = run_body_elaboration(
        module_tree,
        build,
        trace,
        flags,
        exports_hash,
        &mut acc,
        &mut elab_profile,
    ) {
        // A failed walk still discharges what did elaborate, so a false δ
        // proof is reported beside the other errors (ADR 4.10.26e D6).
        let failures = acc.discharge_beside_errors(verbose);
        let failed_obligations = failures.len();
        errors.extend(failures);
        return Err(TreeFailure {
            errors,
            failed_obligations,
            signature_hit_modules: acc.signature_hit_modules,
        });
    }
    let body_elaboration_elapsed = body_elaboration_start.elapsed();
    elab_profile.body_elaboration_total = body_elaboration_elapsed;

    if verbose {
        let total =
            stub_registration_elapsed + signature_collection_elapsed + body_elaboration_elapsed;
        eprintln!(
            "  Elaboration phase timing: stub-reg={:.0?}, sig-collect={:.0?}, body-elab={:.0?}, total={:.0?}",
            stub_registration_elapsed, signature_collection_elapsed, body_elaboration_elapsed, total,
        );
    }

    if profiling {
        elab_profile.emit();
    }

    // Comparator synthesis runs *before* the gate (ADR 11.8.26b §2.3) so its
    // output is inside the trusted boundary rather than appended past it.
    if let Some(notice) = synthesis_notice(verbose, acc.synthesize_comparators()) {
        eprintln!("{notice}");
    }

    // Termination admission (ADR 29.6.26e §2.5). This is the trusted boundary:
    // the first and only point where every definition — freshly elaborated,
    // imported, or reconstructed from cache — is in one set. Running it here
    // rather than in `elaborate_project` means `check`, `run`, `test` and
    // `compile` all traverse the same state machine.
    // Deferred conversions are discharged inside it, against its verdicts
    // (ADR 28.9.26d §2.2). Affine handles are checked beside it (ADR 7.10.26t).
    acc.admit_with_handles(verbose, kernel_mode, module_tree)
        .map_err(|errors| TreeFailure {
            errors,
            // A kernel refusal can be a citation a signature hit left unbound,
            // so it earns the same cold retry (ADR 2.10.26c D5).
            failed_obligations: acc.conversion_failure_count + acc.kernel_failure_count,
            signature_hit_modules: acc.signature_hit_modules,
        })?;
    Ok(acc.into_tree_output())
}

/// What to print about comparator synthesis, or `None` for silence.
///
/// A pure function rather than an `if` at the call site so both conditions are
/// assertable: an `eprintln!` behind `verbose && count > 0` is invisible to the
/// test suite, and every mutation of that guard survives (measured — ADR
/// 11.8.26b's close-out sweep flagged `&&`, `>`, `>=` and `==` here).
pub(super) fn synthesis_notice(verbose: bool, count: usize) -> Option<String> {
    (verbose && count > 0).then(|| format!("Synthesized {count} comparator(s)"))
}

/// Run Body Elaboration: elaborate module bodies in post-order with optional background caching.
fn run_body_elaboration(
    module_tree: &ParsedModule,
    build: &BuildCtx<'_>,
    trace: &TraceOptions,
    flags: ElabCtxFlags,
    exports_hash: [u8; 32],
    acc: &mut ModuleTreeAccumulator,
    elab_profile: &mut profile::ElabProfile,
) -> Result<(), Vec<ElabError>> {
    // Spawn background cache writer for full-output entries (ADR 10.5.26o)
    let bg_writer = if flags.full_output_cache {
        Some(BackgroundWriter::spawn(writer::default_channel_capacity()))
    } else {
        None
    };

    let elab_ctx = build_elab_ctx(flags, build, trace, exports_hash, bg_writer.as_ref());
    let root_path: Vec<String> = Vec::new();
    {
        let mut state = TreeWalkState { acc, elab_profile };
        walk::elaborate_module_tree_rec(module_tree, &root_path, &elab_ctx, &mut state);
    }

    // The walk accumulates instead of short-circuiting (ADR 14.8.26g D1), so
    // the run's verdict is read here, after every module has been examined.
    // Pre-walk (Signature Collection, D4) errors report first, then module
    // groups re-ordered into the canonical (serial) walk order, so the same
    // fault produces a byte-identical list at every `thread_count`.
    if acc.has_accumulated_errors() {
        let mut canonical_order = Vec::new();
        walk::canonical_walk_order(module_tree, &root_path, &mut canonical_order);
        let mut errors = acc.take_accumulated_errors_ordered(&canonical_order);
        phases::finish_failed_body_elaboration(acc, module_tree, &mut errors);
        // The staged cache writes are dropped, not committed (ADR 14.8.26g
        // D5): a failing run leaves the cache exactly as it found it.
        return Err(errors);
    }

    // The whole run succeeded — commit the staged cache writes (ADR 14.8.26g
    // D5). Full-output bytes go to the background writer, which the join
    // below flushes.
    cache::commit_pending_cache_writes(std::mem::take(&mut acc.pending_cache_writes), &elab_ctx);

    // Join background writer — flush all pending entries (ADR 10.5.26o)
    if let Some(writer) = bg_writer {
        let write_errors = writer.join();
        if !write_errors.is_empty() && flags.verbose {
            eprintln!(
                "[elab-cache-full] {} background write error(s) during Body Elaboration",
                write_errors.len()
            );
            for err in &write_errors {
                eprintln!("  {}: {}", err.path.display(), err.error);
            }
        }
    }

    Ok(())
}

/// Context for recursive module elaboration (bundles environment params).
pub(super) struct ElabTreeCtx<'a> {
    pub(super) flags: ElabCtxFlags,
    pub(super) build: &'a BuildCtx<'a>,
    trace: &'a TraceOptions,
    /// Hash of Signature Collection exports for cache key computation (ADR 10.5.26l).
    pub(super) exports_hash: [u8; 32],
    /// Background cache writer for full-output entries (ADR 10.5.26o).
    /// `None` when full-output caching is disabled.
    pub(super) bg_writer: Option<&'a BackgroundWriter>,
    /// Configured thread count for parallel Body Elaboration (ADR 11.5.26b §P5).
    /// Read once from `ALBITE_ELAB_THREADS`; 1 = serial (default).
    thread_count: usize,
    /// How many failing modules the walk tolerates before bailing out
    /// (ADR 14.8.26g D6's wall-clock arm): D7's display budget
    /// (`--max-errors`, 0 = unlimited) applied to modules rather than
    /// diagnostics. P2 measured a failing run at ~10× the short-circuiting
    /// run's wall clock — the cost of examining the whole tree — which is the
    /// accepted price *below* this budget: past it, nothing more is
    /// displayable anyway, and the unreached-module note reports what the
    /// bail-out skipped. Read once, on the caller thread, because the
    /// underlying setting is thread-local and rayon workers would otherwise
    /// read the default.
    pub(super) module_failure_budget: usize,
    /// Shared rayon thread pool for parallel Body Elaboration (ADR 11.5.26b §P5).
    /// `None` when `thread_count == 1` (serial mode).
    parallel_pool: Option<rayon::ThreadPool>,
}

/// Boolean flags controlling per-module elaboration behavior.
#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // Reason: independent per-module switches (ADR 1.10.26j D5)
pub(super) struct ElabCtxFlags {
    pub(super) verbose: bool,
    /// Whether per-module profiling is enabled (ADR 11.5.26b §P0).
    pub(super) profiling: bool,
    /// Whether full-output caching is enabled (ADR 12.5.26a).
    pub(super) full_output_cache: bool,
    /// Whether the bodyless signature-cache tier may be consulted; off for the
    /// cold retry (ADR 28.9.26d D12).
    pub(super) signature_hits: bool,
}

fn build_elab_ctx<'a>(
    flags: ElabCtxFlags,
    build: &'a BuildCtx<'a>,
    trace: &'a TraceOptions,
    exports_hash: [u8; 32],
    bg_writer: Option<&'a BackgroundWriter>,
) -> ElabTreeCtx<'a> {
    let thread_count = cache::equivalence::elab_thread_count();
    let parallel_pool = if thread_count > 1 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(thread_count)
            .build()
            .ok()
    } else {
        None
    };
    ElabTreeCtx {
        flags,
        build,
        trace,
        exports_hash,
        bg_writer,
        thread_count,
        module_failure_budget: super::diagnostics::get_max_errors(),
        parallel_pool,
    }
}
