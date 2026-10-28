//! The project output type and codegen units; value and type formatting is
//! in `format.rs`.

use std::collections::HashMap;

use crate::elaborate::Constructor;
use albite_core::Type;

use super::modules::SourceMap;

mod format;
pub use format::{format_type, format_type_for_display, format_value};

/// Record type definitions: name -> fields.
/// Used by codegen to expand `TyVar("RecordName")` to structural product types.
pub type RecordTypes = HashMap<String, Vec<(String, Type)>>;

/// ADT type definitions: name -> (params, constructors).
/// Used by codegen to expand `Type::App("Name", args)` to sum/mu types.
pub type AdtTypes = HashMap<String, (Vec<String>, Vec<Constructor>)>;

/// Type alias definitions: name -> (params, target type).
/// Used by `info` commands for display.
pub type TypeAliases = HashMap<String, (Vec<String>, Type)>;

/// Per-module value import targets keyed by canonical module path
/// (ADR 12.7.26a §2.1). Codegen's colliding-import resolution input.
pub type ValueImportTargetsByModule =
    std::collections::BTreeMap<Vec<String>, crate::elaborate::ValueImportTargets>;

/// Optional trace filters for elaboration diagnostics.
#[derive(Clone, Default)]
pub struct TraceOptions {
    pub trace_types: Option<String>,
    pub trace_encoding: Option<String>,
    pub trace_normalization: Option<String>,
    pub trace_ctor_registration: bool,
    pub elab_mode: crate::elaborate::ElabMode,
}

/// Pipeline execution options: flags that control check/run behaviour.
pub struct PipelineOpts {
    pub mode: super::Mode,
    pub verbose: bool,
    pub dump_types: bool,
}

/// A codegen unit representing a single top-level function (ADR 9.5.26b).
///
/// Each codegen unit is emitted to a separate `.ll` file. The `source_file` path
/// is used to derive the mirror output path under `target/ll/`. Each unit contains
/// exactly one `CoreDef`; inner lambdas are co-located by the `CodeGen` layer.
#[derive(Debug, Clone)]
pub struct ModuleCodegenUnit {
    /// Logical module path segments (e.g., `["compiler", "lexer"]`)
    pub module_path: Vec<String>,
    /// Original `.tg` source file path (for mirror output path derivation)
    pub source_file: std::path::PathBuf,
    /// The single definition owned by this unit (exactly one element)
    pub defs: Vec<crate::elaborate::CoreDef>,
}

/// Complete output of project elaboration.
///
/// Aggregates elaboration results at the project level (driver output).
/// Unlike `ElabOutput` (per-module elaboration), this includes the source map
/// and omits warnings (which are rendered inline during elaboration).
///
/// `Default` yields the all-empty output, for stub and test-fixture sites; the
/// genuine builder spells out every field, so a new one is a compile error there.
#[derive(Default)]
pub struct ProjectOutput {
    /// The elaborated definitions (flat, for backward compatibility)
    pub defs: Vec<crate::elaborate::CoreDef>,
    /// Per-module codegen units (ADR 6.5.26c §2.1)
    pub codegen_units: Vec<ModuleCodegenUnit>,
    /// Record type definitions: name -> fields
    pub record_types: RecordTypes,
    /// ADT type definitions: name -> (params, constructors)
    pub adt_types: AdtTypes,
    /// Type alias definitions: name -> (params, target type)
    pub type_aliases: TypeAliases,
    /// Type provenance: μ-binder → ADT origin
    pub type_provenance: crate::elaborate::TypeProvenance,
    /// Source map for multi-file error reporting
    pub source_map: SourceMap,
    /// Cached type encodings from Encoding Finalization (ADR 20.4.26c)
    pub encoded_types: std::collections::HashMap<String, Type>,
    /// Mutual recursion groups from Recursion Grouping SCC (ADR 20.4.26c)
    pub mutual_recursion_groups: std::collections::HashMap<String, Vec<String>>,
    /// Parent type visibilities (ADR 14.5.26c).
    /// Maps type name → declared visibility. Used by `info type members visibility`.
    pub type_visibilities: std::collections::HashMap<String, crate::ast::Visibility>,
    /// Per-field visibility overrides for record types (ADR 14.5.26c).
    /// Maps record name → per-field visibility (None = inherit parent).
    pub record_field_visibilities:
        std::collections::HashMap<String, Vec<Option<crate::ast::Visibility>>>,
    /// Per-module value import targets, keyed by canonical module path
    /// (ADR 12.7.26a §2.1): each module's `original name → canonical defining
    /// module` table, consumed by codegen's colliding-import resolution.
    pub value_import_targets: ValueImportTargetsByModule,
    /// Termination annotations and proof-relevance (ADR 29.6.26e), so
    /// `doctor check type termination` re-runs the gate's analysis on the
    /// gate's own input rather than on a reconstruction of it.
    pub termination_meta: std::collections::HashMap<String, crate::elaborate::DefTerminationMeta>,
}

impl ProjectOutput {
    /// Build the set of known concrete type names (ADT ∪ record keys).
    ///
    /// Used by mono discovery and codegen to distinguish cross-module type
    /// references (`TyVar("Binding")`) from abstract type parameters (`TyVar("T")`).
    /// Type aliases are excluded: they are expanded during elaboration and do
    /// not appear as bare `TyVar` in Core IR bodies. See ADR 13.5.26a §2.2.
    pub fn concrete_type_names(&self) -> std::collections::HashSet<String> {
        self.adt_types
            .keys()
            .chain(self.record_types.keys())
            .cloned()
            .collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Codegen unit construction
// ═══════════════════════════════════════════════════════════════════════

/// Build one `ModuleCodegenUnit` per `CoreDef` from per-module definition groups.
///
/// Each codegen unit wraps a single definition — codegen emits one `.ll` file
/// per unit. The module path and source file are propagated from the per-module
/// elaboration output.
pub(crate) fn build_codegen_units(
    module_defs: Vec<(
        Vec<String>,
        std::path::PathBuf,
        Vec<crate::elaborate::CoreDef>,
    )>,
) -> Vec<ModuleCodegenUnit> {
    module_defs
        .into_iter()
        .flat_map(|(module_path, source_file, defs)| {
            defs.into_iter().map(move |def| ModuleCodegenUnit {
                module_path: module_path.clone(),
                source_file: source_file.clone(),
                defs: vec![def],
            })
        })
        .collect()
}
