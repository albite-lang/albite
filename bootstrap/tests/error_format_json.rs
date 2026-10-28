//! `albite check --error-format json` through the binary (ADR 28.9.26e
//! AC2–AC4).
//!
//! What only a spawned process can show: which stream each byte reaches and
//! the exit code a shell sees. Fixtures are written into a tempdir so the
//! assertions hold in a mutation sweep's copied workspace too, and every run
//! is `--no-cache` so a warm cache cannot change what is reported.

use std::path::PathBuf;
use std::process::Command;

const TYPE_ERROR: &str = "fn f() -> Nat { true }\n";
const PARSE_ERROR: &str = "fn f( -> Nat { 1 }\n";
const CLEAN: &str = "fn f() -> Nat { 1 }\n";
const UNREACHABLE_ARM: &str = "fn f(n: Int) -> Nat {\n    match n { 1 => 1, 1 => 2, _ => 3 }\n}\n";
const THREE_ERRORS: &str = "fn a() -> Nat { true }\n\
                            fn b() -> Nat { false }\n\
                            fn c() -> Bool { 1 }\n";

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
}

/// Write a module tree into a fresh tempdir and return it with the entry file.
fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    for (name, source) in files {
        std::fs::write(dir.path().join(name), source).expect("write fixture");
    }
    let entry = dir.path().join(files[0].0);
    (dir, entry)
}

fn check(entry: &std::path::Path, extra: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_albite"))
        .arg("check")
        .arg(entry)
        .arg("--no-cache")
        .args(extra)
        .output()
        .expect("spawn tungsten");
    Run {
        code: output.status.code().expect("exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn check_json(files: &[(&str, &str)]) -> Run {
    let (_dir, entry) = fixture(files);
    let run = check(&entry, &["--error-format", "json"]);
    assert_no_human_rendering(&run);
    run
}

/// The renderer writes nothing to stderr in JSON mode (D3): no ariadne
/// report, no summary, and never the retired placeholder.
fn assert_no_human_rendering(run: &Run) {
    assert!(
        !run.stderr.contains("aborting due to"),
        "summary on stderr:\n{}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("─╯") && !run.stderr.contains("╭─["),
        "ariadne report on stderr:\n{}",
        run.stderr
    );
    assert!(
        !run.stdout.contains("E9999") && !run.stderr.contains("E9999"),
        "the E9999 placeholder is back:\n{}{}",
        run.stdout,
        run.stderr
    );
}

// AC2
#[test]
fn a_type_error_reports_its_real_code() {
    let run = check_json(&[("main.tg", TYPE_ERROR)]);
    assert_eq!(run.code, 1);
    let records = run.records();
    assert_eq!(records.len(), 1, "{}", run.stdout);
    assert_eq!(records[0]["schema"], 1);
    assert_eq!(records[0]["level"], "error");
    assert_eq!(records[0]["code"], "E0010");
}

// AC2 (28.9.26e), amended by 29.9.26f AC3: a parse error now carries its
// syntax-block code instead of `null`.
#[test]
fn a_parse_error_reports_a_syntax_code() {
    let run = check_json(&[("main.tg", PARSE_ERROR)]);
    assert_eq!(run.code, 1);
    let records = run.records();
    assert!(!records.is_empty());
    assert!(
        records.iter().all(|r| is_syntax_code(&r["code"])),
        "{}",
        run.stdout
    );
    assert!(records.iter().all(|r| r["level"] == "error"));
}

fn is_syntax_code(code: &serde_json::Value) -> bool {
    code.as_str()
        .is_some_and(|c| ("E0110"..="E0117").contains(&c))
}

// AC2
#[test]
fn an_error_in_a_sibling_module_names_that_file() {
    let run = check_json(&[
        (
            "main.tg",
            "mod helper;\nuse helper::broken;\nfn main() -> Nat { broken() }\n",
        ),
        ("helper.tg", "pub fn broken() -> Nat { true }\n"),
    ]);
    assert_eq!(run.code, 1);
    let records = run.records();
    let error = records
        .iter()
        .find(|r| r["level"] == "error")
        .unwrap_or_else(|| panic!("no error record:\n{}", run.stdout));
    let file = error["primary_span"]["file"].as_str().unwrap();
    assert!(file.ends_with("/helper.tg"), "{file}");
}

// AC2
#[test]
fn a_clean_file_writes_no_stdout() {
    let run = check_json(&[("main.tg", CLEAN)]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

// AC2
#[test]
fn a_warning_alone_exits_zero_with_one_warning_record() {
    let run = check_json(&[("main.tg", UNREACHABLE_ARM)]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let records = run.records();
    assert_eq!(records.len(), 1, "{}", run.stdout);
    assert_eq!(records[0]["level"], "warning");
    assert_eq!(records[0]["code"], "W0001");
}

// AC2
#[test]
fn a_missing_file_is_one_location_less_record_and_exit_3() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let run = check(&dir.path().join("absent.tg"), &["--error-format=json"]);
    assert_no_human_rendering(&run);
    assert_eq!(run.code, 3);
    let records = run.records();
    assert_eq!(records.len(), 1, "{}", run.stdout);
    assert!(records[0]["code"].is_null());
    assert!(records[0]["primary_span"].is_null());
}

// AC3
#[test]
fn the_json_alias_is_byte_identical_to_the_flag() {
    for source in [TYPE_ERROR, CLEAN] {
        let (_dir, entry) = fixture(&[("main.tg", source)]);
        let via_error_format = check(&entry, &["--error-format=json"]);
        let via_alias = check(&entry, &["--json"]);
        assert_eq!(via_error_format.code, via_alias.code);
        assert_eq!(via_error_format.stdout, via_alias.stdout);
    }
}

// AC3
#[test]
fn the_json_alias_beside_an_explicit_format_is_a_usage_error() {
    let (_dir, entry) = fixture(&[("main.tg", CLEAN)]);
    let run = check(&entry, &["--json", "--error-format", "human"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
}

// AC4
#[test]
fn json_ignores_the_display_budget() {
    let (_dir, entry) = fixture(&[("main.tg", THREE_ERRORS)]);
    let json = check(&entry, &["--error-format=json", "--max-errors", "2"]);
    let errors = json
        .records()
        .iter()
        .filter(|r| r["level"] == "error")
        .count();
    let human = check(&entry, &["--max-errors", "0"]);
    assert!(
        human
            .stderr
            .contains(&format!("aborting due to {errors} errors")),
        "JSON reported {errors}; human said:\n{}",
        human.stderr
    );
    assert!(errors > 2, "the fixture must exceed the budget: {errors}");
}

// ─────────────────────────────────────────────────────────────────────
// `run --error-format json` (ADR 29.9.26f AC5): records on stderr, stdout
// left to the program.
// ─────────────────────────────────────────────────────────────────────

const PRINTS_AND_RETURNS: &str = "\
extern \"C\" fn tg_string_to_cstring(s: String) -> Nat
extern \"C\" fn tg_string_len_internal(s: String) -> Nat
extern \"C\" fn tg_free_string(s: Nat) -> Unit
extern \"C\" fn tg_println(s: Nat, len: Nat) -> Unit
extern \"C\" fn tg_eprintln(s: Nat, len: Nat) -> Unit

fn main() -> Nat {
    let out = tg_string_to_cstring(\"hello\");
    let _a = tg_println(out, tg_string_len_internal(\"hello\"));
    let _b = tg_free_string(out);
    let err = tg_string_to_cstring(\"oops\");
    let _c = tg_eprintln(err, tg_string_len_internal(\"oops\"));
    let _d = tg_free_string(err);
    42
}
";
const TYPE_ERROR_IN_MAIN: &str = "fn main() -> Nat { true }\n";
const BLACK_HOLE: &str = "\
#[partial]
fn loop_forever() -> Nat { loop_forever() }

fn main() -> Nat { loop_forever() }
";

fn run_cmd(entry: &std::path::Path, extra: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_albite"))
        .arg("run")
        .arg(entry)
        .arg("--no-cache")
        .args(extra)
        .output()
        .expect("spawn tungsten");
    Run {
        code: output.status.code().expect("exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `albite run --error-format json` on a one-file fixture, with the checks
/// every JSON case shares: no human rendering and no Signature Collection note.
fn run_json(source: &str) -> Run {
    let (_dir, entry) = fixture(&[("main.tg", source)]);
    let run = run_cmd(&entry, &["--error-format", "json"]);
    assert_no_human_rendering(&run);
    assert!(
        !run.stderr.contains("Signature Collection"),
        "the Signature Collection note is human-only (D11):\n{}",
        run.stderr
    );
    run
}

/// Every stderr line of a failing JSON run, parsed as a schema-1 record.
fn stderr_records(run: &Run) -> Vec<serde_json::Value> {
    run.stderr
        .lines()
        .map(|line| {
            let record: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("stderr line is not JSON ({e}): {line}"));
            assert_eq!(record["schema"], 1, "{line}");
            record
        })
        .collect()
}

#[test]
fn run_json_reports_a_type_error_on_stderr_only() {
    let run = run_json(TYPE_ERROR_IN_MAIN);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let records = stderr_records(&run);
    let e0010 = records.iter().filter(|r| r["code"] == "E0010").count();
    assert_eq!(e0010, 1, "{}", run.stderr);
}

#[test]
fn run_json_reports_a_parse_error_with_a_syntax_code() {
    let run = run_json(PARSE_ERROR);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let records = stderr_records(&run);
    assert!(!records.is_empty());
    assert!(
        records.iter().any(|r| is_syntax_code(&r["code"])),
        "{}",
        run.stderr
    );
}

#[test]
fn run_json_leaves_the_programs_streams_alone() {
    let (_dir, entry) = fixture(&[("main.tg", PRINTS_AND_RETURNS)]);
    let human = run_cmd(&entry, &[]);
    let json = run_cmd(&entry, &["--error-format", "json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.stdout, "hello\n42\n");
    assert_eq!(
        json.stdout, human.stdout,
        "stdout must not depend on the format"
    );
    assert_eq!(json.stderr, "oops\n");
}

#[test]
fn run_json_reports_a_missing_main_as_one_e0030_record() {
    let run = run_json(CLEAN);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let records = stderr_records(&run);
    assert_eq!(records.len(), 1, "{}", run.stderr);
    assert_eq!(records[0]["code"], "E0030");
}

#[test]
fn run_json_reports_a_black_hole_as_an_uncoded_record() {
    let run = run_json(BLACK_HOLE);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let records = stderr_records(&run);
    assert_eq!(records.len(), 1, "{}", run.stderr);
    let stop = &records[0];
    assert_eq!(stop["level"], "error");
    assert!(stop["code"].is_null());
    assert!(stop["primary_span"].is_null());
    assert!(stop["message"].as_str().unwrap().contains("black hole"));
    assert_eq!(
        stop["notes"][0]["message"],
        "the definition re-enters itself while being forced"
    );
}

#[test]
fn run_json_reports_a_missing_file_as_exit_3_on_stderr() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let run = run_cmd(&dir.path().join("absent.tg"), &["--error-format=json"]);
    assert_no_human_rendering(&run);
    assert_eq!(run.code, 3);
    assert_eq!(run.stdout, "");
    let records = stderr_records(&run);
    assert_eq!(records.len(), 1, "{}", run.stderr);
    assert!(records[0]["code"].is_null());
    assert!(records[0]["primary_span"].is_null());
}
