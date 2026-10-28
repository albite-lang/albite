//! The link half of `linking/` (ADR 30.9.26j §2.2): running the linker and
//! `llc`, and turning their failures into [`CompileFailure`] values.
//!
//! A child tool inherits stderr in human mode, so its own lines print above
//! compile's `error:` line exactly as before; in JSON mode its output is
//! captured into the failure's notes instead (D6).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use albite_bootstrap::driver::diagnostics::compile_failure::{
    capture_child_output, BuildErrorKind, CompileFailure,
};
use albite_bootstrap::driver::diagnostics::error_format;

use super::super::per_module::CompiledModule;
use super::super::CompileFlags;
use super::{staticlib_check, LinkerCommand};

/// Run one child tool to completion. In JSON mode its stdout and stderr are
/// captured, and on failure they become the failure's notes (D6); on success
/// they are discarded. `failed` names a non-zero exit, `unavailable` a tool
/// that could not be spawned.
pub(super) fn run_tool(
    cmd: &mut Command,
    failed: impl FnOnce(ExitStatus) -> CompileFailure,
    unavailable: impl FnOnce(std::io::Error) -> CompileFailure,
) -> Result<(), CompileFailure> {
    if capture_child_output(error_format()) {
        let captured = cmd.output().map_err(unavailable)?;
        if captured.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&captured.stderr);
        let stdout = String::from_utf8_lossy(&captured.stdout);
        return Err(failed(captured.status).with_tool_output(&stderr, &stdout));
    }
    match cmd.status().map_err(unavailable)? {
        status if status.success() => Ok(()),
        status => Err(failed(status)),
    }
}

impl LinkerCommand {
    /// Run the linker command, diagnosing an unusable runtime archive first.
    pub(super) fn run_link(&self, mut cmd: Command) -> Result<(), CompileFailure> {
        if self.verbose {
            eprintln!("Library directory: {}", self.lib_dir.display());
        }
        // A devcontainer build can clobber the shared static library with the
        // other platform's archive format (ADR 2.7.26b) — diagnose that here
        // instead of surfacing a cryptic `ld` error. Fail-open on any doubt.
        // A missing archive is likewise diagnosed by name (guaranteed failure).
        let static_lib = self.lib_dir.join("libalbite_core.a");
        if let Some(diagnosis) = staticlib_check::missing_staticlib_diagnosis(&static_lib)
            .or_else(|| staticlib_check::foreign_platform_diagnosis(&static_lib))
        {
            // Pre-formatted text, carried verbatim (D3).
            return Err(CompileFailure::preformatted(
                BuildErrorKind::RuntimeLibraryUnusable,
                &diagnosis,
            ));
        }
        run_tool(
            &mut cmd,
            |s| {
                CompileFailure::build(
                    BuildErrorKind::ToolFailed,
                    format!("linker failed with status {s}"),
                )
            },
            |e| {
                CompileFailure::build(
                    BuildErrorKind::ToolUnavailable,
                    format!("could not run linker: {e}"),
                )
                .with_help("make sure 'cc' (C compiler) is installed and in PATH")
            },
        )
    }
}

/// Link object files into an executable using the system linker: the
/// program's own first, then any extra input (the native core's object).
pub(super) fn link_executable(
    file: &PathBuf,
    output: Option<&Path>,
    inputs: &[PathBuf],
    verbose: bool,
    sanitize: bool,
) -> Result<(), CompileFailure> {
    let linker = LinkerCommand::new(file, output, sanitize, verbose);
    let cmd = linker.build_cc(inputs);

    if verbose && sanitize {
        eprintln!("AddressSanitizer enabled: linking with -fsanitize=address");
    }

    let result = linker.run_link(cmd);

    // Clean up the program's object file; the native core's lives in its own
    // scratch directory, removed when the artifact drops.
    if let Some(obj_path) = inputs.first() {
        let _ = std::fs::remove_file(obj_path);
    }

    if result.is_ok() {
        println!("✓ Compiled to {}", linker.exe_path.display());
    }
    result
}

/// Link multiple compiled modules into a single executable (ADR 6.5.26c §2.7).
///
/// When modules are `.o` (in-process object emission, ADR 9.5.26e §2.1),
/// skips `llc` assembly and links directly. When modules are `.ll`, runs
/// `llc` on each to produce `.o` first.
pub(in crate::compile) fn link_modules(
    modules: &[CompiledModule],
    file: &PathBuf,
    output: Option<&Path>,
    flags: &CompileFlags,
) -> Result<(), CompileFailure> {
    use super::super::per_module::OutputKind;

    #[cfg(feature = "profile")]
    let _span = tracing::info_span!("link").entered();

    // Step 1: Get .o paths — either directly (Obj) or via llc assembly (Ll)
    let all_obj = modules.iter().all(|m| m.kind == OutputKind::Obj);
    let mut obj_paths = if all_obj {
        // In-process .o emission: modules are already .o, skip llc entirely
        modules.iter().map(|m| m.output_path.clone()).collect()
    } else {
        assemble_modules(modules, flags.verbose)?
    };

    // Step 2: Sort object paths for deterministic link order (ADR 10.5.26a §2.2).
    obj_paths.sort();
    link_object_files(&obj_paths, file, output, flags)
}

/// Run `llc` on each module's `.ll` file to produce `.o` files.
fn assemble_modules(
    modules: &[CompiledModule],
    verbose: bool,
) -> Result<Vec<PathBuf>, CompileFailure> {
    let mut obj_paths = Vec::new();
    for module in modules {
        let obj_path = module.output_path.with_extension("o");

        let mut llc_cmd = Command::new("llc");
        llc_cmd
            .arg("-filetype=obj")
            .arg(&module.output_path)
            .arg("-o")
            .arg(&obj_path);

        if verbose {
            eprintln!(
                "  llc {} → {}",
                module.output_path.display(),
                obj_path.display()
            );
        }

        run_tool(
            &mut llc_cmd,
            |s| {
                CompileFailure::build(
                    BuildErrorKind::ToolFailed,
                    format!("llc failed for module '{}' with status {}", module.name, s),
                )
            },
            |e| {
                CompileFailure::build(
                    BuildErrorKind::ToolUnavailable,
                    format!("could not run llc: {e}"),
                )
                .with_help("make sure 'llc' is installed and in PATH")
            },
        )?;
        obj_paths.push(obj_path);
    }
    Ok(obj_paths)
}

/// Link multiple `.o` files into an executable.
fn link_object_files(
    obj_paths: &[PathBuf],
    file: &PathBuf,
    output: Option<&Path>,
    flags: &CompileFlags,
) -> Result<(), CompileFailure> {
    let linker = LinkerCommand::new(file, output, flags.sanitize, flags.verbose);
    let cmd = linker.build_cc(obj_paths);

    if flags.verbose {
        eprintln!(
            "Linking {} object file(s) → {}",
            obj_paths.len(),
            linker.exe_path.display()
        );
    }

    let result = linker.run_link(cmd);

    for obj_path in obj_paths {
        let _ = std::fs::remove_file(obj_path);
    }

    if result.is_ok() {
        println!("✓ Compiled to {}", linker.exe_path.display());
    }
    result
}
