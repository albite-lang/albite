//! Per-module codegen entry point: temp dir, emit-llvm, linking (ADR 7.5.26h).
//!
//! This module contains the top-level `run_codegen_per_module` function and its
//! helpers. The per-unit compilation logic lives in the parent `per_module::mod`.

use std::path::Path;
use std::path::PathBuf;

use albite_bootstrap::driver;
use albite_bootstrap::driver::diagnostics::compile_failure::{BuildErrorKind, CompileFailure};

use super::emit_paths;
use super::emit_paths::resolve_emit_llvm_dir;
use super::run_per_module_codegen;
use crate::compile::native_core::{LinkTarget, NativeCoreArtifact, NATIVE_CORE_UNIT};
use crate::compile::CompileFlags;

/// Per-module codegen entry point: emit separate `.ll` files and link (ADR 7.5.26h).
///
/// Creates a temp directory, compiles each unit, handles `--emit-llvm`, and links.
/// With `--emit-llvm`, produces a mirror directory tree under `target/ll/` (or `-o <dir>`).
pub(in crate::compile) fn run_codegen_per_module(
    file: &PathBuf,
    target: LinkTarget<'_>,
    flags: &CompileFlags,
    project: &driver::ProjectOutput,
    main_ty: &albite_core::types::Type,
) -> Result<(), CompileFailure> {
    let (output, native_core) = (target.output, target.native_core);
    // ADR 10.5.26j §2.2: Initialize Chrome trace layer when profile feature is active.
    // The guard must live through compilation + linking; dropping flushes the trace file.
    #[cfg(feature = "profile")]
    let _trace_guard = init_trace_subscriber();

    let tmp_dir = create_codegen_temp_dir()?;

    let codegen_output = super::CodegenOutput {
        dir: tmp_dir.path(),
        musttail_sink: None,
        unit_cost_sink: None,
    };
    let mut modules = run_per_module_codegen(file, flags, project, main_ty, &codegen_output)
        .map_err(|e| CompileFailure::build(BuildErrorKind::CodeGenerationFailed, e))?;
    // The native core's object joins the link, its `.ll` the tree (ADR 4.10.26n D3).
    modules.extend(native_core.map(NativeCoreArtifact::module));

    if flags.emit_llvm {
        let output_dir = resolve_emit_llvm_dir(file, output);
        let source_root = file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(&output_dir).map_err(|e| {
            not_writable(format!(
                "could not create output dir '{}': {}",
                output_dir.display(),
                e
            ))
        })?;
        // Copy .ll files into mirror directory structure (ADR 9.5.26b §2.4)
        // Per-function units: target/ll/<module_path>/<function_name>.ll
        // The modules list may include synthetic units (like __mono) that
        // aren't in project.codegen_units, so iterate modules directly.
        for module in &modules {
            if module.name == NATIVE_CORE_UNIT {
                let dest = NativeCoreArtifact::emit_llvm_dest(&output_dir);
                create_parent_dir(&dest)?;
                copy_ll(&module.output_path, &dest)?;
                continue;
            }
            if module.name == crate::compile::mono::MONO_DEPOT_UNIT {
                // Copy __mono.ll to output root
                let dest = emit_paths::mono_depot_dest(&output_dir, &module.name);
                copy_ll(&module.output_path, &dest)?;
                continue;
            }
            // Find the matching codegen unit for source path info
            let unit = match project.codegen_units.iter().find(|u| {
                let name = crate::compile::per_module::codegen_unit_name(
                    &u.source_file,
                    source_root,
                    &u.defs[0].name,
                );
                name == module.name
            }) {
                Some(u) => u,
                None => continue,
            };
            // The destination rule is shared with `info codegen unit-paths`
            // (ADR 28.7.26e retrospective) — do not re-derive it here.
            let origin = emit_paths::UnitOrigin {
                source_file: &unit.source_file,
                def_name: &unit.defs[0].name,
            };
            let dest = emit_paths::emit_llvm_dest(&origin, source_root, &output_dir)
                .map_err(|e| not_writable(e.to_string()))?;
            create_parent_dir(&dest)?;
            copy_ll(&module.output_path, &dest)?;
        }
        crate::compile::linking::emit_per_module_ll(&modules, &output_dir, flags.verbose);
        return Ok(());
    }

    // Link compiled modules → binary
    let link_start = std::time::Instant::now();
    let result = crate::compile::linking::link_modules(&modules, file, output, flags);
    if flags.verbose {
        eprintln!("[perf] link={:.2}s", link_start.elapsed().as_secs_f64());
    }
    result
}

/// An `OutputNotWritable` failure (E0146) with this site's text.
fn not_writable(message: String) -> CompileFailure {
    CompileFailure::build(BuildErrorKind::OutputNotWritable, message)
}

/// Create the directory `dest` will be written into.
fn create_parent_dir(dest: &Path) -> Result<(), CompileFailure> {
    let Some(parent) = dest.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|e| {
        not_writable(format!(
            "could not create dir '{}': {}",
            parent.display(),
            e
        ))
    })
}

/// Copy one emitted `.ll` into the `--emit-llvm` mirror tree.
fn copy_ll(from: &Path, dest: &Path) -> Result<(), CompileFailure> {
    std::fs::copy(from, dest)
        .map(|_| ())
        .map_err(|e| not_writable(format!("could not copy '{}': {}", dest.display(), e)))
}

/// Create a temp directory for codegen intermediates (.ll, .o files).
fn create_codegen_temp_dir() -> Result<tempfile::TempDir, CompileFailure> {
    tempfile::tempdir().map_err(|e| not_writable(format!("could not create temp dir: {e}")))
}

/// Initialize the Chrome trace subscriber (ADR 10.5.26j §2.2).
///
/// Returns a flush guard that must live until the end of compilation.
/// Uses `try_init()` semantics so profile builds do not panic if another
/// subscriber has already been installed.
#[cfg(feature = "profile")]
fn init_trace_subscriber() -> Option<tracing_chrome::FlushGuard> {
    use tracing_chrome::ChromeLayerBuilder;
    use tracing_subscriber::prelude::*;

    let trace_path = albite_runtime::settings::read("TRACE_FILE")
        .unwrap_or_else(|| "target/trace.json".to_string());
    let (chrome_layer, guard) = ChromeLayerBuilder::new()
        .file(&trace_path)
        .include_args(true)
        .build();
    match tracing_subscriber::registry().with(chrome_layer).try_init() {
        Ok(()) => {
            eprintln!("[profile] Tracing to {trace_path}");
            Some(guard)
        }
        Err(e) => {
            eprintln!("[profile] Could not initialize tracing: {e}");
            None
        }
    }
}

#[cfg(all(test, feature = "profile"))]
mod tests {
    #[test]
    fn test_init_trace_subscriber_does_not_panic_on_double_init() {
        // Point trace file at a real temp dir so ChromeLayerBuilder doesn't panic
        let tmp = tempfile::tempdir().expect("create temp dir");
        let trace_path = tmp.path().join("test-trace.json");
        std::env::set_var(
            albite_runtime::settings::name("TRACE_FILE"),
            trace_path.to_str().unwrap(),
        );

        let result = super::init_trace_subscriber();
        // First call may succeed or return None (if another test already init'd)
        // Either way, it must not panic.
        drop(result);

        // Second call should also not panic — try_init returns Err, we get None
        let second = super::init_trace_subscriber();
        assert!(
            second.is_none(),
            "double-init should return None, not panic"
        );

        std::env::remove_var(albite_runtime::settings::name("TRACE_FILE"));
    }
}
