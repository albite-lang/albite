//! The parity comparison, over captured runs (ADR 28.9.26e AC5).

use super::*;

const TYPE_ERROR_RECORD: &str = r#"{"schema":1,"kind":"diagnostic","level":"error","code":"E0010","message":"m","primary_span":{"file":"f.tg"}}"#;
const TYPE_ERROR_HEADER: &str = "\x1b[31m[E0010] Error:\x1b[0m expected `Nat`, found `Bool`\n";

fn captured(code: i32, stdout: &str, stderr: &str) -> Captured {
    Captured {
        code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

fn failing_pair() -> (Captured, Captured) {
    (
        captured(1, "", TYPE_ERROR_HEADER),
        captured(1, &format!("{TYPE_ERROR_RECORD}\n"), ""),
    )
}

#[test]
fn a_matching_pair_is_accepted() {
    let (human, json) = failing_pair();
    assert_eq!(compare(&human, &json), Ok(()));
}

#[test]
fn a_clean_pair_is_accepted() {
    assert_eq!(
        compare(&captured(0, "✓ ok\n", ""), &captured(0, "", "")),
        Ok(())
    );
}

// Condition 1
#[test]
fn a_differing_exit_code_diverges() {
    let (human, mut json) = failing_pair();
    json.code = 0;
    assert!(compare(&human, &json).unwrap_err().contains("exit"));
}

// Condition 1
#[test]
fn a_failing_run_needs_an_error_record() {
    let human = captured(1, "", "[W0001] Warning: unreachable\n");
    let warning = r#"{"schema":1,"level":"warning","code":"W0001","primary_span":{}}"#;
    let json = captured(1, warning, "");
    assert!(compare(&human, &json)
        .unwrap_err()
        .contains("no error record"));
}

// Condition 2
#[test]
fn an_unparseable_line_diverges() {
    let (human, mut json) = failing_pair();
    json.stdout.push_str("not json\n");
    assert!(compare(&human, &json).unwrap_err().contains("not JSON"));
}

// Condition 2
#[test]
fn a_record_without_schema_one_diverges() {
    let (human, mut json) = failing_pair();
    json.stdout = json.stdout.replace("\"schema\":1", "\"schema\":2");
    assert!(compare(&human, &json).unwrap_err().contains("schema"));
}

// Condition 2
#[test]
fn a_human_header_on_json_stderr_diverges() {
    let (human, mut json) = failing_pair();
    json.stderr = TYPE_ERROR_HEADER.to_string();
    assert!(compare(&human, &json)
        .unwrap_err()
        .contains("JSON-mode stderr"));
    json.stderr = "error: aborting due to 1 error\n".to_string();
    assert!(compare(&human, &json)
        .unwrap_err()
        .contains("JSON-mode stderr"));
}

#[test]
fn a_non_header_line_on_json_stderr_is_allowed() {
    let (human, mut json) = failing_pair();
    json.stderr =
        "warning: Signature Collection global collection failed\n  first error: error[E0010]\n"
            .to_string();
    assert_eq!(compare(&human, &json), Ok(()));
}

// Condition 3
#[test]
fn a_differing_code_diverges() {
    let (mut human, json) = failing_pair();
    human.stderr = human.stderr.replace("E0010", "E0001");
    assert!(compare(&human, &json).unwrap_err().contains("human"));
}

// Condition 3
#[test]
fn a_differing_count_diverges() {
    let (mut human, json) = failing_pair();
    human.stderr.push_str(TYPE_ERROR_HEADER);
    assert!(compare(&human, &json).is_err());
}

#[test]
fn a_termination_warning_counts_as_a_warning() {
    assert_eq!(
        header("[E0062] Warning: cannot prove termination"),
        Some(("warning".to_string(), Some("E0062".to_string())))
    );
}

#[test]
fn a_bare_error_header_has_no_code() {
    assert_eq!(
        header("Error: unexpected token"),
        Some(("error".to_string(), None))
    );
}

#[test]
fn non_header_lines_are_ignored() {
    for line in [
        "   │ Help: remove it",
        "error: aborting due to 2 errors",
        "  hint: run `albite explain error E0010`",
        "[E001] Error: too short",
        "[X0010] Error: wrong prefix",
        "[E0010] Note: not a level",
        "",
    ] {
        assert_eq!(header(line), None, "{line:?}");
    }
}

#[test]
fn the_exit_3_rule_accepts_one_location_less_record() {
    let human = captured(3, "", "error: file not found: x.tg\n");
    let record = r#"{"schema":1,"level":"error","code":null,"primary_span":null}"#;
    assert_eq!(compare(&human, &captured(3, record, "")), Ok(()));
}

#[test]
fn the_exit_3_rule_rejects_a_located_or_coded_record() {
    let human = captured(3, "", "error: file not found: x.tg\n");
    let coded = r#"{"schema":1,"level":"error","code":"E0010","primary_span":null}"#;
    let located = r#"{"schema":1,"level":"error","code":null,"primary_span":{}}"#;
    assert!(compare(&human, &captured(3, coded, "")).is_err());
    assert!(compare(&human, &captured(3, located, "")).is_err());
    let two = format!("{located}\n{located}");
    assert!(compare(&human, &captured(3, &two, "")).is_err());
}

#[test]
fn the_exit_3_rule_needs_a_human_failure_line() {
    let human = captured(3, "", "error: aborting due to 1 error\n");
    let record = r#"{"schema":1,"level":"error","code":null,"primary_span":null}"#;
    assert!(compare(&human, &captured(3, record, "")).is_err());
}

#[test]
fn comparing_nothing_fails_the_category() {
    assert!(tally(0, 0, 0, 0).is_failure());
}

#[test]
fn comparing_fewer_than_discovered_fails_the_category() {
    assert!(tally(3, 0, 4, 1).is_failure());
}

#[test]
fn a_full_clean_comparison_passes() {
    let clean = tally(4, 0, 4, 1);
    assert!(!clean.is_failure());
    assert_eq!(clean.passed, 4);
}

#[test]
fn each_divergence_is_a_failure() {
    let half_divergent = tally(4, 2, 4, 1);
    assert_eq!((half_divergent.passed, half_divergent.failed), (2, 2));
}

// ─────────────────────────────────────────────────────────────────────
// Rule 4 (ADR 29.9.26f D7 / AC 4): a located record has a code
// ─────────────────────────────────────────────────────────────────────

const SLICE_1_PARSE_RECORD: &str = r#"{"schema":1,"level":"error","code":null,"message":"invalid pattern","primary_span":{"file":"f.tg"}}"#;
const CODED_PARSE_RECORD: &str = r#"{"schema":1,"level":"error","code":"E0113","message":"invalid pattern","primary_span":{"file":"f.tg"}}"#;

/// The must-fail twin: the slice-1 pair is equal under the multiset rule, so
/// only rule 4 can reject it — and the error names rule 4 (29.9.26f AC4).
#[test]
fn rule_4_rejects_the_slice_1_pair_and_names_itself() {
    let human = captured(1, "", "Error: invalid pattern\n");
    let json = captured(1, &format!("{SLICE_1_PARSE_RECORD}\n"), "");
    let why = compare(&human, &json).unwrap_err();
    assert!(why.contains("rule 4"), "{why}");
    assert!(why.contains("located record without a code"), "{why}");
}

#[test]
fn a_coded_parse_pair_is_accepted() {
    let human = captured(1, "", "[E0113] Error: invalid pattern\n");
    let json = captured(1, &format!("{CODED_PARSE_RECORD}\n"), "");
    assert_eq!(compare(&human, &json), Ok(()));
}

/// Rule 3 still pins that the two renderers print the SAME code
/// (29.9.26f AC4).
#[test]
fn a_parse_code_on_the_json_side_only_diverges() {
    let human = captured(1, "", "Error: invalid pattern\n");
    let json = captured(1, &format!("{CODED_PARSE_RECORD}\n"), "");
    let why = compare(&human, &json).unwrap_err();
    assert!(why.contains("human"), "{why}");
}

/// The exit-3 failure record has no span, so rule 4 exempts it
/// (29.9.26f AC4).
#[test]
fn rule_4_exempts_the_exit_3_record() {
    let human = captured(3, "", "error: file not found: x.tg\n");
    let record = r#"{"schema":1,"level":"error","code":null,"primary_span":null}"#;
    assert_eq!(compare(&human, &captured(3, record, "")), Ok(()));
    // And a span-less, code-less record on a failing run is not rule 4's.
    let (human, _) = failing_pair();
    let runtime = format!("{TYPE_ERROR_RECORD}\n{record}\n");
    let why = compare(&human, &captured(1, &runtime, ""));
    assert!(why.is_err_and(|w| !w.contains("rule 4")));
}

#[test]
fn syntax_coded_counts_only_located_syntax_block_codes() {
    let stdout = format!(
        "{CODED_PARSE_RECORD}\n{TYPE_ERROR_RECORD}\n{SLICE_1_PARSE_RECORD}\n\
         {{\"schema\":1,\"level\":\"error\",\"code\":\"E0117\",\"primary_span\":null}}\n\
         {{\"schema\":1,\"level\":\"error\",\"code\":\"E0118\",\"primary_span\":{{}}}}\n\
         {{\"schema\":1,\"level\":\"error\",\"code\":\"E0110\",\"primary_span\":{{}}}}\n"
    );
    assert_eq!(syntax_coded(&captured(1, &stdout, "")), 2);
    assert_eq!(syntax_coded(&captured(1, "not json\n", "")), 0);
}

// 29.9.26f AC4: the syntax-code vacuity floor.
#[test]
fn the_category_fails_when_no_located_record_carries_a_syntax_code() {
    assert!(tally(4, 0, 4, 0).is_failure());
    assert!(!tally(4, 0, 4, 1).is_failure());
}
