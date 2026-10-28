//! `albite test`'s exit contract and `--format json` stream through the
//! binary (ADR 29.9.26i AC 2 and AC 4).
//!
//! What only a spawned process can show: the exit code a shell sees and which
//! stream each byte reaches — in particular that a test body's own console
//! output and its assertion diff land in its record rather than on the
//! process's streams, which needs the real capture sink in a real process.
//! Fixtures are written into tempdirs, so the assertions hold in a mutation
//! sweep's copied workspace too, and every run disables the cache.

use std::path::{Path, PathBuf};
use std::process::Command;

const ASSERT_PRELUDE: &str = "extern \"C\" fn tg_assert_eq_nat(left: Nat, right: Nat) -> Unit\n\
     fn assert_eq_nat(a: Nat, b: Nat) -> Unit { tg_assert_eq_nat(a, b) }\n";

const ALL_PASS: &str = "pub fn test_one() -> Unit { assert_eq_nat(1, 1) }\n";
const ONE_FAILS: &str = "pub fn test_ok() -> Unit { assert_eq_nat(1, 1) }\n\
                         pub fn test_bad() -> Unit { assert_eq_nat(2, 3) }\n";
const ASSERTS_NOTHING: &str = "pub fn test_empty() -> Unit { () }\n";
const EXPECT_TYPE_FAILS: &str = "pub fn test_t() -> Unit { expect_type(1, \"Bool\") }\n";
const TYPE_ERROR: &str = "pub fn test_t() -> Unit { assert_eq_nat(true, 1) }\n";
const NO_TESTS: &str = "fn helper() -> Nat { 1 }\n";
const PRINTS: &str = "extern \"C\" fn tg_println(s: Nat, len: Nat) -> Unit\n\
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
         println(\"hello from body\");\n\
         assert_eq_nat(1, 1)\n\
     }\n";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Every stdout line, parsed — panics on a line that is not one JSON object.
    fn records(&self) -> Vec<serde_json::Value> {
        self.stdout
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"))
            })
            .collect()
    }

    fn kinds(&self) -> Vec<String> {
        self.records()
            .iter()
            .map(|r| r["kind"].as_str().unwrap_or("?").to_string())
            .collect()
    }
}

/// Write `files` (relative paths, parents created) into a fresh tempdir and
/// return it with the first file as the entry.
fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    for (name, source) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).expect("create fixture dir");
        std::fs::write(path, source).expect("write fixture");
    }
    let entry = dir.path().join(files[0].0);
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

/// Run `files` in both formats; assert both exit `code`, and return the JSON run.
fn both_formats_exit(files: &[(&str, &str)], extra: &[&str], code: i32) -> Run {
    let (_dir, entry) = fixture(files);
    let human = run_test(&entry, extra);
    assert_eq!(human.code, code, "human: {}{}", human.stdout, human.stderr);
    let mut json_args = extra.to_vec();
    json_args.extend(["--format", "json"]);
    let json = run_test(&entry, &json_args);
    assert_eq!(json.code, code, "json: {}{}", json.stdout, json.stderr);
    json.records(); // every stdout line parses
    json
}

// ── AC 2: the exit codes, in both formats ────────────────────────────────────

// 29.9.26i AC2: all pass 0; a failing assertion 1; asserted nothing 1.
#[test]
fn verdicts_exit_zero_or_one() {
    both_formats_exit(&[("f.tg", &with_prelude(ALL_PASS))], &[], 0);
    both_formats_exit(&[("f.tg", &with_prelude(ONE_FAILS))], &[], 1);
    both_formats_exit(&[("f.tg", &with_prelude(ASSERTS_NOTHING))], &[], 1);
}

// 29.9.26i AC2: diagnostics are 1 (D3), including a failing `expect_type`.
#[test]
fn diagnostics_exit_one() {
    both_formats_exit(&[("f.tg", EXPECT_TYPE_FAILS)], &[], 1);
    both_formats_exit(&[("f.tg", &with_prelude(TYPE_ERROR))], &[], 1);
}

// 29.9.26i AC2: a missing file and a missing module are runner errors (D4).
#[test]
fn the_module_tree_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist.tg");
    assert_eq!(run_test(&missing, &[]).code, 2);
    assert_eq!(run_test(&missing, &["--format", "json"]).code, 2);
    both_formats_exit(&[("f.tg", "mod nowhere;\nfn x() -> Nat { 1 }\n")], &[], 2);
}

// 29.9.26i AC2: `--require-tests` over zero tests, a malformed manifest and an
// ambiguous `--module` are runner errors.
#[test]
fn runner_refusals_exit_two() {
    // Each refusal is one record naming its cause, so the test asserts WHICH
    // arm fired rather than only that something exited 2.
    let only_message = |run: Run| {
        let records = run.records();
        assert_eq!(records.len(), 1, "{}", run.stdout);
        records[0]["message"].as_str().unwrap().to_string()
    };
    let zero = both_formats_exit(&[("f.tg", NO_TESTS)], &["--require-tests"], 2);
    assert!(only_message(zero).contains("zero runnable tests"));
    let manifest = both_formats_exit(
        &[
            ("f.tg", &with_prelude(ALL_PASS)),
            ("tg-test-tiers.toml", "this is [not toml"),
        ],
        &[],
        2,
    );
    assert!(only_message(manifest).contains("tg-test-tiers.toml is unreadable"));
    let ambiguous = both_formats_exit(&ambiguous_modules(), &["--module", "m.tg"], 2);
    assert!(only_message(ambiguous).contains("is ambiguous, matched:"));
}

/// Two modules whose paths both end in `m.tg`, so `--module m.tg` is ambiguous.
fn ambiguous_modules() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "main.tg",
            "mod a;\nmod b;\npub fn test_main() -> Unit { a::m::one() }\n",
        ),
        ("a/mod.tg", "pub mod m;\n"),
        (
            "a/m.tg",
            "extern \"C\" fn tg_assert_eq_nat(left: Nat, right: Nat) -> Unit\n\
             pub fn one() -> Unit { tg_assert_eq_nat(1, 1) }\n",
        ),
        ("b/mod.tg", "pub mod m;\n"),
        ("b/m.tg", "pub fn two() -> Nat { 2 }\n"),
    ]
}

// ── AC 4: the JSON stream ────────────────────────────────────────────────────

// 29.9.26i AC4: records only; tests then exactly one summary, last.
#[test]
fn a_run_is_test_records_then_one_summary() {
    let run = both_formats_exit(&[("f.tg", &with_prelude(ONE_FAILS))], &[], 1);
    assert_eq!(run.kinds(), ["test", "test", "test_summary"]);
    let summary = &run.records()[2];
    assert_eq!(summary["total"], 2);
    assert_eq!(summary["failed"], 1);
    assert_eq!(summary["result"], "failed");
    assert!(!run.stderr.contains("running"), "{}", run.stderr);
    assert!(!run.stderr.contains("test test_"), "{}", run.stderr);
}

// 29.9.26i AC4: a type error yields diagnostics, no test record, no summary.
#[test]
fn a_type_error_is_diagnostics_only() {
    let run = both_formats_exit(&[("f.tg", &with_prelude(TYPE_ERROR))], &[], 1);
    let kinds = run.kinds();
    assert!(!kinds.is_empty());
    assert!(kinds.iter().all(|k| k == "diagnostic"), "{kinds:?}");
    assert!(
        !run.stderr.contains("Error:"),
        "ariadne leaked: {}",
        run.stderr
    );
}

// 29.9.26i AC4: a missing file is exactly one code-less record.
#[test]
fn a_missing_file_is_one_codeless_record() {
    let dir = tempfile::tempdir().unwrap();
    let run = run_test(&dir.path().join("nope.tg"), &["--format", "json"]);
    assert_eq!(run.code, 2);
    let records = run.records();
    assert_eq!(records.len(), 1, "{}", run.stdout);
    assert_eq!(records[0]["kind"], "diagnostic");
    assert!(records[0]["code"].is_null());
    assert!(records[0]["primary_span"].is_null());
}

// 29.9.26i AC4 (D8): a printing body leaves stdout well formed.
#[test]
fn a_printing_body_is_captured_into_its_record() {
    let run = both_formats_exit(&[("f.tg", &with_prelude(PRINTS))], &[], 0);
    let records = run.records();
    assert_eq!(records[0]["name"], "test_prints");
    assert_eq!(records[0]["stdout"], "hello from body\n");
    assert!(!run.stderr.contains("hello from body"));
}

// 29.9.26i AC4 (D8a): the assertion diff reaches the record, not stderr.
#[test]
fn an_assertion_diff_is_captured_into_its_record() {
    let run = both_formats_exit(&[("f.tg", &with_prelude(ONE_FAILS))], &[], 1);
    let failing = &run.records()[1];
    assert_eq!(failing["verdict"], "failed");
    assert_eq!(failing["message"], "assertion failed");
    let stderr = failing["stderr"].as_str().unwrap();
    assert!(stderr.contains("left:  2"), "{stderr}");
    assert!(stderr.contains("right: 3"), "{stderr}");
    assert!(!run.stderr.contains("left:"), "{}", run.stderr);
}

// 29.9.26i AC4: an unmatched `--module` without `--require-tests` is a
// passing run of zero tests — one zero summary and nothing else.
#[test]
fn an_unmatched_module_is_an_empty_passing_run() {
    let run = both_formats_exit(
        &[("f.tg", &with_prelude(ALL_PASS))],
        &["--module", "zzz.tg"],
        0,
    );
    assert_eq!(run.kinds(), ["test_summary"]);
    assert_eq!(run.records()[0]["total"], 0);
}

// 29.9.26i D7: in human mode a file with no tests says `no tests found` on
// stdout, while an unmatched `--module` says only its stderr warning — the
// two empty runs differ, and only the golden pinned the first until this.
#[test]
fn the_two_human_empty_runs_print_different_things() {
    let (_dir, entry) = fixture(&[("f.tg", NO_TESTS)]);
    let no_tests = run_test(&entry, &[]);
    assert_eq!(no_tests.code, 0);
    assert_eq!(no_tests.stdout.trim(), "no tests found");
    let (_dir, entry) = fixture(&[("f.tg", &with_prelude(ALL_PASS))]);
    let unmatched = run_test(&entry, &["--module", "zzz.tg"]);
    assert_eq!(unmatched.code, 0);
    assert_eq!(unmatched.stdout.trim(), "", "{}", unmatched.stdout);
    assert!(
        unmatched
            .stderr
            .contains("no module matching 'zzz.tg' found; 0 tests run"),
        "{}",
        unmatched.stderr
    );
}

#[test]
fn no_tests_is_one_zero_summary() {
    let run = both_formats_exit(&[("f.tg", NO_TESTS)], &[], 0);
    assert_eq!(run.kinds(), ["test_summary"]);
    assert_eq!(run.records()[0]["result"], "ok");
}

// ── AC 4: agreement with the human run over the runner fixtures ──────────────

const RUNNER_FIXTURES: [(&str, &str); 6] = [
    (
        "allpass",
        include_str!("../../tests/test_runner_allpass.tg"),
    ),
    (
        "pass_fail",
        include_str!("../../tests/test_runner_pass_fail.tg"),
    ),
    (
        "ten_failures",
        include_str!("../../tests/test_runner_ten_failures.tg"),
    ),
    (
        "asserted_nothing",
        include_str!("../../tests/test_runner_asserted_nothing.tg"),
    ),
    (
        "did_not_finish",
        include_str!("../../tests/test_runner_did_not_finish.tg"),
    ),
    (
        "user_main",
        include_str!("../../tests/test_runner_user_main.tg"),
    ),
];

/// The JSON `verdict` a human `test <name> ... <LABEL>` line reports.
fn verdict_of_label(label: &str) -> &'static str {
    match label {
        l if l.starts_with("ok") => "passed",
        l if l.starts_with("FAILED") => "failed",
        l if l.starts_with("ASSERTED NOTHING") => "asserted_nothing",
        l if l.starts_with("DID NOT FINISH") => "did_not_finish",
        other => panic!("unmapped human label: {other}"),
    }
}

// 29.9.26i AC4: the (name, verdict) list and the exit code agree across formats.
#[test]
fn json_and_human_agree_on_every_runner_fixture() {
    for (name, source) in RUNNER_FIXTURES {
        let (_dir, entry) = fixture(&[("f.tg", source)]);
        let human = run_test(&entry, &["--color", "never"]);
        let json = run_test(&entry, &["--format", "json"]);
        assert_eq!(human.code, json.code, "{name}: exit codes differ");
        let from_human: Vec<(String, String)> = human
            .stdout
            .lines()
            .filter_map(|line| line.strip_prefix("test "))
            .filter_map(|rest| rest.split_once(" ... "))
            .map(|(test, label)| (test.to_string(), verdict_of_label(label).to_string()))
            .collect();
        let from_json: Vec<(String, String)> = json
            .records()
            .iter()
            .filter(|r| r["kind"] == "test")
            .map(|r| {
                (
                    r["name"].as_str().unwrap().to_string(),
                    r["verdict"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert!(!from_human.is_empty(), "{name}: no human lines parsed");
        assert_eq!(from_human, from_json, "{name}");
    }
}
