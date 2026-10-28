//! ADR 30.9.26j AC1: one failure value, today's human text, one record.
//!
//! Every expected human line below is copied from the pre-change `eprintln!`
//! it replaces, so a drift in the value's rendering fails here before it
//! reaches a golden.

use super::*;
use crate::driver::diagnostics::json::KIND_DIAGNOSTIC;

fn lines(text: &[&str]) -> Vec<String> {
    text.iter().map(|s| s.to_string()).collect()
}

fn link_failure() -> CompileFailure {
    CompileFailure::build(
        BuildErrorKind::ToolFailed,
        "linker failed with status exit status: 1",
    )
}

/// One value per kind, built the way its site builds it, beside the exact
/// text that site printed before (30.9.26j AC1).
fn every_kind_with_todays_text() -> Vec<(CompileFailure, Vec<String>)> {
    vec![
        (
            CompileFailure::incompatible_no_codegen_and_emit_llvm(),
            lines(&[
                "error: --no-codegen and --emit-llvm are incompatible.",
                "       --no-codegen stops before LLVM IR emission.",
                "       Use --no-codegen alone, or --emit-llvm alone.",
            ]),
        ),
        (
            CompileFailure::only_unit_without_emit_llvm(),
            lines(&[
                "error: --only-unit requires --emit-llvm.",
                "       Unit isolation emits .ll files for the selected unit(s) only;",
                "       a partial object set cannot be linked into a binary.",
            ]),
        ),
        (
            CompileFailure::build(
                BuildErrorKind::CodeGenerationFailed,
                "could not create main wrapper: boom",
            ),
            lines(&["error: could not create main wrapper: boom"]),
        ),
        (
            CompileFailure::build(
                BuildErrorKind::ToolUnavailable,
                "could not run linker: No such file or directory (os error 2)",
            )
            .with_help("make sure 'cc' (C compiler) is installed and in PATH"),
            lines(&[
                "error: could not run linker: No such file or directory (os error 2)",
                "help: make sure 'cc' (C compiler) is installed and in PATH",
            ]),
        ),
        (
            link_failure(),
            lines(&["error: linker failed with status exit status: 1"]),
        ),
        (
            CompileFailure::preformatted(
                BuildErrorKind::RuntimeLibraryUnusable,
                "error: libalbite_core.a not found at /x\nhelp:     cargo build -p albite_core",
            ),
            lines(&[
                "error: libalbite_core.a not found at /x",
                "help:     cargo build -p albite_core",
            ]),
        ),
        (
            CompileFailure::build(
                BuildErrorKind::OutputNotWritable,
                "could not write 'missing/out.ll': No such file or directory (os error 2)",
            )
            .with_exit(3),
            lines(&[
                "error: could not write 'missing/out.ll': No such file or directory (os error 2)",
            ]),
        ),
    ]
}

#[test]
fn every_kind_renders_todays_human_text() {
    for (failure, expected) in every_kind_with_todays_text() {
        assert_eq!(failure.human_lines(), expected);
        assert_eq!(
            failure.output(ErrorFormat::Human),
            FailureOutput::Lines(expected)
        );
    }
}

#[test]
fn every_kind_has_a_distinct_code_inside_the_build_block() {
    let codes: Vec<&str> = every_kind_with_todays_text()
        .iter()
        .map(|(failure, _)| failure.code().expect("a build failure has a code"))
        .collect();
    assert_eq!(
        codes,
        ["E0140", "E0141", "E0142", "E0143", "E0144", "E0145", "E0146"]
    );
}

/// The exit code is the value's, not the kind's (30.9.26j D3).
#[test]
fn output_not_writable_exits_by_site() {
    let ir_write = CompileFailure::build(BuildErrorKind::OutputNotWritable, "x").with_exit(3);
    let copy = CompileFailure::build(BuildErrorKind::OutputNotWritable, "could not copy 'a': e");
    assert_eq!(ir_write.exit_code(), 3);
    assert_eq!(copy.exit_code(), 1);
    assert_eq!(ir_write.code(), copy.code());
}

#[test]
fn a_build_record_carries_code_notes_and_help_and_no_span() {
    let record = CompileFailure::incompatible_no_codegen_and_emit_llvm()
        .with_help("a help line")
        .record()
        .expect("a build failure has a record");
    assert_eq!(record.schema, 1);
    assert_eq!(record.kind, KIND_DIAGNOSTIC);
    assert_eq!(record.level, Level::Error);
    assert_eq!(record.code.as_deref(), Some("E0140"));
    assert_eq!(
        record.message,
        "--no-codegen and --emit-llvm are incompatible."
    );
    assert_eq!(record.primary_span, None);
    let notes: Vec<_> = record.notes.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(
        notes,
        [
            "--no-codegen stops before LLVM IR emission.",
            "Use --no-codegen alone, or --emit-llvm alone."
        ]
    );
    assert!(record.notes.iter().all(|n| n.span.is_none()));
    assert_eq!(record.help, ["a help line"]);
    assert!(record.diagnostic_hints.is_empty());
}

/// A pre-formatted diagnosis round-trips byte for byte, a trailing space
/// included (30.9.26j AC1).
#[test]
fn a_runtime_library_diagnosis_prints_verbatim() {
    let diagnosis = "error: lib.a is a GNU archive — built for the other platform\n\
                     help: a build \n\
                     help:     cargo build --release -p albite_core";
    let failure = CompileFailure::preformatted(BuildErrorKind::RuntimeLibraryUnusable, diagnosis);
    assert_eq!(failure.human_lines().join("\n"), diagnosis);
    assert_eq!(failure.code(), Some("E0145"));
    let record = failure.record().unwrap();
    assert_eq!(
        record.message,
        "lib.a is a GNU archive — built for the other platform"
    );
    assert_eq!(
        record.help,
        ["a build ", "    cargo build --release -p albite_core"]
    );
}

#[test]
fn a_preformatted_continuation_line_keeps_its_indent() {
    let text = "error: x\n       more\nhelp: h";
    let failure = CompileFailure::preformatted(BuildErrorKind::ToolFailed, text);
    assert_eq!(failure.human_lines().join("\n"), text);
}

/// A failed child's stderr then stdout follow the continuation lines in the
/// record, and never reach the human text (30.9.26j D6).
#[test]
fn child_stderr_then_stdout_are_appended_to_the_notes() {
    let failure = link_failure().with_note("a continuation").with_tool_output(
        "ld: open() failed\nclang: error: linker command failed\n",
        "out line\n",
    );
    let record = failure.record().unwrap();
    let notes: Vec<_> = record.notes.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(
        notes,
        [
            "a continuation",
            "ld: open() failed",
            "clang: error: linker command failed",
            "out line"
        ]
    );
    assert_eq!(
        failure.human_lines(),
        lines(&[
            "error: linker failed with status exit status: 1",
            "       a continuation"
        ])
    );
}

#[test]
fn json_output_is_the_record() {
    let failure = link_failure();
    assert_eq!(
        failure.output(ErrorFormat::Json),
        FailureOutput::Record(failure.record().unwrap())
    );
}

#[test]
fn a_frontend_elab_failure_writes_no_record() {
    let failure = CompileFailure::Frontend(PipelineError::ElabFailed("elaboration errors".into()));
    assert_eq!(failure.record(), None);
    assert_eq!(failure.output(ErrorFormat::Json), FailureOutput::Silent);
    assert_eq!(
        failure.human_lines(),
        lines(&["error: compilation failed: elaboration errors"])
    );
    assert_eq!(failure.code(), None);
    assert_eq!(failure.exit_code(), 1);
}

#[test]
fn any_other_frontend_failure_is_a_code_null_record() {
    let failure = CompileFailure::Frontend(PipelineError::IoError(
        "./nope.tg".into(),
        "No such file or directory (os error 2)".into(),
    ));
    let text = "could not read './nope.tg': No such file or directory (os error 2)";
    assert_eq!(failure.record(), Some(json::failure_record(text)));
    assert_eq!(failure.human_lines(), [format!("error: {text}")]);
    assert_eq!(failure.exit_code(), 1);
}

/// The builders only shape a `Build`; on a front-end failure they are no-ops.
#[test]
fn builders_leave_a_frontend_failure_alone() {
    let failure = CompileFailure::Frontend(PipelineError::ElabFailed("parse errors".into()))
        .with_note("n")
        .with_help("h")
        .with_exit(3)
        .with_tool_output("e", "o");
    assert_eq!(failure.exit_code(), 1);
    assert_eq!(
        failure.human_lines(),
        lines(&["error: compilation failed: parse errors"])
    );
}

// 20.10.26i AC2: a plan note is a `note:` line, or a `note`-level record.
#[test]
fn a_note_is_a_line_or_an_uncoded_note_record() {
    let message = "`/p/tungsten.toml` uses the old manifest name";
    assert_eq!(
        note_output(message, ErrorFormat::Human),
        FailureOutput::Lines(lines(&[
            "note: `/p/tungsten.toml` uses the old manifest name"
        ]))
    );
    let FailureOutput::Record(record) = note_output(message, ErrorFormat::Json) else {
        panic!("JSON mode writes a record");
    };
    assert_eq!(record.level, Level::Note);
    assert_eq!(record.code, None);
    assert_eq!(record.message, message);
    let (stdout, stderr) = captured(|sinks| report_note(message, ErrorFormat::Human, sinks));
    assert_eq!(
        (stdout.as_str(), stderr.as_str()),
        ("", "note: `/p/tungsten.toml` uses the old manifest name\n")
    );
}

#[test]
fn a_warning_is_a_line_or_an_uncoded_warning_record() {
    let message = "ignored with --no-codegen: --debug-info";
    assert_eq!(
        warning_output(message, ErrorFormat::Human),
        FailureOutput::Lines(lines(&["warning: ignored with --no-codegen: --debug-info"]))
    );
    let FailureOutput::Record(record) = warning_output(message, ErrorFormat::Json) else {
        panic!("JSON mode writes a record");
    };
    assert_eq!(record.level, Level::Warning);
    assert_eq!(record.code, None);
    assert_eq!(record.message, message);
    assert_eq!(record.primary_span, None);
}

#[test]
fn child_output_is_captured_only_for_json() {
    assert!(capture_child_output(ErrorFormat::Json));
    assert!(!capture_child_output(ErrorFormat::Human));
}

/// Run `write` against two buffers, returning `(stdout, stderr)`.
fn captured(write: impl FnOnce(&mut Sinks<'_>)) -> (String, String) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    write(&mut Sinks {
        stdout: &mut stdout,
        stderr: &mut stderr,
    });
    (
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

// 30.9.26j D3: human mode writes the lines to stderr and returns the site's exit.
#[test]
fn report_writes_human_lines_to_stderr_and_returns_the_exit_code() {
    let failure = CompileFailure::build(BuildErrorKind::OutputNotWritable, "x").with_exit(3);
    let mut exit = 0;
    let (stdout, stderr) = captured(|sinks| exit = report(&failure, ErrorFormat::Human, sinks));
    assert_eq!(exit, 3);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "error: x\n");
}

// 30.9.26j D5: JSON mode writes one record line to the selected record stream.
#[test]
fn report_writes_one_record_to_the_record_stream() {
    crate::driver::diagnostics::set_record_stream(crate::driver::diagnostics::RecordStream::Stderr);
    let failure = CompileFailure::incompatible_no_codegen_and_emit_llvm();
    let mut exit = 0;
    let (stdout, stderr) = captured(|sinks| exit = report(&failure, ErrorFormat::Json, sinks));
    assert_eq!(exit, 1);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        format!("{}\n", json::to_line(&failure.record().unwrap()))
    );
}

#[test]
fn a_silent_failure_writes_nothing() {
    let failure = CompileFailure::Frontend(PipelineError::ElabFailed("parse errors".into()));
    let (stdout, stderr) = captured(|sinks| {
        report(&failure, ErrorFormat::Json, sinks);
    });
    assert_eq!((stdout.as_str(), stderr.as_str()), ("", ""));
}

// 30.9.26j D9: a warning is one stderr line in human mode.
#[test]
fn report_warning_writes_the_warning_line() {
    let (stdout, stderr) =
        captured(|sinks| report_warning("--only-unit ignored", ErrorFormat::Human, sinks));
    assert_eq!(stdout, "");
    assert_eq!(stderr, "warning: --only-unit ignored\n");
}
