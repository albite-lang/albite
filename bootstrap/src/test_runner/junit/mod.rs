//! `JUnit` XML for `albite test --format junit` (ADR 30.9.26k).
//!
//! Rendered from the same [`TestRecord`]s the JSON arm streams (D2), so the
//! two machine formats cannot disagree on a verdict. One document per run:
//! one `<testsuites>`, one `<testsuite>`, one `<testcase>` per record (D3).
//! Everything here is pure; `run::run_junit` and `stop` own the `println!`.
//!
//! `JUnit` sits beside schema 1, not inside it (D8): its shape is pinned by the
//! `tests/golden/test/junit_*` snapshots.

mod escape;

use std::fmt::Write as _;
use std::path::Path;

use super::json::TestRecord;
use escape::{escape_attr, escape_text};

/// What the `<testsuite>` element is named after, and how long the run took.
pub(super) struct SuiteInfo {
    /// The entry file as the user spelled it.
    pub name: String,
    /// The entry file's stem — the entry module's `classname` (D4).
    pub entry_stem: String,
    /// The run's elapsed wall time, not a sum of the testcases'.
    pub duration_ms: u64,
}

impl SuiteInfo {
    /// The suite for a run of `entry_file` that took `duration_ms`.
    pub(super) fn for_entry(entry_file: &str, duration_ms: u64) -> Self {
        let entry_stem = Path::new(entry_file).file_stem().map_or_else(
            || entry_file.to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        Self {
            name: entry_file.to_string(),
            entry_stem,
            duration_ms,
        }
    }
}

/// Why a run stopped before it could report every test (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopKind {
    /// The runner could not produce verdicts (exit 2).
    Runner,
    /// The source did not elaborate (exit 1).
    Diagnostics,
}

impl StopKind {
    /// The synthetic testcase's `name` and its `<error>`'s `type`.
    fn testcase_name_and_type(self) -> (&'static str, &'static str) {
        match self {
            Self::Runner => ("(runner)", "runner_error"),
            Self::Diagnostics => ("(elaboration)", "diagnostics"),
        }
    }
}

/// A finished run's document.
pub(super) fn document(records: &[TestRecord], suite: &SuiteInfo) -> String {
    render(records, None, suite.duration_ms, suite)
}

/// A stopped run's document: the `records` collected before the stop, then
/// one synthetic `<error>` testcase carrying `message`. Its root and suite
/// `time` are 0 (D3), whatever `suite.duration_ms` says.
pub(super) fn stop_document(
    records: &[TestRecord],
    kind: StopKind,
    message: &str,
    suite: &SuiteInfo,
) -> String {
    render(records, Some((kind, message)), 0, suite)
}

/// `classname`: the module path dotted, or the entry file's stem for the
/// entry module (D4). Dashboards split it on `.`, so never a file path.
pub(super) fn classname(record: &TestRecord, entry_stem: &str) -> String {
    match &record.module {
        Some(module) => module.replace("::", "."),
        None => entry_stem.to_string(),
    }
}

/// Whole milliseconds as seconds with exactly three decimals (D9).
pub(super) fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// The `skipped` count (D3): a skipped test, and an expected failure — it ran,
/// but it does not gate.
fn counts_as_skipped(record: &TestRecord) -> bool {
    record.verdict == "skipped" || record.expected_failure.is_some()
}

fn render(
    records: &[TestRecord],
    stop: Option<(StopKind, &str)>,
    duration_ms: u64,
    suite: &SuiteInfo,
) -> String {
    let errors = usize::from(stop.is_some());
    let counts = format!(
        "tests=\"{}\" failures=\"{}\" errors=\"{errors}\" skipped=\"{}\" time=\"{}\"",
        records.len() + errors,
        records.iter().filter(|r| r.gating).count(),
        records.iter().filter(|r| counts_as_skipped(r)).count(),
        seconds(duration_ms),
    );
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(out, "<testsuites name=\"albite test\" {counts}>");
    let _ = writeln!(
        out,
        "  <testsuite name=\"{}\" {counts}>",
        escape_attr(&suite.name)
    );
    for record in records {
        out.push_str(&testcase(record, suite));
    }
    if let Some((kind, message)) = stop {
        out.push_str(&stop_testcase(kind, message, suite));
    }
    out.push_str("  </testsuite>\n</testsuites>");
    out
}

/// The opening of a testcase: D5's fixed attribute set, in order.
fn testcase_open(name: &str, classname: &str, file: &str, assertions: u64, ms: u64) -> String {
    format!(
        "    <testcase name=\"{}\" classname=\"{}\" file=\"{}\" assertions=\"{assertions}\" time=\"{}\"",
        escape_attr(name),
        escape_attr(classname),
        escape_attr(file),
        seconds(ms),
    )
}

/// One record's testcase (D5): the child the verdict maps to, then its
/// captured output — stderr only when no `<failure>` already carries it.
fn testcase(record: &TestRecord, suite: &SuiteInfo) -> String {
    let mut children = String::new();
    if record.gating {
        let message = record.message.as_deref().unwrap_or(record.verdict);
        let _ = writeln!(
            children,
            "      <failure message=\"{}\" type=\"{}\">{}</failure>",
            escape_attr(message),
            escape_attr(record.verdict),
            escape_text(&record.stderr)
        );
    } else if let Some(expected) = &record.expected_failure {
        let message = format!(
            "expected failure: {} (owned by {})",
            expected.reported, expected.owner
        );
        let _ = writeln!(
            children,
            "      <skipped message=\"{}\"/>",
            escape_attr(&message)
        );
    } else if record.verdict == "skipped" {
        let message = record.message.as_deref().unwrap_or(record.verdict);
        let _ = writeln!(
            children,
            "      <skipped message=\"{}\"/>",
            escape_attr(message)
        );
    }
    if !record.stdout.is_empty() {
        let _ = writeln!(
            children,
            "      <system-out>{}</system-out>",
            escape_text(&record.stdout)
        );
    }
    if !record.stderr.is_empty() && !record.gating {
        let _ = writeln!(
            children,
            "      <system-err>{}</system-err>",
            escape_text(&record.stderr)
        );
    }
    let open = testcase_open(
        &record.name,
        &classname(record, &suite.entry_stem),
        &record.file,
        record.assertions,
        record.duration_ms,
    );
    close_testcase(&open, &children)
}

/// The synthetic testcase a stopped run ends with (D6).
fn stop_testcase(kind: StopKind, message: &str, suite: &SuiteInfo) -> String {
    let (name, error_type) = kind.testcase_name_and_type();
    let open = testcase_open(name, &suite.entry_stem, &suite.name, 0, 0);
    let child = format!(
        "      <error message=\"{}\" type=\"{error_type}\"/>\n",
        escape_attr(message)
    );
    close_testcase(&open, &child)
}

fn close_testcase(open: &str, children: &str) -> String {
    if children.is_empty() {
        format!("{open}/>\n")
    } else {
        format!("{open}>\n{children}    </testcase>\n")
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
// Tests: tests_escape.rs — D7's escaping on hostile input
#[path = "tests_escape.rs"]
mod tests_escape;
