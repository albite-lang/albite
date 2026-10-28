//! Schema-1 builder tests (ADR 28.9.26e AC1).

use super::*;
use crate::error::{ParseErrorKind, Suggestion};
use crate::ElabErrorKind;
use albite_core::Type;
use std::path::PathBuf;

/// The column oracle: its second warning's human header reads `13:26` while
/// its byte column is 29, because `é` and `—` are multi-byte.
const MULTIBYTE: &str = include_str!("../../../../../tests/golden/check/multibyte_before_span.tg");

pub(super) fn resolved<'a>(source: &'a str, filename: &'a str) -> ResolvedSource<'a> {
    ResolvedSource { source, filename }
}

fn located(line_start: u32, col_start: u32, line_end: u32, col_end: u32) -> [Option<u32>; 4] {
    [
        Some(line_start),
        Some(col_start),
        Some(line_end),
        Some(col_end),
    ]
}

fn lines_of(span: &SpanRecord) -> [Option<u32>; 4] {
    [span.line_start, span.col_start, span.line_end, span.col_end]
}

pub(super) fn mismatch(start: u32, end: u32) -> ElabError {
    ElabError::type_mismatch(Span::new(start, end), Type::Nat, Type::Bool)
}

/// `StrMap<CtorBucket>` resolved against unresolved: the two render
/// identically, so the error carries a structural-divergence note.
fn identically_rendering_mismatch(start: u32, end: u32) -> ElabError {
    let resolved_map = Type::App(
        "StrMap".to_string(),
        vec![Type::Adt(
            "CtorBucket".to_string(),
            vec![],
            vec![("Bucket".to_string(), Type::Nat)],
        )],
    );
    let unresolved_map = Type::App(
        "StrMap".to_string(),
        vec![Type::App("CtorBucket".to_string(), vec![])],
    );
    ElabError::type_mismatch(Span::new(start, end), resolved_map, unresolved_map)
}

// ─────────────────────────────────────────────────────────────────────
// elab_record
// ─────────────────────────────────────────────────────────────────────

#[test]
fn an_elab_record_carries_every_schema_field() {
    let source = "fn f() -> Nat {\n    true\n}\n";
    let error = identically_rendering_mismatch(20, 24)
        .with_note("first note")
        .with_span_note(Span::new(10, 13), "declared here")
        .with_help("change the return type")
        .with_file_path(PathBuf::from("lib/helper.tg"));
    let record = elab_record(
        &error,
        Level::Error,
        &resolved(source, "lib/helper.tg"),
        None,
    );

    assert_eq!(record.schema, 1);
    assert_eq!(record.kind, "diagnostic");
    assert_eq!(record.level, Level::Error);
    assert_eq!(record.code.as_deref(), Some("E0010"));
    assert_eq!(record.message, format_error_message(&error));
    let span = record.primary_span.expect("an in-source span is located");
    assert_eq!(span.location.file, "lib/helper.tg");
    assert_eq!((span.location.byte_start, span.location.byte_end), (20, 24));
    assert_eq!(lines_of(&span.location), located(2, 5, 2, 9));
    assert_eq!(span.label, format_primary_label(&error));
    assert_eq!(span.text.as_deref(), Some("    true"));

    let messages: Vec<&str> = record.notes.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert_eq!(&messages[..2], &["first note", "declared here"]);
    assert!(
        messages[2].contains("type argument #1 of `StrMap`"),
        "the divergence note comes last: {messages:?}"
    );
    assert_eq!(record.notes[0].span, None);
    let spanned = record.notes[1].span.as_ref().expect("a spanned note");
    assert_eq!(spanned.file, "lib/helper.tg");
    assert_eq!(lines_of(spanned), located(1, 11, 1, 14));
    assert_eq!(record.notes[2].span, None);
    assert_eq!(record.help, vec!["change the return type".to_string()]);
    assert!(!record.diagnostic_hints.is_empty());
    assert!(record.diagnostic_hints[0].command.contains("lib/helper.tg"));
}

#[test]
fn columns_count_characters_not_bytes() {
    let offset = MULTIBYTE
        .find("1 => \"x\"")
        .expect("the oracle's duplicate arm") as u32;
    let error = ElabError::new(Span::new(offset, offset + 1), ElabErrorKind::UnreachableArm);
    let record = elab_record(&error, Level::Warning, &resolved(MULTIBYTE, "m.tg"), None);
    let span = record.primary_span.unwrap();
    let line_start = MULTIBYTE[..offset as usize].rfind('\n').unwrap() + 1;
    assert_eq!(offset as usize - line_start + 1, 29, "the byte column");
    assert_eq!(lines_of(&span.location), located(13, 26, 13, 27));
    assert_eq!(record.level, Level::Warning);
    assert!(
        record.diagnostic_hints.is_empty(),
        "the human mode prints no hints for a warning"
    );
}

#[test]
fn an_empty_span_ends_where_it_starts() {
    let record = elab_record(
        &mismatch(3, 3),
        Level::Error,
        &resolved("abcdef", "t.tg"),
        None,
    );
    let span = record.primary_span.unwrap();
    assert_eq!(lines_of(&span.location), located(1, 4, 1, 4));
}

#[test]
fn an_end_just_after_a_newline_is_column_one_of_the_next_line() {
    let source = "abc\ndef\n";
    let record = elab_record(
        &mismatch(0, 4),
        Level::Error,
        &resolved(source, "t.tg"),
        None,
    );
    let span = record.primary_span.unwrap();
    assert_eq!(lines_of(&span.location), located(1, 1, 2, 1));
    assert_eq!(
        span.text.as_deref(),
        Some("abc"),
        "stops at the line before"
    );
}

#[test]
fn a_multi_line_span_carries_every_covered_line() {
    let source = "abc\ndef\nghi";
    let record = elab_record(
        &mismatch(1, 6),
        Level::Error,
        &resolved(source, "t.tg"),
        None,
    );
    let span = record.primary_span.unwrap();
    assert_eq!(lines_of(&span.location), located(1, 2, 2, 3));
    assert_eq!(span.text.as_deref(), Some("abc\ndef"));
}

#[test]
fn an_other_error_keeps_its_e9999() {
    let error = ElabError::new(
        Span::new(0, 1),
        ElabErrorKind::Other("something".to_string()),
    );
    let record = elab_record(&error, Level::Error, &resolved("x", "t.tg"), None);
    assert_eq!(record.code.as_deref(), Some("E9999"));
}

#[test]
fn an_out_of_bounds_span_keeps_file_and_bytes_and_drops_lines() {
    let record = elab_record(
        &mismatch(40, 50),
        Level::Error,
        &resolved("short", "t.tg"),
        None,
    );
    let span = record.primary_span.expect("still a primary span");
    assert_eq!(span.location.file, "t.tg");
    assert_eq!((span.location.byte_start, span.location.byte_end), (40, 50));
    assert_eq!(lines_of(&span.location), [None; 4]);
    assert_eq!(span.text, None);
}

#[test]
fn a_span_starting_at_end_of_source_takes_the_human_fallback() {
    // The human renderer hands `start >= len` to its plain-text fallback, so
    // JSON reports no lines for it either.
    let record = elab_record(
        &mismatch(5, 5),
        Level::Error,
        &resolved("short", "t.tg"),
        None,
    );
    assert_eq!(lines_of(&record.primary_span.unwrap().location), [None; 4]);
}

#[test]
fn a_span_ending_exactly_at_end_of_source_is_located() {
    let record = elab_record(
        &mismatch(2, 5),
        Level::Error,
        &resolved("short", "t.tg"),
        None,
    );
    let span = record.primary_span.unwrap();
    assert_eq!(lines_of(&span.location), located(1, 3, 1, 6));
    assert_eq!(span.text.as_deref(), Some("short"));
}

#[test]
fn a_span_running_past_end_of_source_takes_the_human_fallback() {
    let record = elab_record(
        &mismatch(2, 6),
        Level::Error,
        &resolved("short", "t.tg"),
        None,
    );
    assert_eq!(lines_of(&record.primary_span.unwrap().location), [None; 4]);
}

#[test]
fn an_offset_inside_a_multi_byte_character_has_no_location() {
    // `é` occupies bytes 1..3, so offset 2 is inside it.
    let record = elab_record(
        &mismatch(2, 4),
        Level::Error,
        &resolved("aéz!", "t.tg"),
        None,
    );
    let span = record.primary_span.unwrap();
    assert_eq!(lines_of(&span.location), [None; 4]);
    assert_eq!((span.location.byte_start, span.location.byte_end), (2, 4));
}

#[test]
fn an_empty_span_at_column_one_keeps_its_own_line() {
    // The exclusive-end rule steps back a line only when the span has
    // extent; an empty span at a line start covers that line.
    let record = elab_record(
        &mismatch(4, 4),
        Level::Error,
        &resolved("abc\ndef", "t.tg"),
        None,
    );
    let span = record.primary_span.unwrap();
    assert_eq!(lines_of(&span.location), located(2, 1, 2, 1));
    assert_eq!(span.text.as_deref(), Some("def"));
}

#[test]
fn a_cross_file_note_locates_against_its_own_file() {
    let mut source_map = SourceMap::new();
    source_map.insert(PathBuf::from("other.tg"), "zero\none two".to_string());
    let error = mismatch(0, 1).with_cross_file_note(
        Span::new(9, 12),
        PathBuf::from("other.tg"),
        "defined here",
    );
    let resolved_main = resolved("main source", "main.tg");

    let with_map = elab_record(&error, Level::Error, &resolved_main, Some(&source_map));
    let note_span = with_map.notes[0].span.as_ref().unwrap();
    assert_eq!(note_span.file, "other.tg");
    assert_eq!(lines_of(note_span), located(2, 5, 2, 8));

    let without_map = elab_record(&error, Level::Error, &resolved_main, None);
    let note_span = without_map.notes[0].span.as_ref().unwrap();
    assert_eq!(note_span.file, "other.tg");
    assert_eq!((note_span.byte_start, note_span.byte_end), (9, 12));
    assert_eq!(lines_of(note_span), [None; 4]);
}

// ─────────────────────────────────────────────────────────────────────
// parse_record / failure_record
// ─────────────────────────────────────────────────────────────────────

// ADR 29.9.26f AC3: the record carries the kind's syntax-block code (it was
// `null` under 28.9.26e D7).
#[test]
fn a_parse_error_carries_its_kinds_code_and_its_message_as_label() {
    let error = ParseError::new(
        Span::new(3, 6),
        ParseErrorKind::UnexpectedToken("foo".to_string()),
    )
    .with_suggestion(Suggestion::new(Span::new(3, 6), ";", "insert a semicolon"))
    .with_suggestion(Suggestion::new(Span::new(3, 6), "", "delete it"));
    let record = parse_record(&error, &resolved("ab foo cd", "p.tg"));
    assert_eq!(record.code.as_deref(), Some("E0110"));
    let lexical = ParseError::new(Span::new(0, 1), ParseErrorKind::Lexical("bad".into()));
    let lexical_record = parse_record(&lexical, &resolved("ab", "p.tg"));
    assert_eq!(lexical_record.code.as_deref(), Some("E0117"));
    assert_eq!(record.level, Level::Error);
    assert_eq!(record.message, "unexpected token `foo`");
    let span = record.primary_span.unwrap();
    assert_eq!(span.label, record.message);
    assert_eq!(span.location.file, "p.tg");
    assert_eq!(lines_of(&span.location), located(1, 4, 1, 7));
    assert_eq!(span.text.as_deref(), Some("ab foo cd"));
    assert_eq!(
        record.help,
        vec!["insert a semicolon".to_string(), "delete it".to_string()]
    );
    assert!(record.diagnostic_hints.is_empty());
}

#[test]
fn a_failure_record_has_no_code_and_no_location() {
    let record = failure_record("could not read x.tg");
    assert_eq!(record.code, None);
    assert_eq!(record.primary_span, None);
    assert_eq!(record.level, Level::Error);
    assert_eq!(record.message, "could not read x.tg");
    assert_eq!(record.schema, SCHEMA_VERSION);
}

// ADR 29.9.26f D9: `run`'s runtime outcomes are uncatalogued — code and span
// null — and differ from each other only in level and notes.
#[test]
fn runtime_records_have_a_null_code_and_no_location() {
    let stopped = stopped_record("black hole: f -> f", Some("re-enters itself"));
    assert_eq!((stopped.level, stopped.code.clone()), (Level::Error, None));
    assert_eq!(stopped.primary_span, None);
    assert_eq!(stopped.message, "black hole: f -> f");
    assert_eq!(
        stopped.notes,
        vec![NoteRecord {
            message: "re-enters itself".into(),
            span: None
        }]
    );
    assert!(stopped_record("step limit", None).notes.is_empty());

    let stuck = stuck_record("evaluation got stuck", "global `g` never resolved");
    assert_eq!((stuck.level, stuck.code.clone()), (Level::Warning, None));
    assert_eq!(stuck.primary_span, None);
    assert_eq!(stuck.message, "evaluation got stuck");
    assert_eq!(stuck.notes.len(), 1);
    assert_eq!(stuck.notes[0].message, "global `g` never resolved");
    assert_eq!(stuck.schema, SCHEMA_VERSION);
}
