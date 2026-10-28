//! `normalize_durations` (ADR 29.9.26i AC 5) and `normalize_junit_times`
//! (ADR 30.9.26k AC 5).

use super::{normalize_durations, normalize_junit_times};

// 30.9.26k AC5: every `time="<digits>.<digits>"` becomes `time="0.000"`.
#[test]
fn junit_times_become_zero() {
    let input = "<testsuites tests=\"2\" time=\"1.500\">\n  <testcase name=\"a\" time=\"0.012\"/>";
    assert_eq!(
        normalize_junit_times(input),
        "<testsuites tests=\"2\" time=\"0.000\">\n  <testcase name=\"a\" time=\"0.000\"/>"
    );
}

#[test]
fn junit_time_rewrite_is_idempotent() {
    let once = normalize_junit_times("time=\"61.001\"");
    assert_eq!(once, "time=\"0.000\"");
    assert_eq!(normalize_junit_times(&once), once);
}

#[test]
fn junit_times_of_another_shape_are_left_alone() {
    for odd in [
        "time=\"\"",
        "time=\"12\"",
        "time=\"1.\"",
        "time=\".5\"",
        "time=\"1.5s\"",
        "time=\"x\"",
        "time=\"1.5",
    ] {
        assert_eq!(normalize_junit_times(odd), odd, "{odd}");
    }
}

#[test]
fn junit_rewrite_leaves_durations_and_human_text_alone() {
    let json = r#"{"duration_ms":0,"time":3}"#;
    assert_eq!(normalize_junit_times(json), json);
    let human = "result: ok. 3 passed; 0 failed; finished in 1.25s";
    assert_eq!(normalize_junit_times(human), human);
}

// 29.9.26i AC5: `normalize_durations` rewrites only `"duration_ms":<digits>`.
#[test]
fn a_duration_becomes_zero() {
    assert_eq!(
        normalize_durations(r#"{"a":1,"duration_ms":123,"b":45}"#),
        r#"{"a":1,"duration_ms":0,"b":45}"#
    );
}

#[test]
fn every_duration_on_every_line_is_rewritten() {
    let input = "{\"duration_ms\":7}\n{\"x\":9,\"duration_ms\":10500}\n";
    assert_eq!(
        normalize_durations(input),
        "{\"duration_ms\":0}\n{\"x\":9,\"duration_ms\":0}\n"
    );
}

#[test]
fn other_digits_and_human_text_are_left_alone() {
    let human = "result: ok. 3 passed; 0 failed; finished in 1.25s";
    assert_eq!(normalize_durations(human), human);
    let other = r#"{"assertions":12,"total":3,"duration":55}"#;
    assert_eq!(normalize_durations(other), other);
}

#[test]
fn a_key_without_digits_is_left_alone() {
    let odd = r#"{"duration_ms":null,"duration_ms":"x"}"#;
    assert_eq!(normalize_durations(odd), odd);
}

#[test]
fn the_rewrite_is_idempotent() {
    let once = normalize_durations(r#"{"duration_ms":99}"#);
    assert_eq!(normalize_durations(&once), once);
    assert_eq!(once, r#"{"duration_ms":0}"#);
}
