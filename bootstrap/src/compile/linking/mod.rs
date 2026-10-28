//! Linking and output emission for compiled Tungsten programs.
//!
//! Handles emitting LLVM IR files, writing object files, linking native
//! executables, the `AddressSanitizer` pipeline via clang, and per-module
//! `.ll` → `.o` → binary linking (ADR 6.5.26c §2.7). The link half lives in
//! `link.rs` (ADR 30.9.26j §2.2). Every failure is a `CompileFailure`.

use std::path::{Path, PathBuf};
use std::process::Command;

use albite_bootstrap::driver::diagnostics::compile_failure::{BuildErrorKind, CompileFailure};

use super::native_core::{LinkTarget, NativeCoreArtifact};
use super::per_module::CompiledModule;
use super::CompileFlags;

mod link;
mod sanitize;
mod staticlib_check;

use link::link_executable;
pub(super) use link::link_modules;

/// Shared linker configuration, extracted from `link_executable`, `link_object_files`,
/// and `emit_sanitized` to eliminate duplication (ADR 10.5.26a §2.2).
pub(super) struct LinkerCommand {
    pub(super) exe_path: PathBuf,
    pub(super) lib_dir: PathBuf,
    pub(super) sanitize: bool,
    pub(super) verbose: bool,
}

impl LinkerCommand {
    pub(super) fn new(file: &Path, output: Option<&Path>, sanitize: bool, verbose: bool) -> Self {
        let exe_path = output.map_or_else(
            || {
                #[cfg(target_os = "windows")]
                {
                    file.with_extension("exe")
                }
                #[cfg(not(target_os = "windows"))]
                {
                    file.with_extension("")
                }
            },
            std::path::Path::to_path_buf,
        );
        let lib_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("target/release"));
        Self {
            exe_path,
            lib_dir,
            sanitize,
            verbose,
        }
    }

    /// Build a `cc` command with common flags, accepting input files separately.
    ///
    /// Inputs are added in the order provided — callers are responsible for
    /// sorting object files for deterministic linking.
    /// Uses `cc -fuse-ld=lld` when `lld` is available (ADR 9.5.26d §2.4).
    pub(super) fn build_cc(&self, inputs: &[PathBuf]) -> Command {
        let mut cmd = Command::new("cc");

        // Prefer lld when available (ADR 9.5.26d §2.4).
        // Skip on macOS: ld64.lld's MachO backend has incomplete support —
        // notably -stack_size is unimplemented (silently ignored), which
        // causes stack overflows in large programs. Apple's system ld64
        // handles -stack_size correctly.
        #[cfg(not(target_os = "macos"))]
        if lld_available() {
            cmd.arg("-fuse-ld=lld");
        }

        for input in inputs {
            cmd.arg(input);
        }
        cmd.arg("-o").arg(&self.exe_path);

        // Link albite_core as a static archive (ADR 18.5.26e §2.2).
        // Use the full path to the .a file to avoid accidental dynamic linking.
        let static_lib = self.lib_dir.join("libalbite_core.a");
        cmd.arg(&static_lib);

        // Rust's staticlib does not bundle its transitive C dependencies the way
        // cdylib does. Add them explicitly per platform.
        // Authoritative source: `rustc --print native-static-libs --crate-type staticlib`
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
            // No `-z stack-size`: glibc sizes the main thread from
            // RLIMIT_STACK and never reads PT_GNU_STACK's size, so the flag
            // was a no-op. The program body runs on a thread the runtime
            // sizes instead (`__albite_run_on_stack`, ADR 19.10.26c D5).
        }
        #[cfg(target_os = "macos")]
        {
            // cc (Apple Clang) implicitly links libSystem, which provides libc
            // and libm. No explicit system libs needed — adding them causes
            // "ld: warning: ignoring duplicate libraries: '-lSystem'".
            //
            // Set stack size to 128 MB — elaboration of large programs (e.g., the
            // compiler checking itself) can exceed the default 8 MB.
            // See ADR 20.5.26e (tail-recursive list operations). ld64 does
            // size the main thread, which is the one the runtime's fallback
            // runs the body on when its own thread cannot start (ADR 19.10.26c).
            cmd.args(["-Wl,-stack_size,0x8000000"]);
        }

        // A program carries only the core code it calls (ADR 6.10.26i D2 A).
        cmd.args(strip_flags(std::env::consts::OS));

        if self.sanitize {
            cmd.arg("-fsanitize=address");
        }
        cmd
    }
}

/// The linker flag that drops unreferenced sections on a user-program link,
/// keyed on a `std::env::consts::OS` spelling (ADR 6.10.26i D2 A).
///
/// `libalbite_core.a` bundles everything the core and its dependencies
/// compile to, and the linker pulls a whole archive member to resolve one
/// symbol — so without this, code nothing calls (4.10.26k's `toml`) rides
/// along into every program. `rustc` builds the core's objects to be stripped
/// (one function per section on ELF, subsections-via-symbols on Mach-O).
/// Any other OS answers no flag: an explicit arm, not a fall-through.
pub(super) fn strip_flags(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["-Wl,-dead_strip"],
        "linux" => &["-Wl,--gc-sections"],
        _ => &[],
    }
}

/// Check if `lld` is available via `cc -fuse-ld=lld` (ADR 9.5.26d §2.4).
///
/// Probes once per process via a cached result. Falls back to default `cc`
/// linking when `lld` is not installed.
///
/// Skipped on macOS: `ld64.lld` has incomplete `MachO` support (e.g., `-stack_size`
/// is unimplemented). We always use Apple's system ld64 there.
#[cfg(not(target_os = "macos"))]
fn lld_available() -> bool {
    use std::sync::OnceLock;
    static LLD_AVAILABLE: OnceLock<bool> = OnceLock::new();
    *LLD_AVAILABLE.get_or_init(|| {
        // Use -nostdlib -nostartfiles to avoid "undefined symbol: main" from
        // empty input. We only need to check that `cc` accepts `-fuse-ld=lld`
        // and can invoke lld without error.
        Command::new("cc")
            .args([
                "-fuse-ld=lld",
                "-nostdlib",
                "-nostartfiles",
                "-shared",
                "-x",
                "c",
                "-",
                "-o",
                "/dev/null",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Emit compiled output: LLVM IR file or native executable (object + link).
pub(super) fn emit_output(
    codegen: &albite_codegen::CodeGen<'_>,
    file: &PathBuf,
    target: LinkTarget<'_>,
    flags: &CompileFlags,
) -> Result<(), CompileFailure> {
    use std::fs;

    let output = target.output;
    // The native core's object is one more link input, before
    // `libalbite_core.a` (ADR 4.10.26n D3).
    let core_inputs: Vec<PathBuf> = target
        .native_core
        .map(|core| core.module().output_path)
        .into_iter()
        .collect();

    if flags.emit_llvm {
        let ir = codegen.get_ir_string();
        let output_path =
            output.map_or_else(|| file.with_extension("ll"), std::path::Path::to_path_buf);

        if let Err(e) = fs::write(&output_path, ir) {
            // The one site that has always exited 3, as an I/O failure (D3).
            return Err(CompileFailure::build(
                BuildErrorKind::OutputNotWritable,
                format!("could not write '{}': {}", output_path.display(), e),
            )
            .with_exit(3));
        }

        if let Some(core_ll) = core_inputs.first() {
            let dir = output_path.parent().unwrap_or(Path::new("."));
            let dest = NativeCoreArtifact::emit_llvm_dest(dir);
            let copied = fs::create_dir_all(dest.parent().unwrap_or(dir))
                .and_then(|()| fs::copy(core_ll, &dest));
            if let Err(e) = copied {
                return Err(CompileFailure::build(
                    BuildErrorKind::OutputNotWritable,
                    format!("could not write '{}': {}", dest.display(), e),
                ));
            }
        }
        println!("✓ Wrote LLVM IR to {}", output_path.display());
        return Ok(());
    }

    // Write object file
    if flags.verbose {
        eprintln!("Writing object file...");
    }

    // When --sanitize is enabled, go through an IR→clang path so that clang
    // can apply ASan instrumentation passes to the generated LLVM IR.
    // Direct object emission skips ASan passes, defeating the purpose.
    if flags.sanitize {
        return sanitize::emit_sanitized(codegen, file, output, flags.verbose, &core_inputs);
    }

    let obj_path = file.with_extension("o");
    if let Err(e) = codegen.write_object_file(&obj_path) {
        return Err(CompileFailure::build(
            BuildErrorKind::CodeGenerationFailed,
            format!("could not write object file: {e}"),
        ));
    }

    if flags.verbose {
        eprintln!("Wrote object file to {}", obj_path.display());
    }

    // Link with system linker
    let mut inputs = vec![obj_path];
    inputs.extend(core_inputs);
    link_executable(file, output, &inputs, flags.verbose, flags.sanitize)
}

/// Emit per-module `.ll` files (for --emit-llvm with per-module codegen).
pub(super) fn emit_per_module_ll(modules: &[CompiledModule], output_dir: &Path, verbose: bool) {
    for module in modules {
        if verbose {
            eprintln!("  {}", module.output_path.display());
        }
    }
    println!(
        "✓ Wrote {} LLVM IR file(s) to {}",
        modules.len(),
        output_dir.display()
    );
}

#[cfg(test)]
mod tests;
