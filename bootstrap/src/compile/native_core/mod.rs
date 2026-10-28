//! The native core's code-generation run (ADR 4.10.26n D3).
//!
//! With `ALBITE_NATIVE_CORE=on`, `albite compile` elaborates the embedded
//! core (`albite_bootstrap::driver::native_core`) as its own project and
//! lowers it as its own LLVM module, independent of the program's, so the
//! program's names and the core's never meet. The module then:
//!
//! 1. defines each `NATIVE_CORE_EXPORTS` row as a C-ABI shim (D2), and
//! 2. forces `__albite_native_core__` onto every other external definition.
//!
//! Its object joins the program's link, before `libalbite_core.a`; under
//! `--emit-llvm` its `.ll` joins the mirror tree as `__native_core/native_core.ll`,
//! so every path that globs the tree — `albite-dev`'s self-compile and the
//! `make` recipes — links it with no change (P0.5).
//!
//! It runs BEFORE the program is elaborated: elaborating a project resets
//! process-wide registries (the comparator requests), which the program's
//! own code generation reads.

use std::path::{Path, PathBuf};

use albite_bootstrap::driver::diagnostics::compile_failure::{BuildErrorKind, CompileFailure};
use albite_bootstrap::driver::native_core::rules::{check_core_definitions, write_cover_refusal};
use albite_bootstrap::driver::native_core::switch::{
    native_core_choice_from_env, NativeCoreChoice,
};
use albite_bootstrap::driver::native_core::{elaborate_native_core, NATIVE_CORE_SOURCES};
use albite_codegen::inkwell::OptimizationLevel;
use albite_codegen::NativeCoreSwitch;

use super::per_module::{CompiledModule, OutputKind};
use super::{compile_all_defs, convert_adt_types_for_codegen, declare_and_register_defs};

/// Defined in the lib (ADR 28.10.26e D4), so the `--no-default-features` IR
/// audits that skip this unit's directory read the name this run writes.
pub(crate) use albite_bootstrap::driver::native_core::NATIVE_CORE_UNIT;

/// The core's file inside that directory.
pub(crate) const NATIVE_CORE_LL: &str = "native_core.ll";

/// `ALBITE_NATIVE_CORE`, read once, as the code generator's switch.
///
/// # Errors
/// A value other than `on`, `off` or `all`.
pub(crate) fn switch_from_env() -> Result<NativeCoreSwitch, String> {
    native_core_choice_from_env().map(|choice| match choice {
        NativeCoreChoice::On => NativeCoreSwitch::On,
        NativeCoreChoice::Off => NativeCoreSwitch::Off,
        NativeCoreChoice::All => NativeCoreSwitch::All,
    })
}

/// The core's compiled file, and the scratch directory that holds it, which
/// lives until the link is done.
pub(crate) struct NativeCoreArtifact {
    _scratch: tempfile::TempDir,
    module: CompiledModule,
}

impl NativeCoreArtifact {
    /// The core as one more module to link or to copy into the IR tree.
    pub(crate) fn module(&self) -> CompiledModule {
        CompiledModule {
            output_path: self.module.output_path.clone(),
            name: self.module.name.clone(),
            kind: self.module.kind,
        }
    }

    /// Where `--emit-llvm` puts the core's `.ll` under `output_dir`.
    pub(crate) fn emit_llvm_dest(output_dir: &Path) -> PathBuf {
        output_dir.join(NATIVE_CORE_UNIT).join(NATIVE_CORE_LL)
    }
}

/// Where a build's output goes, and what joins its link besides the
/// program's own objects: the native core's, when the switch is on.
#[derive(Clone, Copy)]
pub(crate) struct LinkTarget<'a> {
    /// `-o`, if given.
    pub(crate) output: Option<&'a Path>,
    /// The native core's compiled file, under `ALBITE_NATIVE_CORE=on`.
    pub(crate) native_core: Option<&'a NativeCoreArtifact>,
}

/// [`switch_from_env`], with a bad value reported on stderr: `Err` is the
/// exit code `albite compile` stops with.
pub(crate) fn switch_or_report() -> Result<NativeCoreSwitch, std::process::ExitCode> {
    switch_from_env().map_err(|message| {
        eprintln!("error: {message}");
        std::process::ExitCode::from(2)
    })
}

fn core_failure(message: impl std::fmt::Display) -> CompileFailure {
    CompileFailure::build(
        BuildErrorKind::CodeGenerationFailed,
        format!("native core: {message}"),
    )
}

/// `--alloc-profile`'s value as the code generator's filter: empty profiles
/// every function, anything else names the functions to profile.
pub(crate) fn alloc_profile_filter(flag: &str) -> Option<String> {
    (!flag.is_empty()).then(|| flag.to_string())
}

/// The level the core's object is written at, whatever the program's
/// (ADR 13.10.26p D2). The core's IR is fixed, so the machine code it becomes
/// is fixed too: written at the program's `None`, `FastISel` and the fast
/// register allocator spilled every value of the owned join to the stack.
/// Program objects stay at `None`; `albite1`'s `llc` link sets its own.
pub(crate) const NATIVE_CORE_OBJECT_LEVEL: OptimizationLevel = OptimizationLevel::Default;

/// What one build asks of the core's run.
pub(crate) struct NativeCoreBuild<'a> {
    /// An object when set, else `.ll` text.
    pub(crate) emit_obj: bool,
    /// Print the core's run time.
    pub(crate) verbose: bool,
    /// The program's `--alloc-profile` value, which the core's allocations
    /// are recorded under too (D5).
    pub(crate) alloc_profile: Option<&'a str>,
    /// The program's switch, so the core's own byte checks call the trap
    /// row its setting enables (ADR 7.10.26q D3).
    pub(crate) switch: NativeCoreSwitch,
}

/// Elaborate and lower the core, run its pass pipeline, and write it in the
/// form `build` asks for.
///
/// # Errors
/// The core does not elaborate, does not lower, does not optimise, or cannot
/// be written.
pub(crate) fn compile_native_core(
    build: &NativeCoreBuild<'_>,
) -> Result<NativeCoreArtifact, CompileFailure> {
    compile_native_core_at(build, NATIVE_CORE_OBJECT_LEVEL)
}

/// [`compile_native_core`], with the object written at `object_level`; split
/// out so a test can write the same core at another level and compare.
fn compile_native_core_at(
    build: &NativeCoreBuild<'_>,
    object_level: OptimizationLevel,
) -> Result<NativeCoreArtifact, CompileFailure> {
    use albite_codegen::inkwell::context::Context as LlvmContext;
    use albite_codegen::CodeGen;

    let started = std::time::Instant::now();
    let scratch = tempfile::tempdir().map_err(|e| core_failure(format!("scratch dir: {e}")))?;
    let source_dir = scratch.path().join("src");
    std::fs::create_dir_all(&source_dir).map_err(core_failure)?;
    let mut project = elaborate_native_core(&source_dir).map_err(core_failure)?;
    let violations = check_core_definitions(&project.defs, &NATIVE_CORE_SOURCES);
    // The write-cover rule stands in for a zero-fill (ADR 9.10.26n D3), so it
    // refuses the run in every build, release included.
    if let Some(refusal) = write_cover_refusal(&violations) {
        return Err(core_failure(refusal));
    }
    debug_assert!(
        violations.is_empty(),
        "the embedded native core breaks its own rules: {violations:?}"
    );
    for def in &mut project.defs {
        *def = def.erase_for_codegen();
    }

    let llvm_context = LlvmContext::create();
    let mut codegen = CodeGen::new(&llvm_context, NATIVE_CORE_UNIT);
    codegen.set_native_core_run();
    codegen.set_native_core_switch(build.switch);
    if let Some(flag) = build.alloc_profile {
        codegen.set_alloc_profile(alloc_profile_filter(flag));
    }
    codegen.register_record_types(project.record_types.clone());
    codegen.register_adt_types(convert_adt_types_for_codegen(project.adt_types.clone()));
    let names = declare_and_register_defs(&mut codegen, &project.defs).map_err(core_failure)?;
    compile_all_defs(&mut codegen, &project.defs, &names, false).map_err(core_failure)?;
    codegen.emit_native_core_exports().map_err(core_failure)?;
    codegen.prefix_native_core_symbols();
    codegen.optimize_native_core().map_err(core_failure)?;

    let (output_path, kind) = if build.emit_obj {
        let path = scratch.path().join("native_core.o");
        codegen
            .write_object_file_with_opt(&path, object_level)
            .map_err(core_failure)?;
        (path, OutputKind::Obj)
    } else {
        let path = scratch.path().join(NATIVE_CORE_LL);
        std::fs::write(&path, codegen.get_ir_string()).map_err(core_failure)?;
        (path, OutputKind::Ll)
    };
    if build.verbose {
        eprintln!(
            "[perf] native core: {:.2}s ({} definition(s))",
            started.elapsed().as_secs_f64(),
            project.defs.len()
        );
    }
    Ok(NativeCoreArtifact {
        _scratch: scratch,
        module: CompiledModule {
            output_path,
            name: NATIVE_CORE_UNIT.to_string(),
            kind,
        },
    })
}

#[cfg(test)]
mod args_tests;
#[cfg(test)]
mod fill_tests;
#[cfg(test)]
mod level_tests;
#[cfg(test)]
mod tests;
