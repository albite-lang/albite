//! `albite test --format junit` through the binary (ADR 30.9.26k AC 4).
//!
//! What only a spawned process can show: that stdout holds exactly one
//! well-formed document on every exit — a passing run, a failing one, a
//! diagnostics stop, a runner error — that the exit code is the one JSON mode
//! gives, and that a body's output and assertion diff land in the document
//! rather than on the process's streams. The fixture shapes are copied from
//! `test_format_json.rs`, since integration-test files are separate crates.

use std::path::{Path, PathBuf};
use std::process::Command;

const ASSERT_PRELUDE: &str = "extern \"C\" fn tg_assert_eq_nat(left: Nat, right: Nat) -> Unit\n\
     fn assert_eq_nat(a: Nat, b: Nat) -> Unit { tg_assert_eq_nat(a, b) }\n";

const ALL_PASS: &str = "pub fn test_one() -> Unit { assert_eq_nat(1, 1) }\n";
const ONE_FAILS: &str = "pub fn test_ok() -> Unit { assert_eq_nat(1, 1) }\n\
                         pub fn test_bad() -> Unit { assert_eq_nat(2, 3) }\n";
const ASSERTS_NOTHING: &str = "pub fn test_empty() -> Unit { () }\n";
const TYPE_ERROR: &str = "pub fn test_t() -> Unit { assert_eq_nat(true, 1) }\n";
const NO_TESTS: &str = "fn helper() -> Nat { 1 }\n";
const PRINTS_HOSTILE: &str = "extern \"C\" fn tg_println(s: Nat, len: Nat) -> Unit\n\
     extern \"C\" fn tg_string_to_cstring(s: String) -> Nat\n\
     extern \"C\" fn tg_string_len_internal(s: String) -> Nat\n\
     extern \"C\" fn tg_free_string(s: Nat) -> Unit\n\
     fn println(s: String) -> Unit {\n\
         let c = tg_string_to_cstring(s);\n\
         let len = tg_string_len_internal(s);\n\
         tg_println(c, len);\n\
         tg_free_string(c)\n\
     }\n\
     pub fn test_prints() -> Unit {\n\
         println(\"<b>&]]>\\x1b[31m\");\n\
         assert_eq_nat(1, 1)\n\
     }\n";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// One testcase, reduced to what AC 4 compares: its name, the child element
/// its verdict maps to (`failure`, `skipped`, `error` or `none`), and that
/// child's text.
#[derive(Debug, PartialEq)]
struct Case {
    name: String,
    outcome: String,
    text: String,
    system_out: Option<String>,
}

impl Run {
    /// Stdout parsed as exactly one XML document, reduced to its testcases —
    /// panics when stdout is not one well-formed document.
    fn cases(&self) -> Vec<Case> {
        let doc = roxmltree::Document::parse(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not one XML document ({e}):\n{}", self.stdout));
        assert_eq!(doc.root_element().tag_name().name(), "testsuites");
        doc.descendants()
            .filter(|n| n.has_tag_name("testcase"))
            .map(|case| {
                let verdict = case.children().find(|n| {
                    ["failure", "skipped", "error"]
                        .iter()
                        .any(|t| n.has_tag_name(*t))
                });
                let system_out = case
                    .children()
                    .find(|n| n.has_tag_name("system-out"))
                    .map(|n| n.text().unwrap_or("").to_string());
                Case {
                    name: case.attribute("name").unwrap().to_string(),
                    outcome: verdict.map_or("none", |n| n.tag_name().name()).to_string(),
                    text: verdict.and_then(|n| n.text()).unwrap_or("").to_string(),
                    system_out,
                }
            })
            .collect()
    }
}

fn fixture(source: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let entry = dir.path().join("f.tg");
    std::fs::write(&entry, source).expect("write fixture");
    (dir, entry)
}

fn run_test(entry: &Path, extra: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_albite"))
        .arg("test")
        .arg(entry)
        .args(extra)
        .env("ALBITE_NO_CACHE", "1")
        .output()
        .expect("spawn tungsten");
    Run {
        code: output.status.code().expect("exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn with_prelude(body: &str) -> String {
    format!("{ASSERT_PRELUDE}{body}")
}

/// Run `source` under `--format junit` (plus `extra`), assert the exit code,
/// and return the run.
fn junit_run(source: &str, extra: &[&str], code: i32) -> Run {
    let (_dir, entry) = fixture(source);
    let mut args = extra.to_vec();
    args.extend(["--format", "junit"]);
    let run = run_test(&entry, &args);
    assert_eq!(run.code, code, "junit: {}{}", run.stdout, run.stderr);
    run
}

// 30.9.26k AC4: all pass is exit 0 and one passing testcase.
#[test]
fn all_pass_is_exit_zero_with_no_verdict_child() {
    let run = junit_run(&with_prelude(ALL_PASS), &[], 0);
    let cases = run.cases();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].name, "test_one");
    assert_eq!(cases[0].outcome, "none");
}

// 30.9.26k AC4: the assertion diff rides in `<failure>`, not on stderr.
#[test]
fn a_failing_assertion_carries_its_diff_in_the_document() {
    let run = junit_run(&with_prelude(ONE_FAILS), &[], 1);
    let bad = run
        .cases()
        .into_iter()
        .find(|c| c.name == "test_bad")
        .unwrap();
    assert_eq!(bad.outcome, "failure");
    assert!(bad.text.contains("left:  2"), "{}", bad.text);
    assert!(bad.text.contains("right: 3"), "{}", bad.text);
    assert!(!run.stderr.contains("left:"), "{}", run.stderr);
}

#[test]
fn asserted_nothing_is_exit_one_and_a_failure() {
    let run = junit_run(&with_prelude(ASSERTS_NOTHING), &[], 1);
    assert_eq!(run.cases()[0].outcome, "failure");
}

// 30.9.26k AC4: a type error is exit 1, one `(elaboration)` error, and the
// human diagnostic on stderr.
#[test]
fn a_type_error_is_one_elaboration_error_and_a_human_diagnostic() {
    let run = junit_run(&with_prelude(TYPE_ERROR), &[], 1);
    let cases = run.cases();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].name, "(elaboration)");
    assert_eq!(cases[0].outcome, "error");
    assert!(run.stderr.contains("E0010"), "{}", run.stderr);
    assert!(
        !run.stderr.contains("\"kind\""),
        "diagnostics stay human: {}",
        run.stderr
    );
}

// 30.9.26k AC4: a missing file is exit 2 and one `(runner)` error.
#[test]
fn a_missing_file_is_one_runner_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let run = run_test(&dir.path().join("absent.tg"), &["--format", "junit"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    let cases = run.cases();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].name, "(runner)");
    assert_eq!(cases[0].outcome, "error");
}

#[test]
fn require_tests_over_nothing_is_one_runner_error() {
    let run = junit_run(NO_TESTS, &["--require-tests"], 2);
    let cases = run.cases();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].name, "(runner)");
}

#[test]
fn no_tests_is_exit_zero_and_an_empty_suite() {
    let run = junit_run(NO_TESTS, &[], 0);
    assert!(run.cases().is_empty());
    assert!(run.stdout.contains("tests=\"0\""), "{}", run.stdout);
}

// 30.9.26k AC4: a body printing markup and ESC round-trips through
// `<system-out>`, ESC as its visible `\u{1b}` text.
#[test]
fn hostile_body_output_round_trips_through_system_out() {
    let run = junit_run(&with_prelude(PRINTS_HOSTILE), &[], 0);
    let cases = run.cases();
    assert_eq!(cases[0].system_out.as_deref(), Some("<b>&]]>\\u{1b}[31m\n"));
}

/// JSON's `(name, verdict)` list, mapped through D5 to JUnit's child element.
fn json_cases_as_junit(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|record| record["kind"] == "test")
        .map(|record| {
            let outcome = match (
                record["gating"].as_bool().unwrap(),
                record["verdict"].as_str().unwrap(),
            ) {
                (true, _) => "failure",
                (false, "skipped" | "expected_failure") => "skipped",
                (false, _) => "none",
            };
            (
                record["name"].as_str().unwrap().to_string(),
                outcome.to_string(),
            )
        })
        .collect()
}

// 30.9.26k AC4: over the same fixtures, JSON and JUnit agree on every verdict
// under D5's mapping, and on the exit code.
#[test]
fn junit_and_json_agree_on_verdicts_and_exit() {
    for source in [
        with_prelude(ALL_PASS),
        with_prelude(ONE_FAILS),
        with_prelude(ASSERTS_NOTHING),
        with_prelude(PRINTS_HOSTILE),
    ] {
        let (_dir, entry) = fixture(&source);
        let json = run_test(&entry, &["--format", "json"]);
        let junit = run_test(&entry, &["--format", "junit"]);
        assert_eq!(json.code, junit.code, "{source}");
        let from_junit: Vec<(String, String)> = junit
            .cases()
            .into_iter()
            .map(|case| (case.name, case.outcome))
            .collect();
        assert_eq!(json_cases_as_junit(&json.stdout), from_junit, "{source}");
    }
}
