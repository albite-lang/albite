//! Test execution + reporting for `albite test` (ADR 29.6.26f / T13).
//!
//! Each `test_*` body is evaluated through the bootstrap evaluator (which executes
//! the assertion FFIs, P6-runtime) and the runtime failure flag is consulted to
//! decide `Passed`/`Failed`. Previously every elaborated test was reported `ok`
//! unconditionally — a silent-failure trap where `assert_eq(1, 2)` passed.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use albite_bootstrap::comparator::ComparatorTypes;
use albite_bootstrap::driver::diagnostics::json::to_line;
use albite_bootstrap::elaborate::CoreDef;
use albite_core::eval::{eval_with_env, eval_with_env_until, EvalStopped};
use albite_core::ffi::console_capture;
use albite_core::Term;

use super::json::{
    location_of, summary_record, test_record, whole_ms, TestContext, TestLocation, TestRecord,
};
use super::junit::{self, StopKind, SuiteInfo};
use super::stop::runner_error;
use super::summary::{apply_expected_failure, census_line, classify_finished_body};
use super::{TestExit, TestFormatArg, TestFunction, TestOutcome};

mod report;
pub(crate) use report::Tally;
use report::{format_report, report_outcome};

/// Options controlling a whole `albite test` run.
pub(super) struct RunOptions {
    pub check_only: bool,
    /// Per-test wall-clock bound in seconds; `0` disables the watchdog.
    pub watchdog_secs: u64,
    pub use_color: bool,
    /// Print the per-test executed-assertion count (ADR 6.8.26b D4).
    pub assertion_census: bool,
    /// Tests the manifest permits to fail, mapped to the successor ADR that
    /// owns removing each entry (ADR 6.8.26c D6). Empty is the normal state.
    pub expected_failures: BTreeMap<String, String>,
    /// `--format` (ADR 29.9.26i D1).
    pub format: TestFormatArg,
    /// Where each test lives, for the JSON records.
    pub locations: HashMap<String, TestLocation>,
    /// The entry file as the user spelled it — a test's file when it is not
    /// indexed in `locations`.
    pub entry_file: String,
}

/// Evaluate a single `test_*` body and report `Passed`/`Failed` from the runtime
/// failure flag. A 0-arg `test_*` stores its body directly (like `main`), so
/// evaluating the def's term runs the body — including any assertion FFIs.
///
/// Evaluation runs under a wall-clock watchdog (ADR 21.7.26f / D1) so a
/// non-terminating body becomes a named TIMEOUT failure rather than a silent
/// hang; `watchdog_secs == 0` restores the unbounded behaviour.
fn run_test_body(
    name: &str,
    defs: &[CoreDef],
    globals: &HashMap<String, Term>,
    comparator_types: &ComparatorTypes,
    watchdog_secs: u64,
) -> (TestOutcome, u64) {
    let Some(def) = defs.iter().find(|d| d.name == name) else {
        return (
            TestOutcome::Failed("test definition not found after discovery".to_string()),
            0,
        );
    };

    // Reset the per-test failure flag (null name is safe — the FFI null-checks it).
    albite_core::ffi::tg_test_begin(std::ptr::null(), 0);

    let env = albite_bootstrap::comparator::eval::eval_env(globals.clone(), comparator_types);
    let evaluated = if watchdog_secs == 0 {
        eval_with_env(&def.term.term, &env)
    } else {
        let deadline = Instant::now() + Duration::from_secs(watchdog_secs);
        eval_with_env_until(&def.term.term, &env, deadline)
    };
    let finished_value = match evaluated {
        Ok(value) => value,
        Err(EvalStopped::TimedOut { steps }) => {
            return (
                TestOutcome::TimedOut {
                    secs: watchdog_secs,
                    steps,
                },
                env.assertions_executed(),
            );
        }
        Err(EvalStopped::BlackHole { cycle }) => {
            return (TestOutcome::BlackHole { cycle }, env.assertions_executed());
        }
        // A comparison that never ran (ADR 1.8.26b D3). Reported before the
        // failure flag is consulted, because the flag is exactly what such a
        // test fails to set.
        Err(
            stopped @ (EvalStopped::Uncomparable(_)
            | EvalStopped::ComparisonNeverRan { .. }
            | EvalStopped::MalformedElimination { .. }),
        ) => {
            return (
                TestOutcome::NeverCompared {
                    reason: stopped.to_string(),
                },
                env.assertions_executed(),
            );
        }
        // The runner sets no step limit, so this stop cannot occur here; if
        // a limit is ever wired in, it must fail the test, never pass it.
        // An `Int` trap (ADR 14.9.26c) is a program error and fails the same
        // way — natively it would have aborted the harness. So does a read of
        // an unwritten byte (ADR 9.10.26n D3), a hole in the core's own copies.
        Err(
            stopped @ (EvalStopped::StepLimit { .. }
            | EvalStopped::IntTrap { .. }
            | EvalStopped::UnwrittenByteRead { .. }),
        ) => {
            return (
                TestOutcome::Failed(stopped.to_string()),
                env.assertions_executed(),
            );
        }
    };

    let assertions = env.assertions_executed();
    let outcome = classify_finished_body(
        albite_core::ffi::tg_test_check_failure() == 1,
        assertions,
        matches!(finished_value, Term::Unit),
    );
    (outcome, assertions)
}

/// Run one test — or skip it under check-only — and re-read the outcome
/// through the expected-failure list. Shared by both output arms, so the
/// verdicts cannot differ between them.
fn run_one_test(
    test: &TestFunction,
    defs: &[CoreDef],
    globals: &HashMap<String, Term>,
    comparator_types: &ComparatorTypes,
    opts: &RunOptions,
) -> (TestOutcome, u64) {
    let (outcome, assertions) = if opts.check_only {
        // check-only: `expect_type`/`expect_error` already ran during elaboration.
        (TestOutcome::Skipped("check-only".to_string()), 0)
    } else {
        run_test_body(
            &test.name,
            defs,
            globals,
            comparator_types,
            opts.watchdog_secs,
        )
    };
    let outcome = apply_expected_failure(
        outcome,
        opts.expected_failures.get(&test.name).map(String::as_str),
    );
    (outcome, assertions)
}

/// Run all tests and report them in `opts.format`. The exit is
/// [`TestExit::Failed`] iff the tally gates (ADR 29.9.26i D2).
pub(super) fn run_and_report(
    tests: &[TestFunction],
    defs: &[CoreDef],
    comparator_types: &ComparatorTypes,
    opts: &RunOptions,
) -> TestExit {
    match opts.format {
        TestFormatArg::Human => run_human(tests, defs, comparator_types, opts),
        TestFormatArg::Json => run_json(tests, defs, comparator_types, opts),
        TestFormatArg::Junit => run_junit(tests, defs, comparator_types, opts),
    }
}

/// The JSON arm (ADR 29.9.26i D7/D8): one record per test as it finishes, then
/// the summary. Printing each record at once means a crash still leaves every
/// finished record on stdout.
fn run_json(
    tests: &[TestFunction],
    defs: &[CoreDef],
    comparator_types: &ComparatorTypes,
    opts: &RunOptions,
) -> TestExit {
    let emit = |record: TestRecord| println!("{}", to_line(&record));
    match run_captured(tests, defs, comparator_types, opts, emit) {
        Ok((tally, duration_ms)) => {
            println!("{}", to_line(&summary_record(&tally, duration_ms)));
            TestExit::from_failing(tally.is_failing())
        }
        Err(message) => runner_error(&message, TestFormatArg::Json, &opts.entry_file),
    }
}

/// The `JUnit` arm (ADR 30.9.26k D2): the same records, collected and rendered
/// as one document. A mid-run stop still writes one — the records so far,
/// then a `(runner)` error testcase (D6).
fn run_junit(
    tests: &[TestFunction],
    defs: &[CoreDef],
    comparator_types: &ComparatorTypes,
    opts: &RunOptions,
) -> TestExit {
    let mut records = Vec::new();
    let result = run_captured(tests, defs, comparator_types, opts, |record| {
        records.push(record);
    });
    match result {
        Ok((tally, duration_ms)) => {
            let suite = SuiteInfo::for_entry(&opts.entry_file, duration_ms);
            println!("{}", junit::document(&records, &suite));
            TestExit::from_failing(tally.is_failing())
        }
        Err(message) => {
            let suite = SuiteInfo::for_entry(&opts.entry_file, 0);
            let text = junit::stop_document(&records, StopKind::Runner, &message, &suite);
            println!("{text}");
            TestExit::RunnerError
        }
    }
}

/// The capture loop both machine formats share (ADR 30.9.26k D2), so their
/// verdicts, captured output and durations cannot differ. Each body runs
/// inside a console capture sink, so its output and its assertion diffs land
/// in its record rather than on the process's streams; `emit` receives each
/// record as it finishes. Returns the tally and the run's elapsed
/// milliseconds, or the message of the one mid-run stop: a capture sink that
/// cannot be installed.
fn run_captured(
    tests: &[TestFunction],
    defs: &[CoreDef],
    comparator_types: &ComparatorTypes,
    opts: &RunOptions,
    mut emit: impl FnMut(TestRecord),
) -> Result<(Tally, u64), String> {
    let globals = albite_bootstrap::comparator::eval::defs_as_globals(defs);
    let mut tally = Tally::default();
    let start = Instant::now();
    for test in tests {
        // Never a silently uncaptured run: a sink someone else holds is a
        // runner error, not a fall-back to the process streams.
        if let Err(e) = console_capture::install() {
            return Err(e.to_string());
        }
        let body_start = Instant::now();
        let (outcome, assertions) = run_one_test(test, defs, &globals, comparator_types, opts);
        let duration_ms = whole_ms(body_start.elapsed());
        let captured = console_capture::take().unwrap_or_default();
        tally.count(&outcome);
        let (module, file) = location_of(&opts.locations, &test.name, &opts.entry_file);
        let record = test_record(
            &outcome,
            &TestContext {
                name: &test.name,
                module,
                file,
                assertions,
                duration_ms,
                stdout: &captured.stdout,
                stderr: &captured.stderr,
            },
        );
        emit(record);
    }
    Ok((tally, whole_ms(start.elapsed())))
}

/// The human arm: the per-test lines, the failures block and the summary
/// line, byte-identical to the output before `--format` existed.
fn run_human(
    tests: &[TestFunction],
    defs: &[CoreDef],
    comparator_types: &ComparatorTypes,
    opts: &RunOptions,
) -> TestExit {
    let use_color = opts.use_color;
    let total = tests.len();
    println!(
        "\nrunning {} test{}",
        total,
        if total == 1 { "" } else { "s" }
    );
    println!();

    // Globals for environment-based evaluation of each test body — the same map
    // `albite run` builds for `main`, carrying the lazy `__cmp<T>` comparator
    // synthesis callback so `compare`/`assert_*` resolve (ADR 29.6.26f §T11.2a).
    let globals = albite_bootstrap::comparator::eval::defs_as_globals(defs);

    let mut tally = Tally::default();
    let mut failures: Vec<(&str, String)> = Vec::new();
    let start = Instant::now();

    for test in tests {
        let (outcome, assertions) = run_one_test(test, defs, &globals, comparator_types, opts);
        report_outcome(&test.name, &outcome, &mut tally, &mut failures, use_color);
        if opts.assertion_census {
            println!("{}", census_line(assertions));
        }
    }

    println!(
        "{}",
        format_report(&tally, &failures, start.elapsed().as_secs_f64(), use_color)
    );

    TestExit::from_failing(tally.is_failing())
}

#[cfg(test)]
// Tests: tests_watchdog.rs
#[path = "tests_watchdog.rs"]
mod tests_watchdog;

#[cfg(test)]
// Tests: tests_never_compared.rs
#[path = "tests_never_compared.rs"]
mod tests_never_compared;

#[cfg(test)]
// Tests: tests_asserted_nothing.rs
#[path = "tests_asserted_nothing.rs"]
mod tests_asserted_nothing;

#[cfg(test)]
// Tests: tests_expected_failure.rs
#[path = "tests_expected_failure.rs"]
mod tests_expected_failure;
