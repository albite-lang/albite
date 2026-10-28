//! Compile command implementation and codegen configuration.

use std::path::PathBuf;
use std::process::ExitCode;

use super::CompileFlags;
use super::{compile_all_defs, declare_and_register_defs, warn_sorry_defs};
use super::{convert_adt_types_for_codegen, find_main_type, validate_no_codegen_flags};
use super::{diagnostics, linking, native_core, per_module, validation};
use albite_bootstrap::driver::diagnostics::compile_failure::{
    self, BuildErrorKind, CompileFailure, Sinks,
};
use albite_bootstrap::driver::diagnostics::{set_error_format, set_record_stream};
use albite_bootstrap::driver::diagnostics::{ErrorFormat, RecordStream};
use albite_bootstrap::driver::{self};

/// Compile command: compile to native executable.
///
/// The one place a [`CompileFailure`] becomes an `ExitCode` (ADR 30.9.26j).
/// With `--error-format json` every record goes to **stderr**, because
/// stdout is compile's own product (D5): `✓ Compiled to …` and the dumps.
pub(crate) fn cmd_compile(
    file: &PathBuf,
    output: Option<&std::path::Path>,
    flags: &CompileFlags,
) -> ExitCode {
    set_error_format(flags.error_format);
    set_record_stream(RecordStream::Stderr);
    match compile_or_fail(file, output, flags) {
        Ok(code) => code,
        Err(failure) => ExitCode::from(with_process_sinks(|sinks| {
            compile_failure::report(&failure, flags.error_format, sinks)
        })),
    }
}

/// Hand the process's own stdout and stderr to a report.
fn with_process_sinks<R>(write: impl FnOnce(&mut Sinks<'_>) -> R) -> R {
    let (stdout, stderr) = (std::io::stdout(), std::io::stderr());
    write(&mut Sinks {
        stdout: &mut stdout.lock(),
        stderr: &mut stderr.lock(),
    })
}

/// Report a default-path warning in `format` (ADR 30.9.26j D9).
fn report_warning(message: &str, format: ErrorFormat) {
    with_process_sinks(|sinks| compile_failure::report_warning(message, format, sinks));
}

/// [`cmd_compile`]'s body: every failure is returned, never printed.
/// `Ok(code)` is a success, or a failure the front end already rendered.
fn compile_or_fail(
    file: &PathBuf,
    output: Option<&std::path::Path>,
    flags: &CompileFlags,
) -> Result<ExitCode, CompileFailure> {
    let format = flags.error_format;
    if flags.no_codegen {
        for flag in validate_no_codegen_flags(flags, output)? {
            report_warning(&format!("ignored with --no-codegen: {flag}"), format);
        }
    }

    validate_only_unit_flags(flags)?;

    // The native core is compiled first (ADR 4.10.26n D3): elaborating a
    // project resets process-wide registries the program's codegen reads.
    let native_core = compile_native_core_if_on(flags)?;

    // Use driver's elaborate_project for multi-module support
    let trace_opts = driver::TraceOptions {
        trace_types: flags.diagnostics.trace_types.clone(),
        trace_encoding: flags.diagnostics.tracing.trace_encoding.clone(),
        trace_normalization: flags.diagnostics.tracing.trace_normalization.clone(),
        trace_ctor_registration: flags.diagnostics.tracing.trace_constructor_registration,
        ..Default::default()
    };
    let elab_start = std::time::Instant::now();
    let mut project =
        driver::elaborate_project(file, flags.verbose, flags.max_errors, Some(&trace_opts))
            .map_err(CompileFailure::Frontend)?;
    let elab_elapsed = elab_start.elapsed();

    log_elaboration_summary(flags, &project, elab_elapsed);

    if flags.dump_types {
        for def in &project.defs {
            eprintln!("  {} : {}", def.name, driver::format_type(&def.ty));
        }
    }

    prepare_defs_for_codegen(&mut project, flags);

    // Handle diagnostic dump modes (early return — these are useful with or without --no-codegen)
    if let Some(ref pattern) = flags.diagnostics.dump_ir {
        diagnostics::dump_core_ir(pattern, &project.defs, &project.type_provenance);
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(ref adt_name) = flags.diagnostics.dump_encoding {
        diagnostics::dump_adt_encoding(adt_name, &project.adt_types, &project.type_provenance);
        return Ok(ExitCode::SUCCESS);
    }

    // --no-codegen: stop after Core IR generation, skip LLVM codegen + linking.
    // In JSON mode the exit code is the verdict, so the line is suppressed (D9).
    if flags.no_codegen {
        if format == ErrorFormat::Human {
            eprintln!(
                "[no-codegen] Core IR generated successfully. {} definition(s).",
                project.defs.len()
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    // The sorry census is `check`'s; JSON mode suppresses it (D9).
    if format == ErrorFormat::Human {
        warn_sorry_defs(&project.defs);
    }

    // Find main function — its E0030 is rendered as a diagnostic, in either format.
    let main_ty = match find_main_type(file, &project.defs) {
        Ok(ty) => ty,
        Err(code) => return Ok(code),
    };

    // Choose codegen strategy: per-module when multi-module, single-module otherwise
    let has_multiple_units = project.codegen_units.len() > 1;
    warn_only_unit_ignored_for_single_unit(flags, has_multiple_units);
    let codegen_start = std::time::Instant::now();
    let target = native_core::LinkTarget {
        output,
        native_core: native_core.as_ref(),
    };
    let result = if has_multiple_units {
        per_module::run_codegen_per_module(file, target, flags, &project, &main_ty)
    } else {
        run_codegen(file, target, flags, project, &main_ty)
    };
    if flags.verbose {
        let total = elab_start.elapsed();
        let codegen_link = codegen_start.elapsed();
        let elab_pct = elab_elapsed.as_secs_f64() / total.as_secs_f64() * 100.0;
        let codegen_pct = codegen_link.as_secs_f64() / total.as_secs_f64() * 100.0;
        eprintln!(
            "[perf] total={:.2}s elab={:.2}s ({:.0}%) codegen+link={:.2}s ({:.0}%)",
            total.as_secs_f64(),
            elab_elapsed.as_secs_f64(),
            elab_pct,
            codegen_link.as_secs_f64(),
            codegen_pct,
        );
    }
    result.map(|()| ExitCode::SUCCESS)
}

/// The native core's object or `.ll`, when `ALBITE_NATIVE_CORE` is `on` or
/// `all` and code is generated at all (ADR 4.10.26n D3, D6; 6.10.26n D5). Its
/// object is written at `NATIVE_CORE_OBJECT_LEVEL` whatever the program's
/// (ADR 13.10.26p D2), after the core's own pass pipeline (ADR 7.10.26p D2). The
/// program's `--alloc-profile` reaches the core's run too (D5), or every `++`
/// allocation would vanish from the profile once the core is on.
fn compile_native_core_if_on(
    flags: &CompileFlags,
) -> Result<Option<native_core::NativeCoreArtifact>, CompileFailure> {
    if !flags.native_core.links_core() || flags.no_codegen {
        return Ok(None);
    }
    let options = native_core::NativeCoreBuild {
        emit_obj: !flags.emit_llvm,
        verbose: flags.verbose,
        alloc_profile: flags.diagnostics.alloc_profile.as_deref(),
        switch: flags.native_core,
    };
    native_core::compile_native_core(&options).map(Some)
}

/// Gate for `--only-unit` (ADR 3.7.26b): isolation compiles a subset of
/// codegen units; the resulting partial object set cannot link, so
/// --emit-llvm is required (--no-codegen warns the flag away instead).
pub(super) fn validate_only_unit_flags(flags: &CompileFlags) -> Result<(), CompileFailure> {
    if !flags.diagnostics.only_units.is_empty() && !flags.emit_llvm && !flags.no_codegen {
        return Err(CompileFailure::only_unit_without_emit_llvm());
    }
    Ok(())
}

/// `--only-unit` only has meaning on the per-module codegen path; a
/// single-unit program silently taking the single-module path would make
/// the flag a no-op, so say so (ADR 3.7.26b).
fn warn_only_unit_ignored_for_single_unit(flags: &CompileFlags, has_multiple_units: bool) {
    if !flags.diagnostics.only_units.is_empty() && !has_multiple_units {
        report_warning(
            "--only-unit ignored: program has a single codegen unit",
            flags.error_format,
        );
    }
}

/// Verbose post-elaboration summary (definition/record/ADT counts + timing).
fn log_elaboration_summary(
    flags: &CompileFlags,
    project: &driver::ProjectOutput,
    elab_elapsed: std::time::Duration,
) {
    if flags.verbose {
        eprintln!("[perf] elab={:.2}s", elab_elapsed.as_secs_f64());
        eprintln!("Elaborated {} definition(s)", project.defs.len());
        if !project.record_types.is_empty() {
            eprintln!("Found {} record type(s)", project.record_types.len());
        }
        if !project.adt_types.is_empty() {
            eprintln!("Found {} ADT type(s)", project.adt_types.len());
        }
    }
}

/// Apply compile flags to a `CodeGen` instance.
fn configure_codegen(
    codegen: &mut albite_codegen::CodeGen<'_>,
    file: &PathBuf,
    flags: &CompileFlags,
) {
    if flags.debug_info {
        let source_text = std::fs::read_to_string(file).unwrap_or_default();
        let source_path = file.to_string_lossy();
        codegen.enable_debug_info(&source_path, &source_text);
        if flags.verbose {
            eprintln!("Debug info enabled (definition-level DWARF)");
        }
    }
    if flags.diagnostics.codegen_backtrace {
        codegen.set_codegen_backtrace(true);
    }
    if let Some(ref filter) = flags.diagnostics.tracing.trace_adt_ops {
        codegen.set_trace_adt_ops(filter.clone());
    }
    if flags.diagnostics.tracing.trace_musttail {
        codegen.set_trace_musttail();
    }
    if flags.diagnostics.tracing.trace_escape {
        codegen.set_trace_escape();
    }
    if let Some(ref filter) = flags.diagnostics.dump_synthesized {
        codegen.set_dump_synthesized(filter.clone());
    }
    if flags.named_lambdas {
        codegen.set_named_lambdas(true);
        if flags.verbose {
            eprintln!("Named lambdas enabled (source-level IR names)");
        }
    }
    codegen.set_native_core_switch(flags.native_core);
    if let Some(ref filter) = flags.diagnostics.alloc_profile {
        codegen.set_alloc_profile(native_core::alloc_profile_filter(filter));
        if flags.verbose {
            eprintln!("Allocation profiling enabled");
        }
    }
}

/// Initialize LLVM codegen, compile definitions, and emit output.
fn run_codegen(
    file: &PathBuf,
    target: native_core::LinkTarget<'_>,
    flags: &CompileFlags,
    project: driver::ProjectOutput,
    main_ty: &albite_core::Type,
) -> Result<(), CompileFailure> {
    use albite_codegen::inkwell::context::Context as LlvmContext;
    use albite_codegen::CodeGen;

    // Initialize codegen
    let llvm_context = LlvmContext::create();
    let module_name = file.file_stem().unwrap_or_default().to_string_lossy();
    let mut codegen = CodeGen::new(&llvm_context, &module_name);

    configure_codegen(&mut codegen, file, flags);
    codegen.set_regions_used(project.source_map.uses_regions());

    codegen.register_record_types(project.record_types.clone());
    // Comparator synthesis resolves generic instantiations through the same ADT
    // definitions codegen lowers (ADR 1.8.26c), so keep a copy before the
    // codegen-shaped conversion consumes them.
    let adt_types_for_comparator = project.adt_types.clone();
    codegen.register_adt_types(convert_adt_types_for_codegen(project.adt_types));
    // Install the lazy comparator-synthesis callback (ADR 29.6.26f P6′ step 2).
    codegen.set_comparator_synth(albite_bootstrap::comparator::codegen_hook::codegen_synth(
        albite_bootstrap::comparator::ComparatorTypes::new(
            project.record_types,
            &project.encoded_types,
            &project.type_provenance,
            adt_types_for_comparator,
            &project.mutual_recursion_groups,
        ),
    ));

    // Pass 1: Declare all functions and register definitions
    let codegen_failed =
        |message: String| CompileFailure::build(BuildErrorKind::CodeGenerationFailed, message);
    let extern_name_map =
        declare_and_register_defs(&mut codegen, &project.defs).map_err(codegen_failed)?;

    // Pass 2: Compile all definitions
    compile_all_defs(&mut codegen, &project.defs, &extern_name_map, flags.verbose)
        .map_err(codegen_failed)?;

    // Create main wrapper
    if flags.verbose {
        eprintln!("Creating main wrapper...");
    }
    codegen
        .compile_main_wrapper(main_ty)
        .map_err(|e| codegen_failed(format!("could not create main wrapper: {e}")))?;

    // Finalize debug info before any code emission
    codegen.finalize_debug_info();

    // Emit output (LLVM IR or native executable)
    linking::emit_output(&codegen, file, target, flags)
}

// Re-export extern naming utilities for use by submodules.

/// Post-elaboration preparation: the one codegen erasure, then validation
/// and type-variable substitution over the erased definitions.
fn prepare_defs_for_codegen(project: &mut driver::ProjectOutput, flags: &CompileFlags) {
    // Downstream of the IR cache, upstream of validation, TCO, mono and
    // codegen, none of which reads a `Pi` (ADR 30.9.26d D6).
    erase_dependent_types(project);
    validation::validate_tyvar_escapes(&project.defs, flags.diagnostics.check_tyvar_escape);
    validation::apply_tyvar_substitutions(
        &mut project.defs,
        &project.type_provenance,
        &project.adt_types,
        flags.verbose,
    );
}

/// Rewrite every `Pi` in the project's definitions — flat and per unit — to
/// the `Arrow` it erases to (ADR 30.9.26d D6), and erase every proof over a
/// proposition (ADR 3.10.26e D4).
fn erase_dependent_types(project: &mut driver::ProjectOutput) {
    for def in &mut project.defs {
        *def = def.erase_for_codegen();
    }
    for unit in &mut project.codegen_units {
        for def in &mut unit.defs {
            *def = def.erase_for_codegen();
        }
    }
}
