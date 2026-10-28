//! Elaborator: Surface AST to Core Terms
//!
//! The elaborator bridges the gap between the human-friendly surface syntax
//! and the mathematically-precise Core calculus that `albite_core` can process.
//!
//! ## Responsibilities
//!
//! 1. **Name resolution**: Resolve identifiers to definitions
//! 2. **Type inference**: Infer omitted type annotations (bidirectional)
//! 3. **Desugaring**: Convert surface constructs to core primitives
//! 4. **Validation**: Reject Phase 1 unsupported features with helpful errors
//! 5. **Core term construction**: Build `albite_core::Term` values
//!
//! ## Architecture
//!
//! ```text
//! Surface AST
//!      │
//!      ▼
//! ┌─────────────────────────────────────┐
//! │         NAME RESOLUTION             │
//! │  • Build symbol table from items    │
//! │  • Resolve identifiers              │
//! └──────────────────┬──────────────────┘
//!                    │
//!                    ▼
//! ┌─────────────────────────────────────┐
//! │    ELABORATION + TYPE INFERENCE     │
//! │  • Bidirectional type checking      │
//! │  • Infer omitted annotations        │

// Submodules
//! │  • Desugar surface constructs       │
//! │  • Build Core terms                 │
//! └──────────────────┬──────────────────┘
//!                    │
//!                    ▼
//!            Core Terms + Diagnostics
//! ```

mod codegen_types;
mod collection;
pub(crate) mod collection_checks;
pub mod conversion;
pub mod env;
mod error;
mod error_recording;
mod exprs;
pub mod handles;
mod items;
pub mod kernel;
pub(crate) mod mutable;
mod normalization_view;
mod output;
pub mod positivity;
mod regions;
mod resolve_tyvars;
pub(crate) mod tactics;
pub mod termination;
// Phase-1d resolution-attempt instrumentation lives under `resolve_tyvars`
// (the pass it measures); re-exported here so the public path
// `crate::elaborate::resolution_metrics` stays stable (ADR 23.7.26a §6.1).
pub use resolve_tyvars::resolution_metrics;
// The census reads the gate's set (ADR 13.10.26t D4).
pub(crate) use items::carries_proof;
mod trace;
mod type_cache;
pub(crate) mod types;

// `pub(crate)` so diagnostic commands' tests can elaborate inline fixtures
// with `elab_ok` (ADR 6.10.26r).
#[cfg(test)]
pub(crate) mod tests;

pub use collection_checks::{ElaborationPhase, PhaseCheckResult};
pub use env::{
    canonicalize_module_path, Constructor, ConstructorInfo, ConstructorStubDetail, CoreNames, Env,
    ImportInfo, ImportTarget, LocalBinding, ModuleContents, ModulePath, PackageId, PackageTable,
    PathResolutionError, ResolvedValue, TypeDef, TypeDefKind, ValueDef, ValueImportTargets,
};
pub use error::{
    ElabError, ElabErrorKind, ExpectedContext, ExpectedReason, HandleErrorKind, Note,
    PackageErrorKind, RegionErrorKind, TraceFrame,
};
pub use normalization_view::ProjectNormalizer;
pub use output::elaborate_with_phase_checks;
pub use output::{
    first_poisoned_export, AdtOrigin, CollectedElaborator, CollectionResult, CoreDef,
    DefTerminationMeta, ElabOutput, ElabResult, FailedBodyPass, ModuleExports, TypeProvenance,
};

// Re-export top-level entry points
pub use output::{
    collect_definitions, collect_definitions_for_signature_collection,
    collect_definitions_with_exports, collect_definitions_with_modules, elaborate,
    elaborate_with_warnings, elaborate_with_warnings_full,
};

use albite_core::Context;

/// The elaborator state machine.
///
/// Elaboration mode — controls expect_type behaviour (ADR 4.5.26g §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ElabMode {
    /// albite check: validate expect_type args, skip comparison
    #[default]
    Check,
    /// albite test: validate + compare types
    Test,
    /// albite compile/run: reject expect_type
    Compile,
}

/// Holds all context needed during elaboration:
/// - Name resolution environment
/// - Reference to Core context (for validation)
/// - Accumulated errors
/// - Current de Bruijn depth
/// - Context stack for "expected because..." messages
#[allow(clippy::struct_excessive_bools)] // Reason: independent elaboration mode switches (ADR 1.10.26j D5)
pub struct Elaborator<'a> {
    /// Name resolution environment.
    ///
    /// Module-private: reachable from `elaborate::*` descendants but NOT from
    /// sibling crate modules (e.g. `driver`). Any helper that needs the live
    /// type env — such as `ProjectNormalizer` (ADR 21.7.26j) — must therefore
    /// live under `elaborate`, not `driver`; the driver re-exports it.
    env: Env,
    /// Reference to the Core context (for type validation)
    #[allow(dead_code)]
    core_ctx: &'a mut Context,
    /// Accumulated errors (we try to continue after errors)
    errors: Vec<ElabError>,
    /// How many collection-pass errors were compensated by a poison
    /// registration (ADR 14.8.26g D3) — a failed signature registered as
    /// `Type::Error`, or a failed type body's stub poisoned. The collection
    /// pass defers its short-circuit only when EVERY recorded error was
    /// compensated (D2's conditional form): an unpoisoned error means a name
    /// is simply missing, and Pass 2 would cascade rather than suppress.
    poison_compensated_error_count: usize,
    /// Defer the collection-pass short-circuit unconditionally
    /// (ADR 14.8.26g D2) — set only by the global Signature Collection
    /// entry point, which runs no Pass 2 and needs its partial exports back
    /// even when an unpoisoned error was recorded.
    defer_all_collection_errors: bool,
    /// Accumulated warnings (non-fatal diagnostics)
    warnings: Vec<ElabError>,
    /// Current de Bruijn depth for local variables
    depth: usize,
    /// Stack of type expectation contexts for error messages
    context_stack: Vec<ExpectedContext>,
    /// Counter for generating unique names
    name_counter: usize,
    /// Current module path (for visibility checking)
    current_module: ModulePath,
    /// Name of the definition currently being elaborated (for --trace-types)
    current_def_name: Option<String>,
    /// Index of the item the value-collection or body pass is visiting, which
    /// names its file and so its module (ADR 4.10.26j D1).
    current_item_index: Option<usize>,
    /// Target definition for type tracing (--trace-types=<name>)
    trace_target: Option<String>,
    /// Target type for encoding tracing (--trace-encoding=<name>)
    trace_encoding: Option<String>,
    /// Target type for normalization tracing (--trace-normalization=<name>)
    trace_normalization: Option<String>,
    /// Trace constructor registration calls (--trace-constructor-registration, ADR 7.5.26e)
    trace_ctor_registration: bool,
    /// Type provenance map built during ADT encoding (ADR 13.4.26c §3)
    type_provenance: TypeProvenance,
    /// Mutual recursion group membership (ADR 18.4.26i §5).
    /// Maps each type name → the full group (all members including itself).
    /// Only populated for types in SCCs of size > 1.
    mutual_recursion_groups: std::collections::HashMap<String, Vec<String>>,
    /// True during Type-Body Collection (type collection). ADT cross-references are deferred
    /// as TyVar("@Name") instead of eagerly encoded, so mutual recursion groups
    /// can be computed before encoding (ADR 18.4.26i §5).
    pub(crate) collecting_type_bodies: bool,
    /// When true, run phase invariant checks after each pipeline phase (ADR 20.4.26e).
    pub(crate) check_phase_invariants: bool,
    /// Accumulated phase invariant check results (ADR 20.4.26e).
    pub(crate) phase_invariant_results: Vec<collection_checks::PhaseCheckResult>,
    /// Debug-mode consistency map: records the first recursiveness decision for each
    /// ADT name and panics if a subsequent call returns a different result.
    /// Zero overhead in release builds. (ADR 21.4.26c)
    #[cfg(debug_assertions)]
    pub(crate) recursiveness_decisions: std::cell::RefCell<std::collections::HashMap<String, bool>>,
    /// Elaboration mode (ADR 4.5.26g)
    pub(crate) elab_mode: ElabMode,
    /// When true, value collection (functions, extern fns, theorems, axioms)
    /// overwrites existing entries rather than reporting duplicates. Set by
    /// Signature Collection's `collect_definitions_with_exports()` to allow re-collection
    /// of values that were pre-registered by Stub Registration stubs (ADR 5.5.26c).
    pub(crate) allow_value_overwrite: bool,
    /// The declared return type of the current function (for `return` expressions).
    /// Set when entering a function body, reset on exit.
    pub(crate) current_return_type: Option<albite_core::Type>,
    /// Termination annotations and proof-relevance, per definition name
    /// (ADR 29.6.26e). Recorded for every item the elaborator visits, whether
    /// or not it produces a `CoreDef`.
    pub(crate) termination_meta: std::collections::HashMap<String, DefTerminationMeta>,
    /// Whether a conversion failure δ could change is deferred as an
    /// obligation rather than raised (ADR 28.9.26d D3). Off by default: only
    /// the per-module Body Elaboration walk, the one path that reaches
    /// admission, turns it on.
    pub(crate) defer_conversion: bool,
    /// The obligations deferred so far (ADR 28.9.26d §2.1).
    pub(crate) conversion_obligations: Vec<conversion::ConversionObligation>,
    /// How many deferrals were recorded, checked at discharge (D15).
    pub(crate) conversion_deferred: usize,
    /// Whether kernel obligations wait for the admission boundary rather than
    /// being discharged at the end of the body pass (ADR 2.10.26c D5). Off by
    /// default; only the per-module walk turns it on.
    pub(crate) defer_kernel: bool,
    /// Tactic proofs recorded for the kernel, not yet discharged (D5).
    pub(crate) kernel_obligations: Vec<kernel::KernelObligation>,
    /// How many obligations the body pass discharged locally (D5).
    pub(crate) kernel_discharged: usize,
    /// Whether a `theorem` or `lemma` body is being elaborated, where the
    /// kernel converts, so `absurd` may convert too (ADR 7.10.26m D4).
    pub(crate) in_proof_body: bool,
    /// The joins of each lowered block being elaborated, with its expected
    /// type, so a join is checked against known types (ADR 13.10.26r D3).
    pub(crate) join_frames: Vec<(mutable::JoinMap, Option<albite_core::Type>)>,
}

impl<'a> Elaborator<'a> {
    /// Create a new elaborator with an empty environment.
    pub fn new(core_ctx: &'a mut Context) -> Self {
        Self {
            env: Env::new(),
            core_ctx,
            errors: Vec::new(),
            poison_compensated_error_count: 0,
            defer_all_collection_errors: false,
            warnings: Vec::new(),
            depth: 0,
            context_stack: Vec::new(),
            name_counter: 0,
            current_module: ModulePath::root(),
            current_def_name: None,
            current_item_index: None,
            trace_target: None,
            // Seeded from ALBITE_TRACE_ENCODING so encoding tracing reaches
            // every entry path, not just the compile/check CLI (ADR 22.7.26d).
            // Unset var → None → identical to prior behaviour.
            trace_encoding: trace::trace_encoding_from_env(),
            trace_normalization: None,
            trace_ctor_registration: false,
            type_provenance: TypeProvenance::default(),
            mutual_recursion_groups: std::collections::HashMap::new(),
            collecting_type_bodies: false,
            check_phase_invariants: false,
            phase_invariant_results: Vec::new(),
            #[cfg(debug_assertions)]
            recursiveness_decisions: std::cell::RefCell::new(std::collections::HashMap::new()),
            elab_mode: ElabMode::Check,
            allow_value_overwrite: false,
            current_return_type: None,
            termination_meta: std::collections::HashMap::new(),
            defer_conversion: false,
            conversion_obligations: Vec::new(),
            conversion_deferred: 0,
            defer_kernel: false,
            kernel_obligations: Vec::new(),
            kernel_discharged: 0,
            in_proof_body: false,
            join_frames: Vec::new(),
        }
    }
    /// Execute `f` with `current_return_type` set to `ty`, restoring the previous
    /// value afterward (even on error). Use this instead of manual save/restore.
    pub(crate) fn with_return_context<T>(
        &mut self,
        ty: Option<albite_core::Type>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let prev = std::mem::replace(&mut self.current_return_type, ty);
        let result = f(self);
        self.current_return_type = prev;
        result
    }

    /// Generate a fresh unique variable name with the given prefix.
    pub fn fresh_var(&mut self, prefix: &str) -> String {
        let name = format!("__{}{}", prefix, self.name_counter);
        self.name_counter += 1;
        name
    }

    /// Get the current module path.
    pub fn get_current_module(&self) -> &ModulePath {
        &self.current_module
    }

    /// Set the current module path (for entering nested modules).
    pub fn set_current_module(&mut self, module: ModulePath) {
        self.current_module = module;
    }

    /// Enable phase invariant checking (ADR 20.4.26e).
    pub fn set_check_phase_invariants(&mut self, enabled: bool) {
        self.check_phase_invariants = enabled;
    }

    /// Take the accumulated phase invariant results, leaving the vec empty.
    pub fn take_phase_invariant_results(&mut self) -> Vec<collection_checks::PhaseCheckResult> {
        std::mem::take(&mut self.phase_invariant_results)
    }
}
