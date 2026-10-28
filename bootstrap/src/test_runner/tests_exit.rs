//! The exit contract (ADR 29.9.26i AC 1): `TestExit`'s numbers, the runner
//! error's two renderings, and the exit class of each early return that is
//! decided by a pure function. Split from `tests.rs` for the file-size cap.
//!
//! The early returns that are one `runner_error` call — the manifest error,
//! the pipeline `Err`, an unexpected result, `--require-tests` over zero — are
//! pinned end to end through the binary in `bootstrap/tests/test_format_json.rs`.

use std::path::PathBuf;
use std::process::ExitCode;

use super::scope::{scope_decision, ModuleScopeResult, ScopeStop};
use super::stop::{
    diagnostics_stop, diagnostics_stop_text, empty_run, empty_run_text, runner_error,
    runner_error_text, DIAGNOSTICS_STOP_MESSAGE,
};
use super::{TestExit, TestFormatArg};

// 29.9.26i AC1: the three exits are 0, 1 and 2, and nothing else.
#[test]
fn test_exit_codes_are_zero_one_two() {
    assert_eq!(TestExit::Passed.code(), 0);
    assert_eq!(TestExit::Failed.code(), 1);
    assert_eq!(TestExit::RunnerError.code(), 2);
}

#[test]
fn test_exit_converts_to_the_process_exit_code() {
    assert_eq!(ExitCode::from(TestExit::Passed), ExitCode::from(0));
    assert_eq!(ExitCode::from(TestExit::Failed), ExitCode::from(1));
    assert_eq!(ExitCode::from(TestExit::RunnerError), ExitCode::from(2));
}

#[test]
fn a_gating_tally_is_failed_and_a_clean_one_passed() {
    assert_eq!(TestExit::from_failing(true), TestExit::Failed);
    assert_eq!(TestExit::from_failing(false), TestExit::Passed);
}

#[test]
fn a_human_runner_error_is_an_error_line_on_stderr() {
    let (text, to_stdout) = runner_error_text("cannot read f.tg", TestFormatArg::Human, "f.tg");
    assert_eq!(text, "error: cannot read f.tg");
    assert!(!to_stdout);
}

/// D7: a JSON runner error is exactly one location-less diagnostic record.
#[test]
fn a_json_runner_error_is_one_codeless_record_on_stdout() {
    let (text, to_stdout) = runner_error_text("cannot read f.tg", TestFormatArg::Json, "f.tg");
    assert!(to_stdout);
    assert!(!text.contains('\n'));
    let record: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(record["schema"], 1);
    assert_eq!(record["kind"], "diagnostic");
    assert_eq!(record["message"], "cannot read f.tg");
    assert!(record["code"].is_null());
    assert!(record["primary_span"].is_null());
}

#[test]
fn runner_error_always_returns_the_runner_error_exit() {
    assert_eq!(
        runner_error("x", TestFormatArg::Human, "f.tg"),
        TestExit::RunnerError
    );
    assert_eq!(
        runner_error("x", TestFormatArg::Json, "f.tg"),
        TestExit::RunnerError
    );
}

#[test]
fn runner_error_in_junit_returns_the_runner_error_exit() {
    assert_eq!(
        runner_error("x", TestFormatArg::Junit, "f.tg"),
        TestExit::RunnerError
    );
}

// ── JUnit's early exits (ADR 30.9.26k D6) ───────────────────────────────────

/// The one testcase a stop document holds, as `[name, classname, error type,
/// error message]`, after asserting the document parses and counts one error.
fn only_stop_testcase(xml: &str) -> [String; 4] {
    let doc = roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{e}:\n{xml}"));
    assert_eq!(doc.root_element().attribute("errors"), Some("1"));
    assert_eq!(doc.root_element().attribute("tests"), Some("1"));
    let cases: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("testcase"))
        .collect();
    assert_eq!(cases.len(), 1);
    let error = cases[0]
        .children()
        .find(|n| n.has_tag_name("error"))
        .unwrap();
    let attr = |node: roxmltree::Node<'_, '_>, name| node.attribute(name).unwrap().to_string();
    [
        attr(cases[0], "name"),
        attr(cases[0], "classname"),
        attr(error, "type"),
        attr(error, "message"),
    ]
}

// 30.9.26k AC3: a JUnit runner error is one `(runner)` document on stdout.
#[test]
fn a_junit_runner_error_is_one_runner_testcase_on_stdout() {
    let (text, to_stdout) = runner_error_text("cannot read f.tg", TestFormatArg::Junit, "d/f.tg");
    assert!(to_stdout);
    assert_eq!(
        only_stop_testcase(&text),
        ["(runner)", "f", "runner_error", "cannot read f.tg"].map(String::from)
    );
}

// 30.9.26k AC3: a JUnit diagnostics stop is one `(elaboration)` document;
// the other formats add nothing to stdout.
#[test]
fn a_junit_diagnostics_stop_is_one_elaboration_testcase() {
    let text = diagnostics_stop_text(TestFormatArg::Junit, "d/f.tg").unwrap();
    assert_eq!(
        only_stop_testcase(&text),
        [
            "(elaboration)",
            "f",
            "diagnostics",
            DIAGNOSTICS_STOP_MESSAGE
        ]
        .map(String::from)
    );
    assert_eq!(diagnostics_stop_text(TestFormatArg::Human, "f.tg"), None);
    assert_eq!(diagnostics_stop_text(TestFormatArg::Json, "f.tg"), None);
}

#[test]
fn a_diagnostics_stop_is_failed_in_every_format() {
    for format in [
        TestFormatArg::Human,
        TestFormatArg::Json,
        TestFormatArg::Junit,
    ] {
        assert_eq!(diagnostics_stop(format, "f.tg"), TestExit::Failed);
    }
}

// 30.9.26k AC3: an empty JUnit run is a `tests="0"` document.
#[test]
fn an_empty_run_renders_per_format() {
    let junit = empty_run_text(TestFormatArg::Junit, true, "d/f.tg").unwrap();
    let doc = roxmltree::Document::parse(&junit).unwrap();
    assert_eq!(doc.root_element().attribute("tests"), Some("0"));
    assert_eq!(
        doc.descendants()
            .filter(|n| n.has_tag_name("testcase"))
            .count(),
        0
    );
    assert_eq!(
        empty_run_text(TestFormatArg::Human, true, "f.tg").as_deref(),
        Some("no tests found")
    );
    assert_eq!(empty_run_text(TestFormatArg::Human, false, "f.tg"), None);
    let json = empty_run_text(TestFormatArg::Json, false, "f.tg").unwrap();
    let record: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(record["kind"], "test_summary");
    assert_eq!(record["total"], 0);
}

#[test]
fn an_empty_run_passes_in_every_format() {
    for format in [
        TestFormatArg::Human,
        TestFormatArg::Json,
        TestFormatArg::Junit,
    ] {
        assert_eq!(empty_run(format, false, "f.tg"), TestExit::Passed);
    }
}

// ── --module's exit class (D2) ──────────────────────────────────────────────

#[test]
fn a_matched_module_runs_its_defs() {
    let decided = scope_decision(ModuleScopeResult::Matched(Vec::new()), "m.tg", true);
    assert!(matches!(decided, Ok(defs) if defs.is_empty()));
}

#[test]
fn an_unmatched_module_under_require_tests_is_a_runner_error() {
    let decided = scope_decision(ModuleScopeResult::NoMatch, "m.tg", true);
    assert_eq!(
        decided.unwrap_err(),
        ScopeStop::RunnerError(
            "no module matching 'm.tg' found and --require-tests is set".to_string()
        )
    );
}

#[test]
fn an_unmatched_module_without_require_tests_is_an_empty_passing_run() {
    let decided = scope_decision(ModuleScopeResult::NoMatch, "m.tg", false);
    assert_eq!(
        decided.unwrap_err(),
        ScopeStop::EmptyRun("warning: no module matching 'm.tg' found; 0 tests run".to_string())
    );
}

#[test]
fn an_ambiguous_module_is_a_runner_error_naming_every_match() {
    let paths = vec![PathBuf::from("a/m.tg"), PathBuf::from("b/m.tg")];
    for require_tests in [true, false] {
        let decided = scope_decision(
            ModuleScopeResult::Ambiguous(paths.clone()),
            "m.tg",
            require_tests,
        );
        assert_eq!(
            decided.unwrap_err(),
            ScopeStop::RunnerError(
                "module path 'm.tg' is ambiguous, matched:\n  a/m.tg\n  b/m.tg".to_string()
            )
        );
    }
}
