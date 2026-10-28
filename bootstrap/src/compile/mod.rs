//! `albite compile` — Compile to native executable via LLVM codegen.
//!
//! This module contains the compile command and its helpers:
//! - `cmd_compile()`: orchestrates elaboration → codegen → linking
//! - `TyVar` validation and post-elaboration cleanup
//! - Diagnostic helpers (dump-ir, dump-encoding)
//! - Type parameter substitution extraction

pub(crate) mod check_extern_map_ambiguity;
pub(crate) mod check_mono_coverage;
mod diagnostics;
mod extern_naming;
mod linking;
pub(crate) mod mangling;
pub(crate) mod mono;
pub(crate) mod native_core;
pub(crate) mod per_module;
pub(crate) mod tco;
pub(crate) mod unit_cost;
mod validation;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;

use albite_bootstrap::driver::diagnostics::compile_failure::CompileFailure;
use albite_bootstrap::driver::diagnostics::ErrorFormat;
use albite_bootstrap::driver::{self, AdtTypes, CoreDef};

/// Compute the LLVM-level name for a definition.
///
/// Renames "main" to "`albite_main`" to avoid conflicts with the C main wrapper.
fn def_llvm_name(def_name: &str) -> String {
    if def_name == "main" {
        "albite_main".to_string()
    } else {
        def_name.to_string()
    }
}

/// Convert bootstrap ADT constructors to codegen-compatible constructors.
pub(crate) fn convert_adt_types_for_codegen(
    adt_types: AdtTypes,
) -> std::collections::HashMap<String, (Vec<String>, Vec<albite_codegen::CodegenConstructor>)> {
    adt_types
        .into_iter()
        .map(|(name, (params, constructors))| {
            let codegen_ctors: Vec<albite_codegen::CodegenConstructor> = constructors
                .into_iter()
                .map(|ctor| albite_codegen::CodegenConstructor {
                    name: ctor.name,
                    fields: ctor.fields,
                    index: ctor.index,
                })
                .collect();
            (name, (params, codegen_ctors))
        })
        .collect()
}

/// Pass 1: Declare all function prototypes and register definitions with codegen.
///
/// Returns the extern name mapping (`original_name` → LLVM name) on success.
pub(super) fn declare_and_register_defs(
    codegen: &mut albite_codegen::CodeGen<'_>,
    defs: &[CoreDef],
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut extern_name_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // Declare all functions (add prototypes to enable forward references)
    for def in defs {
        let original_name = def_llvm_name(&def.name);

        let llvm_name = if let Some((orig, wrap)) = extern_wrap_name(&def.name, &def.term) {
            extern_name_map.insert(orig, wrap.clone());
            wrap
        } else {
            original_name.clone()
        };

        if matches!(&def.ty, albite_core::types::Type::Forall(_, _)) {
            codegen.register_def_type(&llvm_name, &def.ty);
        } else if let Err(e) = codegen.declare_def(&llvm_name, &def.ty) {
            return Err(format!("declaration failed for '{}': {}", def.name, e));
        }
    }

    // Register extern name mappings with codegen for Global lookups
    codegen.register_extern_name_map(extern_name_map.clone());

    // Register term definitions for potential monomorphization
    for def in defs {
        let original_name = def_llvm_name(&def.name);
        let llvm_name = extern_name_map
            .get(&original_name)
            .cloned()
            .unwrap_or(original_name);
        codegen.register_term_def(&llvm_name, def.term.term.clone());
    }

    Ok(extern_name_map)
}

/// Pass 2: Compile all definitions into LLVM IR.
pub(super) fn compile_all_defs(
    codegen: &mut albite_codegen::CodeGen<'_>,
    defs: &[CoreDef],
    extern_name_map: &std::collections::HashMap<String, String>,
    verbose: bool,
) -> Result<(), String> {
    let total_defs = defs.len();
    for (i, def) in defs.iter().enumerate() {
        let original_name = def_llvm_name(&def.name);
        let llvm_name = extern_name_map
            .get(&original_name)
            .cloned()
            .unwrap_or(original_name);

        // Skip polymorphic definitions — compiled via monomorphization
        if matches!(&def.ty, albite_core::types::Type::Forall(_, _)) {
            continue;
        }

        if verbose {
            eprintln!("Compiling {} [{}/{}]...", def.name, i + 1, total_defs);
        }

        if let Err(e) = codegen.compile_def_with_span(
            &llvm_name,
            &def.term.term,
            &def.ty,
            def.term.span.map(|s| s.start),
        ) {
            return Err(format!("codegen failed for '{}': {}", def.name, e));
        }
    }
    Ok(())
}

/// Warn about definitions that still contain `sorry`, with each one's counts
/// from the classifier `doctor check sorry-sites` uses (ADR 18.9.26g).
pub(super) fn warn_sorry_defs(defs: &[CoreDef]) {
    let sorry_defs: Vec<_> = defs.iter().filter(|d| d.term.contains_sorry()).collect();
    if !sorry_defs.is_empty() {
        eprintln!(
            "warning: {} definition(s) contain `sorry` (may be dead code from pattern matching):",
            sorry_defs.len()
        );
        for def in &sorry_defs {
            let counts = albite_core::terms::analysis::SorryCounts::of([&*def.term]);
            eprintln!(
                "  - {} ({} authored, {} synthesised, {} unclassified)",
                def.name, counts.authored, counts.synthesised, counts.unclassified
            );
        }
    }
}

/// Diagnostic and tracing flags for the compile command.
///
/// Groups optional diagnostic parameters that control IR dumps,
/// type tracing, and encoding inspection.
#[derive(Default)]
pub(crate) struct DiagnosticFlags {
    pub(crate) dump_ir: Option<String>,
    pub(crate) trace_types: Option<String>,
    pub(crate) dump_encoding: Option<String>,
    pub(crate) codegen_backtrace: bool,
    pub(crate) check_tyvar_escape: bool,
    /// Runtime and codegen tracing flags (musttail, escape, mono, ADT ops, etc.)
    pub(crate) tracing: TraceFlags,
    /// Allocation profiling filter (None = disabled, Some("") = all, Some(pat) = filtered).
    pub(crate) alloc_profile: Option<String>,
    /// Compile only these codegen units (`--only-unit`, ADR 3.7.26b).
    /// Empty = compile everything. Non-empty requires --emit-llvm, skips the
    /// __mono depot, and disables serial-unit scheduling.
    pub(crate) only_units: Vec<String>,
    /// Dump synthesized comparator Core terms (`--dump-synthesized`, ADR
    /// 12.7.26c P6). `None` = off; `Some(None)` = all; `Some(Some(sym))` =
    /// only defs whose symbol contains `sym`.
    #[allow(clippy::option_option)] // Reason: off / all / one-symbol tri-state
    pub(crate) dump_synthesized: Option<Option<String>>,
}

/// Runtime and codegen tracing toggles.
///
/// Grouped separately from `DiagnosticFlags` because these are all
/// simple on/off toggles that control stderr trace output during codegen.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // Reason: independent trace toggles (ADR 1.10.26j D5)
#[allow(clippy::struct_field_names)] // Reason: each field mirrors its `--trace-*` CLI flag name
pub(crate) struct TraceFlags {
    pub(crate) trace_adt_ops: Option<String>,
    pub(crate) trace_encoding: Option<String>,
    pub(crate) trace_normalization: Option<String>,
    pub(crate) trace_constructor_registration: bool,
    pub(crate) trace_musttail: bool,
    pub(crate) trace_escape: bool,
    pub(crate) trace_mono: bool,
}

/// Configuration flags for the compile command.
///
/// Bundles the boolean flags and diagnostic parameters that are
/// threaded through `cmd_compile` → `run_codegen` → `emit_output`.
///
/// `Default` yields an all-off config (note: `codegen_jobs` defaults to 0 —
/// diagnostic callers that drive codegen must set it to ≥1).
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // Reason: independent CLI compile flags (ADR 1.10.26j D5)
pub(crate) struct CompileFlags {
    pub(crate) emit_llvm: bool,
    pub(crate) verbose: bool,
    pub(crate) max_errors: usize,
    pub(crate) dump_types: bool,
    pub(crate) debug_info: bool,
    pub(crate) sanitize: bool,
    pub(crate) named_lambdas: bool,
    pub(crate) no_codegen: bool,
    pub(crate) diagnostics: DiagnosticFlags,
    /// Number of parallel codegen jobs (default: `num_cpus`).
    /// Configured via `ALBITE_CODEGEN_JOBS` env var.
    pub(crate) codegen_jobs: usize,
    /// Codegen units serialized onto one dedicated worker while the rest
    /// parallelize normally (ADR 3.7.26b Stub Registration). Configured via the
    /// `ALBITE_CODEGEN_SERIAL_UNITS` env var (comma-separated unit names);
    /// set by `albite-dev self-compile` for the known-pathological units.
    /// Never raises the worker count above `codegen_jobs`.
    pub(crate) codegen_serial_units: Vec<String>,
    /// `--error-format` (ADR 30.9.26j): selects how failures, warnings and
    /// child-tool output are reported. Records go to stderr.
    pub(crate) error_format: ErrorFormat,
    /// `ALBITE_NATIVE_CORE` (ADR 4.10.26n D6), read once by
    /// `dispatch_compile`: which concat a non-owned `++` calls, and whether
    /// the native core is compiled and linked.
    pub(crate) native_core: albite_codegen::NativeCoreSwitch,
}

/// Parse the `ALBITE_CODEGEN_JOBS` setting (default: `num_cpus`, minimum: 1).
pub(crate) fn parse_codegen_jobs() -> usize {
    albite_runtime::settings::read("CODEGEN_JOBS")
        .and_then(|s| s.parse::<usize>().ok())
        .map_or_else(
            || {
                std::thread::available_parallelism()
                    .map(std::num::NonZero::get)
                    .unwrap_or(4)
            },
            |n| n.max(1),
        )
}

/// Parse the `ALBITE_CODEGEN_SERIAL_UNITS` setting: comma-separated codegen
/// unit names to serialize onto one worker (ADR 3.7.26b Stub Registration).
pub(crate) fn parse_codegen_serial_units() -> Vec<String> {
    albite_runtime::settings::read("CODEGEN_SERIAL_UNITS")
        .map(|value| split_serial_units_value(&value))
        .unwrap_or_default()
}

/// Split an `ALBITE_CODEGEN_SERIAL_UNITS` value into unit names.
/// Whitespace around names is tolerated; empty entries are dropped.
fn split_serial_units_value(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(String::from)
        .collect()
}

/// Validate --no-codegen flag interactions.
///
/// `--emit-llvm` is an `IncompatibleFlags` failure (E0140). Otherwise the
/// codegen-only flags that will be ignored are returned, in the order they
/// were always warned about, for the caller to report in its format
/// (ADR 30.9.26j D9) — this function prints nothing.
pub(super) fn validate_no_codegen_flags(
    flags: &CompileFlags,
    output: Option<&std::path::Path>,
) -> Result<Vec<&'static str>, CompileFailure> {
    if flags.emit_llvm {
        return Err(CompileFailure::incompatible_no_codegen_and_emit_llvm());
    }
    let tracing = &flags.diagnostics.tracing;
    let candidates = [
        (flags.debug_info, "--debug-info"),
        (flags.sanitize, "--sanitize"),
        (flags.diagnostics.codegen_backtrace, "--codegen-backtrace"),
        (tracing.trace_adt_ops.is_some(), "--trace-adt-ops"),
        (tracing.trace_musttail, "--trace-musttail"),
        (tracing.trace_escape, "--trace-escape"),
        (tracing.trace_mono, "--trace-mono"),
        (!flags.diagnostics.only_units.is_empty(), "--only-unit"),
        (output.is_some(), "-o/--output"),
    ];
    let mut ignored = Vec::new();
    for (set, flag) in candidates {
        if set {
            ignored.push(flag);
        }
    }
    Ok(ignored)
}

/// Find the main function's type, or report a diagnostic error.
#[allow(clippy::cast_possible_truncation)] // Reason: source offset into a Span's u32; sources never reach 4 GiB
pub(super) fn find_main_type(
    file: &PathBuf,
    defs: &[CoreDef],
) -> Result<albite_core::Type, ExitCode> {
    use std::fs;
    let source = fs::read_to_string(file).unwrap_or_default();
    let eof_span = albite_bootstrap::span::Span::new(source.len() as u32, source.len() as u32);
    if let Some(d) = defs.iter().find(|d| d.name == "main") {
        Ok(d.ty.clone())
    } else {
        let err = albite_bootstrap::ElabError::no_main_function(eof_span);
        driver::render_diagnostics(&source, &file.to_string_lossy(), &[err], &[]);
        Err(ExitCode::FAILURE)
    }
}

mod cmd_compile;
pub(crate) use cmd_compile::cmd_compile;

pub(crate) use extern_naming::extern_wrap_name;
