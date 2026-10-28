//! The `compile` arm's comparison, over captured runs (ADR 30.9.26j AC6).

use super::*;

const E0140_RECORD: &str = r#"{"schema":1,"kind":"diagnostic","level":"error","code":"E0140","message":"m","primary_span":null}"#;
const FLAG_CONFLICT_HUMAN: &str = "error: --no-codegen and --emit-llvm are incompatible.\n       --no-codegen stops before LLVM IR emission.\n";
const TYPE_ERROR_RECORD: &str = r#"{"schema":1,"kind":"diagnostic","level":"error","code":"E0010","message":"m","primary_span":{"file":"f.tg"}}"#;
const TYPE_ERROR_HUMAN: &str = "[E0010] Error: expected `Nat`, found `Bool`\nerror: aborting due to 1 error\nerror: compilation failed: elaboration errors\n";

fn captured(code: i32, stdout: &str, stderr: &str) -> Captured {
    Captured {
        code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

fn flag_conflict_pair() -> (Captured, Captured) {
    (
        captured(1, "", FLAG_CONFLICT_HUMAN),
        captured(1, "", &format!("{E0140_RECORD}\n")),
    )
}

#[test]
fn a_back_end_pair_with_one_record_per_failure_line_is_accepted() {
    let (human, json) = flag_conflict_pair();
    assert_eq!(compare_compile(&human, &json), Ok(()));
}

// 30.9.26j AC6: rule 5 rejects a human failure line with no record behind it.
// Exit 1 with nothing written is rule 1's case; a warning alone is rule 5's.
#[test]
fn rule_5_rejects_a_dropped_record() {
    let (human, _) = flag_conflict_pair();
    let silent = captured(1, "", "");
    assert!(compare_compile(&human, &silent)
        .unwrap_err()
        .starts_with("rule 1"));
    let human_two = captured(1, "", "error: a\nerror: b\n");
    let one = captured(1, "", &format!("{E0140_RECORD}\n"));
    assert!(compare_compile(&human_two, &one)
        .unwrap_err()
        .starts_with("rule 5"));
}

#[test]
fn rule_5_rejects_an_extra_record() {
    let (human, _) = flag_conflict_pair();
    let two = captured(1, "", &format!("{E0140_RECORD}\n{E0140_RECORD}\n"));
    assert!(compare_compile(&human, &two)
        .unwrap_err()
        .starts_with("rule 5"));
}

// 30.9.26j AC6: a stray stderr line fails the strengthened rule 2.
#[test]
fn a_stray_stderr_line_fails_rule_2() {
    let (human, mut json) = flag_conflict_pair();
    json.stderr.push_str("warning: cannot read directory : x\n");
    assert!(compare_compile(&human, &json)
        .unwrap_err()
        .starts_with("rule 2"));
}

// 30.9.26j AC6: a front-end fixture passes on located records alone — the
// `compilation failed:` restatement needs no record.
#[test]
fn a_front_end_fixture_passes_on_located_records_alone() {
    let human = captured(1, "", TYPE_ERROR_HUMAN);
    let json = captured(1, "", &format!("{TYPE_ERROR_RECORD}\n"));
    assert_eq!(compare_compile(&human, &json), Ok(()));
}

#[test]
fn a_located_mismatch_fails_rule_3() {
    let human = captured(1, "", "[E0011] Error: x\n");
    let json = captured(1, "", &format!("{TYPE_ERROR_RECORD}\n"));
    assert!(compare_compile(&human, &json)
        .unwrap_err()
        .starts_with("rule 3"));
}

#[test]
fn a_located_record_without_a_code_fails_rule_4() {
    let human = captured(1, "", "Error: x\n");
    let record = r#"{"schema":1,"level":"error","code":null,"primary_span":{"file":"f"}}"#;
    let json = captured(1, "", record);
    assert!(compare_compile(&human, &json)
        .unwrap_err()
        .starts_with("rule 4"));
}

#[test]
fn differing_exit_codes_fail_rule_1() {
    let (human, mut json) = flag_conflict_pair();
    json.code = 3;
    assert!(compare_compile(&human, &json)
        .unwrap_err()
        .starts_with("rule 1"));
}

#[test]
fn a_clean_pair_is_accepted() {
    let human = captured(0, "✓ Compiled to x\n", "");
    let json = captured(0, "✓ Compiled to x\n", "");
    assert_eq!(compare_compile(&human, &json), Ok(()));
}

// 30.9.26j AC6: a `json_` fixture is a golden twin, not a pair.
#[test]
fn a_json_fixture_is_not_run_as_a_pair() {
    assert!(!is_pair_fixture(Path::new(
        "compile_error/json_flag_conflict.tg"
    )));
    assert!(is_pair_fixture(Path::new("compile_error/flag_conflict.tg")));
    assert!(is_pair_fixture(Path::new("compile_error/a_json_twin.tg")));
}

#[test]
fn build_coded_counts_only_the_build_block() {
    let json = captured(1, "", &format!("{E0140_RECORD}\n{TYPE_ERROR_RECORD}\n"));
    assert_eq!(build_coded(&json), 1);
    assert_eq!(build_coded(&captured(1, "", "not json")), 0);
    let e0149 = E0140_RECORD.replace("E0140", "E0149");
    let e0150 = E0140_RECORD.replace("E0140", "E0150");
    assert_eq!(
        build_coded(&captured(1, "", &format!("{e0149}\n{e0150}\n"))),
        1
    );
}

// 30.9.26j AC6: the floor fails when no record carries a build code.
#[test]
fn the_floor_fails_on_zero_build_coded_records() {
    assert_eq!(
        compile_tally(5, 0, 0, 0),
        CategoryTally {
            passed: 5,
            failed: 1,
            ..CategoryTally::default()
        }
    );
    assert_eq!(
        compile_tally(5, 1, 3, 0),
        CategoryTally {
            passed: 4,
            failed: 1,
            ..CategoryTally::default()
        }
    );
    assert_eq!(compile_tally(0, 0, 0, 0).failed, 1);
}

#[test]
fn a_codegen_less_run_skips_without_failing() {
    assert_eq!(
        compile_tally(0, 0, 0, 5),
        CategoryTally {
            skipped: 5,
            ..CategoryTally::default()
        }
    );
}

#[test]
fn the_restatement_and_summary_are_not_failure_lines() {
    assert_eq!(failure_lines(TYPE_ERROR_HUMAN), 0);
    assert_eq!(failure_lines(FLAG_CONFLICT_HUMAN), 1);
    assert_eq!(failure_lines("clang: error: linker command failed\n"), 0);
}
