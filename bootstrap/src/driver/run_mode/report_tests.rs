//! Tests for what `run` writes for a runtime outcome in each format
//! (`stopped_report` / `stuck_report`, ADR 29.9.26f D9). Split from
//! `tests.rs`, which holds the stuck-diagnosis tests, to stay under the
//! file-size cap.
//!
//! In JSON mode each arm is a `code: null`, span-less record; in human mode it
//! is the unchanged text.

use albite_core::eval::EvalStopped;

use super::{stopped_note, stopped_report, stuck_report, RunReport};
use crate::driver::diagnostics::{json::Level, ErrorFormat};

fn black_hole() -> EvalStopped {
    EvalStopped::BlackHole {
        cycle: vec!["f".into(), "f".into()],
    }
}

// 29.9.26f AC5: a stop is an `error` record carrying its `Display` and note.
#[test]
fn a_stop_in_json_mode_is_an_uncoded_error_record_with_its_note() {
    let hole = black_hole();
    let RunReport::Record(stop) = stopped_report(&hole, ErrorFormat::Json) else {
        panic!("JSON mode must build a record");
    };
    assert_eq!(
        (stop.level, stop.code, stop.primary_span),
        (Level::Error, None, None)
    );
    assert_eq!(stop.message, hole.to_string());
    assert_eq!(stop.notes.len(), 1);
    assert_eq!(stop.notes[0].message, stopped_note(&hole).unwrap());
}

#[test]
fn a_stop_with_no_note_has_empty_notes() {
    let stop = EvalStopped::StepLimit { limit: 1 };
    let RunReport::Record(record) = stopped_report(&stop, ErrorFormat::Json) else {
        panic!("JSON mode must build a record");
    };
    assert!(record.notes.is_empty(), "{:?}", record.notes);
    let RunReport::Lines(lines) = stopped_report(&stop, ErrorFormat::Human) else {
        panic!("human mode must print lines");
    };
    assert_eq!(lines, [format!("error: {stop}")]);
}

#[test]
fn a_stop_in_human_mode_is_the_unchanged_text() {
    let hole = black_hole();
    let RunReport::Lines(lines) = stopped_report(&hole, ErrorFormat::Human) else {
        panic!("human mode must print lines");
    };
    assert_eq!(
        lines,
        [
            format!("error: {hole}"),
            format!("  = note: {}", stopped_note(&hole).unwrap())
        ]
    );
}

// 29.9.26f AC5: the stuck arm is a `warning` record with the diagnosis as note.
#[test]
fn a_stuck_residual_in_json_mode_is_an_uncoded_warning_record() {
    let RunReport::Record(stuck) = stuck_report("global `g` never resolved", ErrorFormat::Json)
    else {
        panic!("JSON mode must build a record");
    };
    assert_eq!(
        (stuck.level, stuck.code, stuck.primary_span),
        (Level::Warning, None, None)
    );
    assert!(
        stuck.message.starts_with("evaluation got stuck"),
        "{}",
        stuck.message
    );
    assert_eq!(stuck.notes.len(), 1);
    assert_eq!(stuck.notes[0].message, "global `g` never resolved");
}

#[test]
fn a_stuck_residual_in_human_mode_is_the_unchanged_text() {
    let RunReport::Lines(lines) = stuck_report("x", ErrorFormat::Human) else {
        panic!("human mode must print lines");
    };
    assert_eq!(
        lines,
        [
            "warning: evaluation got stuck — the result below is a residual term, not a value"
                .to_string(),
            "  = stuck at: x".to_string(),
        ]
    );
}
