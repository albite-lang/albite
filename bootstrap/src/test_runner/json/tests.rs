//! The record builders (ADR 29.9.26i AC 3): every verdict's string, gating
//! value and message; expected-failure nesting; the carried-through context;
//! the summary's counts and `result`; one line per record.

use std::path::{Path, PathBuf};

use super::*;
use albite_bootstrap::span::Span;
use albite_core::terms::SpannedTerm;
use albite_core::{Term, Type};

fn context<'a>(stdout: &'a [u8], stderr: &'a [u8]) -> TestContext<'a> {
    TestContext {
        name: "test_x",
        module: Some("a::b".to_string()),
        file: "src/a/b.tg".to_string(),
        assertions: 3,
        duration_ms: 17,
        stdout,
        stderr,
    }
}

fn record_of(outcome: &TestOutcome) -> TestRecord {
    test_record(outcome, &context(b"", b""))
}

// 29.9.26i AC3: each of the nine verdicts, as (outcome, verdict, gating, message).
#[test]
fn every_verdict_has_its_string_gating_value_and_message() {
    let cases: Vec<(TestOutcome, &str, bool, Option<&str>)> = vec![
        (TestOutcome::Passed, "passed", false, None),
        (
            TestOutcome::Failed("assertion failed".into()),
            "failed",
            true,
            Some("assertion failed"),
        ),
        (
            TestOutcome::Skipped("check-only".into()),
            "skipped",
            false,
            Some("check-only"),
        ),
        (
            TestOutcome::TimedOut {
                secs: 60,
                steps: 4_100_000,
            },
            "timed_out",
            true,
            Some("after 60s (~4.1M steps)"),
        ),
        (
            TestOutcome::BlackHole {
                cycle: vec!["f".into(), "g".into(), "f".into()],
            },
            "black_hole",
            true,
            Some("f → g → f"),
        ),
        (
            TestOutcome::NeverCompared {
                reason: "no comparator".into(),
            },
            "never_compared",
            true,
            Some("no comparator"),
        ),
        (TestOutcome::AssertedNothing, "asserted_nothing", true, None),
        (
            TestOutcome::DidNotFinish { assertions: 2 },
            "did_not_finish",
            true,
            Some("after 2 assertion(s)"),
        ),
        (
            TestOutcome::ExpectedFailure {
                owner: "1.2.26a".into(),
                reported: Box::new(TestOutcome::Failed("boom".into())),
            },
            "expected_failure",
            false,
            Some("boom"),
        ),
    ];
    for (outcome, verdict, gating, message) in cases {
        let record = record_of(&outcome);
        assert_eq!(record.verdict, verdict);
        assert_eq!(record.gating, gating, "{verdict}");
        assert_eq!(record.message.as_deref(), message, "{verdict}");
        assert_eq!(record.schema, 1);
        assert_eq!(record.kind, "test");
    }
}

#[test]
fn an_expected_failure_nests_its_owner_and_underlying_verdict() {
    let record = record_of(&TestOutcome::ExpectedFailure {
        owner: "6.8.26c".into(),
        reported: Box::new(TestOutcome::AssertedNothing),
    });
    assert_eq!(
        record.expected_failure,
        Some(ExpectedFailureRecord {
            owner: "6.8.26c".into(),
            reported: "asserted_nothing",
        })
    );
    assert!(!record.gating);
    assert!(record_of(&TestOutcome::Passed).expected_failure.is_none());
}

#[test]
fn the_context_is_carried_through_and_output_is_lossy_utf8() {
    let stderr = b"  assertion failed: assert_eq_nat\n    left:  2\n    right: 3\n";
    let record = test_record(
        &TestOutcome::Failed("assertion failed".into()),
        &context(b"hi\n\xff", stderr),
    );
    assert_eq!(record.name, "test_x");
    assert_eq!(record.module.as_deref(), Some("a::b"));
    assert_eq!(record.file, "src/a/b.tg");
    assert_eq!(record.assertions, 3);
    assert_eq!(record.duration_ms, 17);
    assert_eq!(record.stdout, "hi\n\u{fffd}");
    assert!(record.stderr.contains("left:  2"));
    assert!(record.stderr.contains("right: 3"));
}

#[test]
fn a_record_is_one_json_line_with_every_field() {
    let line = to_line(&test_record(&TestOutcome::Passed, &context(b"a\nb\n", b"")));
    assert!(!line.contains('\n'), "{line}");
    let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
    for field in [
        "schema",
        "kind",
        "name",
        "module",
        "file",
        "verdict",
        "gating",
        "message",
        "assertions",
        "duration_ms",
        "expected_failure",
        "stdout",
        "stderr",
    ] {
        assert!(parsed.get(field).is_some(), "missing {field}: {line}");
    }
    assert_eq!(parsed["stdout"], "a\nb\n");
}

// ── the summary ─────────────────────────────────────────────────────────────

#[test]
fn the_summary_counts_equal_the_tally() {
    let tally = Tally {
        passed: 1,
        failed: 2,
        skipped: 3,
        timed_out: 4,
        black_holed: 5,
        never_compared: 6,
        asserted_nothing: 7,
        did_not_finish: 8,
        expected_failures: 9,
    };
    let summary = summary_record(&tally, 250);
    assert_eq!(summary.schema, 1);
    assert_eq!(summary.kind, "test_summary");
    assert_eq!(summary.total, 45);
    assert_eq!(
        [
            summary.passed,
            summary.failed,
            summary.skipped,
            summary.timed_out,
            summary.black_holed,
            summary.never_compared,
            summary.asserted_nothing,
            summary.did_not_finish,
            summary.expected_failures,
        ],
        [1, 2, 3, 4, 5, 6, 7, 8, 9]
    );
    assert_eq!(summary.duration_ms, 250);
    assert_eq!(summary.result, "failed");
}

#[test]
fn the_summary_result_is_failed_exactly_when_the_tally_gates() {
    let clean = Tally {
        passed: 2,
        skipped: 1,
        expected_failures: 1,
        ..Tally::default()
    };
    assert_eq!(summary_record(&clean, 0).result, "ok");
    for gating in [
        Tally {
            failed: 1,
            ..Tally::default()
        },
        Tally {
            asserted_nothing: 1,
            ..Tally::default()
        },
        Tally {
            did_not_finish: 1,
            ..Tally::default()
        },
    ] {
        assert_eq!(summary_record(&gating, 0).result, "failed");
    }
}

#[test]
fn counting_an_outcome_moves_its_own_bucket_and_the_total() {
    let mut tally = Tally::default();
    for outcome in [
        TestOutcome::Passed,
        TestOutcome::Failed(String::new()),
        TestOutcome::Skipped(String::new()),
        TestOutcome::TimedOut { secs: 1, steps: 1 },
        TestOutcome::BlackHole { cycle: vec![] },
        TestOutcome::NeverCompared {
            reason: String::new(),
        },
        TestOutcome::AssertedNothing,
        TestOutcome::DidNotFinish { assertions: 1 },
        TestOutcome::ExpectedFailure {
            owner: String::new(),
            reported: Box::new(TestOutcome::AssertedNothing),
        },
    ] {
        tally.count(&outcome);
    }
    assert_eq!(tally.total(), 9);
    assert_eq!(summary_record(&tally, 0).expected_failures, 1);
    assert_eq!(
        tally.asserted_nothing, 1,
        "the nested verdict is not counted"
    );
}

// 29.9.26i D9: `duration_ms` is whole milliseconds, truncated and saturating.
#[test]
fn a_duration_is_whole_truncated_milliseconds() {
    use std::time::Duration;
    assert_eq!(whole_ms(Duration::ZERO), 0);
    assert_eq!(whole_ms(Duration::from_micros(1_999)), 1);
    assert_eq!(whole_ms(Duration::from_millis(250)), 250);
    assert_eq!(whole_ms(Duration::from_secs(3)), 3_000);
    assert_eq!(whole_ms(Duration::MAX), u64::MAX);
}

#[test]
fn an_empty_run_summarises_to_all_zeros_and_ok() {
    let parsed: serde_json::Value = serde_json::from_str(&empty_summary_line()).unwrap();
    assert_eq!(parsed["kind"], "test_summary");
    assert_eq!(parsed["total"], 0);
    assert_eq!(parsed["result"], "ok");
}

// ── locations ───────────────────────────────────────────────────────────────

fn def(name: &str) -> CoreDef {
    CoreDef {
        name: name.to_string(),
        ty: Type::Unit,
        term: SpannedTerm::generated(Term::Unit),
        span: Span::new(0, 0),
    }
}

#[test]
fn a_test_in_a_submodule_is_located_by_its_module_path_and_file() {
    let module_defs = vec![
        (
            vec![],
            PathBuf::from("/abs/main.tg"),
            vec![def("test_main")],
        ),
        (
            vec!["util".to_string(), "strings".to_string()],
            PathBuf::from("/abs/util/strings.tg"),
            vec![def("test_strings")],
        ),
    ];
    let locations = test_locations(&module_defs, Path::new("main.tg"));
    assert_eq!(
        location_of(&locations, "test_strings", "main.tg"),
        (
            Some("util::strings".to_string()),
            "/abs/util/strings.tg".to_string()
        )
    );
    // The entry module is spelled as the user passed it.
    assert_eq!(
        location_of(&locations, "test_main", "main.tg"),
        (None, "main.tg".to_string())
    );
    // A single-file run indexes nothing and falls back to the entry file.
    assert_eq!(
        location_of(&HashMap::new(), "test_any", "f.tg"),
        (None, "f.tg".to_string())
    );
}
