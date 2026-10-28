//! Serialisation tests, and the tests moved from `hints/tests_helpers.rs`
//! under their existing names (ADR 28.9.26e §2.3). Split from `tests.rs`,
//! which holds the builder tests, to stay under the file-size cap.

use super::tests::{mismatch, resolved};
use super::*;
use crate::ElabErrorKind;
use albite_core::Type;
use std::path::PathBuf;

// ─────────────────────────────────────────────────────────────────────
// to_line
// ─────────────────────────────────────────────────────────────────────

#[test]
fn to_line_is_one_line_even_for_a_multi_line_message() {
    let line = to_line(&failure_record("first\nsecond"));
    assert!(!line.contains('\n'), "{line}");
    let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed["message"], "first\nsecond");
}

/// 29.9.26i AC 3 / D6: the kinds registry's three values are distinct and
/// stable — a rename is a schema bump under D9, so this pins the spelling.
#[test]
fn the_three_record_kinds_are_distinct_and_stable() {
    assert_eq!(KIND_DIAGNOSTIC, "diagnostic");
    assert_eq!(KIND_TEST, "test");
    assert_eq!(KIND_TEST_SUMMARY, "test_summary");
    assert_eq!(SCHEMA_VERSION, 1);
}

// ─────────────────────────────────────────────────────────────────────
// Moved from hints/tests_helpers.rs (ADR 28.9.26e §2.3), same names,
// rewritten against schema 1 so the test-name ratchet loses no key.
// ─────────────────────────────────────────────────────────────────────

fn line_of(source: &str, offset: u32) -> Option<u32> {
    line_and_char_column(source, &LineIndex::new(source), offset).map(|(line, _)| line)
}

#[test]
fn test_compute_line_number_first_line() {
    assert_eq!(line_of("hello\nworld\n", 0), Some(1));
}

#[test]
fn test_compute_line_number_second_line() {
    assert_eq!(line_of("hello\nworld\n", 6), Some(2));
}

#[test]
fn test_compute_line_number_past_end_clamps() {
    // Past the end is no location at all now, rather than the last line.
    assert_eq!(line_of("hello\n", 999), None);
    assert_eq!(line_of("hello\n", 6), Some(2));
}

#[test]
fn test_compute_line_number_empty_source() {
    assert_eq!(line_of("", 0), Some(1));
}

#[test]
fn test_error_to_json_structure() {
    // "fn main() -> X {\n    Cons(1, Nil)\n}\n" — offset 15 is on line 1.
    let error = ElabError::new(
        Span { start: 15, end: 22 },
        ElabErrorKind::TypeMismatch {
            expected: Type::Adt("List".to_string(), vec![Type::Nat], vec![]),
            found: Type::Bool,
        },
    )
    .with_file_path(PathBuf::from("examples/list.tg"));
    let source = "fn main() -> X {\n    Cons(1, Nil)\n}\n";
    let record = elab_record(
        &error,
        Level::Error,
        &resolved(source, "examples/list.tg"),
        None,
    );

    assert_eq!(record.code.as_deref(), Some("E0010"));
    let span = record.primary_span.unwrap();
    assert_eq!(span.location.file, "examples/list.tg");
    assert_eq!(span.location.line_start, Some(1));
    assert!(!record.diagnostic_hints.is_empty());
    assert!(record.diagnostic_hints[0].command.contains("List"));
    assert!(!record.diagnostic_hints[0].reason.is_empty());
}

#[test]
fn test_json_diagnostic_report_round_trip() {
    let source = "let a = true\nlet b = x\n";
    let errors = [
        mismatch(0, 5).with_file_path(PathBuf::from("test.tg")),
        ElabError::new(
            Span { start: 21, end: 22 },
            ElabErrorKind::UndefinedVariable("x".to_string()),
        ),
    ];
    let lines: Vec<String> = errors
        .iter()
        .map(|e| {
            to_line(&elab_record(
                e,
                Level::Error,
                &resolved(source, "test.tg"),
                None,
            ))
        })
        .collect();

    let parsed: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect();
    assert_eq!(parsed[0]["schema"], 1);
    assert_eq!(parsed[0]["kind"], "diagnostic");
    assert_eq!(parsed[0]["level"], "error");
    assert_eq!(parsed[0]["code"], "E0010");
    assert_eq!(parsed[0]["primary_span"]["file"], "test.tg");
    assert_eq!(parsed[0]["primary_span"]["line_start"], 1);
    assert_eq!(parsed[1]["code"], "E0001");
    assert_eq!(parsed[1]["primary_span"]["line_start"], 2);
    assert_eq!(parsed[1]["primary_span"]["col_start"], 9);
    let hint = &parsed[0]["diagnostic_hints"][0];
    assert!(hint["command"].is_string());
    assert!(hint["reason"].is_string());
    assert!(parsed[0]["notes"].is_array());
    assert!(parsed[0]["help"].is_array());
}

// ─────────────────────────────────────────────────────────────────────
// the record stream (ADR 29.9.26f D10)
// ─────────────────────────────────────────────────────────────────────

/// The JSON arm's one print site writes each record, as one line, to the
/// selected stream and to nothing else.
#[test]
fn a_record_goes_to_the_selected_stream_only() {
    use crate::driver::diagnostics::{write_record, RecordStream};
    let record = failure_record("m");
    for (stream, to_stdout) in [(RecordStream::Stdout, true), (RecordStream::Stderr, false)] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        write_record(&record, stream, &mut out, &mut err);
        let (hit, miss) = if to_stdout { (out, err) } else { (err, out) };
        assert_eq!(
            String::from_utf8(hit).unwrap(),
            format!("{}\n", to_line(&record))
        );
        assert!(miss.is_empty(), "{stream:?} leaked to the other stream");
    }
    assert_eq!(RecordStream::default(), RecordStream::Stdout);
}
