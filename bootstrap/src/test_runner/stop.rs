//! The early exits of `albite test`, in each `--format` (ADR 29.9.26i D7,
//! 30.9.26k D6). What each format prints is decided by a pure `*_text`
//! renderer; the printing wrappers stay a few lines each.
//!
//! In `JUnit` mode every exit writes exactly one well-formed document, so a CI
//! step that uploads the report unconditionally always has one to parse.

use crate::cli::TestFormatArg;
use albite_bootstrap::driver::diagnostics::json as diagnostic_json;

use super::json;
use super::junit::{self, StopKind, SuiteInfo};
use super::TestExit;

/// The `(elaboration)` testcase's message: the diagnostics themselves stay
/// human, on stderr (D6).
pub(super) const DIAGNOSTICS_STOP_MESSAGE: &str =
    "the file did not elaborate; diagnostics are on stderr";

/// What a runner error prints, and whether on stdout: `error: …` on stderr in
/// human mode, one location-less record in JSON mode (29.9.26i D7), one
/// `(runner)` stop document in `JUnit` mode.
pub(super) fn runner_error_text(
    message: &str,
    format: TestFormatArg,
    entry_file: &str,
) -> (String, bool) {
    match format {
        TestFormatArg::Human => (format!("error: {message}"), false),
        TestFormatArg::Json => (
            diagnostic_json::to_line(&diagnostic_json::failure_record(message)),
            true,
        ),
        TestFormatArg::Junit => (
            junit::stop_document(
                &[],
                StopKind::Runner,
                message,
                &SuiteInfo::for_entry(entry_file, 0),
            ),
            true,
        ),
    }
}

/// Report a runner error in `format` and return [`TestExit::RunnerError`].
pub(super) fn runner_error(message: &str, format: TestFormatArg, entry_file: &str) -> TestExit {
    let (text, to_stdout) = runner_error_text(message, format, entry_file);
    if to_stdout {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
    TestExit::RunnerError
}

/// What a passing run with zero tests prints on stdout: `no tests found` in
/// human mode (when `human_line` says so), a zero `test_summary` in JSON
/// mode, a `tests="0"` document in `JUnit` mode.
pub(super) fn empty_run_text(
    format: TestFormatArg,
    human_line: bool,
    entry_file: &str,
) -> Option<String> {
    match format {
        TestFormatArg::Json => Some(json::empty_summary_line()),
        TestFormatArg::Junit => Some(junit::document(&[], &SuiteInfo::for_entry(entry_file, 0))),
        TestFormatArg::Human if human_line => Some("no tests found".to_string()),
        TestFormatArg::Human => None,
    }
}

/// Report a passing run with zero tests and return [`TestExit::Passed`].
pub(super) fn empty_run(format: TestFormatArg, human_line: bool, entry_file: &str) -> TestExit {
    if let Some(text) = empty_run_text(format, human_line, entry_file) {
        println!("{text}");
    }
    TestExit::Passed
}

/// What a diagnostics stop adds on stdout. Human and JSON modes add nothing:
/// the diagnostics are already on their stream. `JUnit` mode adds one
/// `(elaboration)` stop document.
pub(super) fn diagnostics_stop_text(format: TestFormatArg, entry_file: &str) -> Option<String> {
    match format {
        TestFormatArg::Human | TestFormatArg::Json => None,
        TestFormatArg::Junit => Some(junit::stop_document(
            &[],
            StopKind::Diagnostics,
            DIAGNOSTICS_STOP_MESSAGE,
            &SuiteInfo::for_entry(entry_file, 0),
        )),
    }
}

/// Report a source that did not elaborate and return [`TestExit::Failed`]:
/// diagnostics are 1, not 2 (29.9.26i D3).
pub(super) fn diagnostics_stop(format: TestFormatArg, entry_file: &str) -> TestExit {
    if let Some(text) = diagnostics_stop_text(format, entry_file) {
        println!("{text}");
    }
    TestExit::Failed
}
