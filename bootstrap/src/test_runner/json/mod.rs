//! Schema-1 JSON records for `albite test --format json` (ADR 29.9.26i).
//!
//! One `"kind": "test"` record per test, in run order, then one
//! `"kind": "test_summary"`. Both share the envelope, `SCHEMA_VERSION` and the
//! kinds registry that `driver::diagnostics::json` owns (D6), so a consumer
//! reads `check`'s and `test`'s streams with one parser.
//!
//! Everything here is pure: the builders take an already-decided
//! [`TestOutcome`] and the facts the runner measured around it, and return a
//! value. `run::run_and_report`'s JSON arm owns the only `println!`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use albite_bootstrap::driver::diagnostics::json::{
    to_line, KIND_TEST, KIND_TEST_SUMMARY, SCHEMA_VERSION,
};
use albite_bootstrap::elaborate::CoreDef;
use serde::Serialize;

use super::run::Tally;
use super::summary::format_step_count;
use super::TestOutcome;

/// `expected_failure`'s nested object: the ADR that owns the manifest entry,
/// and the verdict the test would otherwise have had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ExpectedFailureRecord {
    pub owner: String,
    pub reported: &'static str,
}

/// One `"kind": "test"` record. Field order is serde's declaration order,
/// which the goldens pin but schema 1 does not make contractual.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TestRecord {
    pub schema: u32,
    pub kind: &'static str,
    pub name: String,
    pub module: Option<String>,
    pub file: String,
    pub verdict: &'static str,
    pub gating: bool,
    pub message: Option<String>,
    pub assertions: u64,
    pub duration_ms: u64,
    pub expected_failure: Option<ExpectedFailureRecord>,
    pub stdout: String,
    pub stderr: String,
}

/// The facts about one test that its outcome does not carry: where it lives,
/// what it measured, and what its body wrote while captured (D8).
pub(super) struct TestContext<'a> {
    pub name: &'a str,
    pub module: Option<String>,
    pub file: String,
    pub assertions: u64,
    pub duration_ms: u64,
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
}

/// The one `"kind": "test_summary"` record: the tally, the run's wall time,
/// and `result`. There is no `exit_code` field — the process status is it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TestSummaryRecord {
    pub schema: u32,
    pub kind: &'static str,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub timed_out: usize,
    pub black_holed: usize,
    pub never_compared: usize,
    pub asserted_nothing: usize,
    pub did_not_finish: usize,
    pub expected_failures: usize,
    pub duration_ms: u64,
    pub result: &'static str,
}

impl TestOutcome {
    /// The record's `verdict` string. A new value is an additive change.
    pub(super) fn verdict_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed(_) => "failed",
            Self::Skipped(_) => "skipped",
            Self::TimedOut { .. } => "timed_out",
            Self::BlackHole { .. } => "black_hole",
            Self::NeverCompared { .. } => "never_compared",
            Self::AssertedNothing => "asserted_nothing",
            Self::DidNotFinish { .. } => "did_not_finish",
            Self::ExpectedFailure { .. } => "expected_failure",
        }
    }

    /// Whether this verdict fails the run (D2) — the same set
    /// `Tally::is_failing` counts, so a consumer needs no verdict list.
    pub(super) fn gating(&self) -> bool {
        !matches!(
            self,
            Self::Passed | Self::Skipped(_) | Self::ExpectedFailure { .. }
        )
    }

    /// The text the human report prints after the verdict's label, or `None`
    /// for the verdicts that print nothing there. An expected failure carries
    /// its underlying verdict's message.
    pub(super) fn record_message(&self) -> Option<String> {
        match self {
            Self::Passed | Self::AssertedNothing => None,
            Self::Failed(text) | Self::Skipped(text) => Some(text.clone()),
            Self::NeverCompared { reason } => Some(reason.clone()),
            Self::TimedOut { secs, steps } => Some(format!(
                "after {secs}s (~{} steps)",
                format_step_count(*steps)
            )),
            Self::BlackHole { cycle } => Some(cycle.join(" → ")),
            Self::DidNotFinish { assertions } => Some(format!("after {assertions} assertion(s)")),
            Self::ExpectedFailure { reported, .. } => reported.record_message(),
        }
    }
}

/// Build one test's record from its outcome and context.
pub(super) fn test_record(outcome: &TestOutcome, context: &TestContext<'_>) -> TestRecord {
    let expected_failure = match outcome {
        TestOutcome::ExpectedFailure { owner, reported } => Some(ExpectedFailureRecord {
            owner: owner.clone(),
            reported: reported.verdict_str(),
        }),
        _ => None,
    };
    TestRecord {
        schema: SCHEMA_VERSION,
        kind: KIND_TEST,
        name: context.name.to_string(),
        module: context.module.clone(),
        file: context.file.clone(),
        verdict: outcome.verdict_str(),
        gating: outcome.gating(),
        message: outcome.record_message(),
        assertions: context.assertions,
        duration_ms: context.duration_ms,
        expected_failure,
        stdout: String::from_utf8_lossy(context.stdout).into_owned(),
        stderr: String::from_utf8_lossy(context.stderr).into_owned(),
    }
}

/// Build the summary record from the run's tally and wall time.
pub(super) fn summary_record(tally: &Tally, duration_ms: u64) -> TestSummaryRecord {
    TestSummaryRecord {
        schema: SCHEMA_VERSION,
        kind: KIND_TEST_SUMMARY,
        total: tally.total(),
        passed: tally.passed,
        failed: tally.failed,
        skipped: tally.skipped,
        timed_out: tally.timed_out,
        black_holed: tally.black_holed,
        never_compared: tally.never_compared,
        asserted_nothing: tally.asserted_nothing,
        did_not_finish: tally.did_not_finish,
        expected_failures: tally.expected_failures,
        duration_ms,
        result: if tally.is_failing() { "failed" } else { "ok" },
    }
}

/// Where one test lives: its module path joined with `::` (`None` for the
/// entry module) and that module's source file.
pub(super) type TestLocation = (Option<String>, String);

/// Where each test lives, for the records: name → [`TestLocation`]. The entry
/// module's file is spelled as the user passed it, as the human diagnostics
/// spell it (28.9.26e D12). A single-file run has no `module_defs` at all, so
/// [`location_of`] falls back to the entry file for any name not indexed here.
pub(super) fn test_locations(
    module_defs: &[(Vec<String>, PathBuf, Vec<CoreDef>)],
    entry_file: &Path,
) -> HashMap<String, TestLocation> {
    let mut locations = HashMap::new();
    for (module_path, source_file, defs) in module_defs {
        let location = if module_path.is_empty() {
            (None, entry_file.display().to_string())
        } else {
            (
                Some(module_path.join("::")),
                source_file.display().to_string(),
            )
        };
        for def in defs {
            locations
                .entry(def.name.clone())
                .or_insert_with(|| location.clone());
        }
    }
    locations
}

/// One test's location, defaulting to the entry module.
pub(super) fn location_of(
    locations: &HashMap<String, TestLocation>,
    name: &str,
    entry_file: &str,
) -> TestLocation {
    locations
        .get(name)
        .cloned()
        .unwrap_or_else(|| (None, entry_file.to_string()))
}

/// A record's `duration_ms`: whole milliseconds, truncated, saturating rather
/// than wrapping on a duration no test run will reach. Pure over the
/// `Duration`, so the conversion is assertable without timing anything.
pub(super) fn whole_ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// The summary line a run with no tests prints: every count zero, `ok`.
pub(super) fn empty_summary_line() -> String {
    to_line(&summary_record(&Tally::default(), 0))
}

#[cfg(test)]
mod tests;
