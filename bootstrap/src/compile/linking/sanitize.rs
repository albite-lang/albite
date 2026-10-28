//! `AddressSanitizer` pipeline for compiled Tungsten programs.
//!
//! Routes through clang's `ASan` instrumentation instead of direct object
//! emission, so that `ASan` module+function passes are applied to the
//! generated LLVM IR.

use std::path::{Path, PathBuf};
use std::process::Command;

use albite_bootstrap::driver::diagnostics::compile_failure::{BuildErrorKind, CompileFailure};

use super::link::run_tool;
use super::{strip_flags, LinkerCommand};

/// Emit a sanitized binary by routing through clang's `ASan` pipeline.
///
/// Instead of direct object emission (which bypasses sanitizer passes), we:
/// 1. Emit LLVM IR to a temp file
/// 2. Use `clang -fsanitize=address` to compile+link in one step
///
/// This lets clang run the `ASan` module+function passes on the IR before
/// code generation, instrumenting all memory accesses in the Tungsten-generated code.
pub(super) fn emit_sanitized(
    codegen: &albite_codegen::CodeGen<'_>,
    file: &PathBuf,
    output: Option<&std::path::Path>,
    verbose: bool,
    extra_inputs: &[PathBuf],
) -> Result<(), CompileFailure> {
    use std::fs;

    let ir = codegen.get_ir_string();

    // Write IR to temp file
    let not_writable =
        |message: String| CompileFailure::build(BuildErrorKind::OutputNotWritable, message);
    let tmp_dir =
        tempfile::tempdir().map_err(|e| not_writable(format!("could not create temp dir: {e}")))?;
    let ll_path = tmp_dir.path().join("module.ll");
    fs::write(&ll_path, &ir).map_err(|e| not_writable(format!("could not write temp IR: {e}")))?;

    let linker = LinkerCommand::new(file, output, true, verbose);
    let mut cmd = sanitize_command(&linker, &ll_path, extra_inputs);

    if verbose {
        eprintln!(
            "ASan pipeline: clang -fsanitize=address {} → {}",
            ll_path.display(),
            linker.exe_path.display()
        );
    }

    run_tool(
        &mut cmd,
        |s| {
            CompileFailure::build(
                BuildErrorKind::ToolFailed,
                format!("clang -fsanitize=address failed with status {s}"),
            )
            .with_help("make sure 'clang' is installed and in PATH")
        },
        |e| {
            CompileFailure::build(
                BuildErrorKind::ToolUnavailable,
                format!("could not run clang: {e}"),
            )
            .with_help("ASan requires clang. Install with: brew install llvm (macOS) or apt install clang (Linux)")
        },
    )?;
    println!(
        "✓ Compiled to {} (AddressSanitizer enabled)",
        linker.exe_path.display()
    );
    Ok(())
}

/// The `clang -fsanitize=address` command that compiles `ll_path` and links
/// it with `extra_inputs` and `libalbite_core.a` in one step. Pure: it only
/// builds the command, so its arguments are testable (ADR 6.10.26i D2 A).
///
/// clang applies `ASan` passes to the IR, instruments memory accesses, and
/// links against the `ASan` runtime automatically. Links `albite_core` as a
/// static archive (ADR 18.5.26e §2.3), stripped like every user link.
pub(super) fn sanitize_command(
    linker: &LinkerCommand,
    ll_path: &Path,
    extra_inputs: &[PathBuf],
) -> Command {
    let static_lib = linker.lib_dir.join("libalbite_core.a");
    let mut cmd = Command::new("clang");
    cmd.arg("-fsanitize=address")
        .arg(ll_path)
        .args(extra_inputs)
        .arg("-o")
        .arg(&linker.exe_path)
        .arg(&static_lib);

    #[cfg(target_os = "linux")]
    {
        cmd.args([
            "-lgcc_s",
            "-lutil",
            "-lrt",
            "-lpthread",
            "-lm",
            "-ldl",
            "-lc",
        ]);
    }
    // macOS: clang implicitly links libSystem (provides libc + libm), so no
    // explicit system libs are needed there.

    cmd.args(strip_flags(std::env::consts::OS));
    cmd
}
