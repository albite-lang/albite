//! Schema-1 JSON diagnostic records for `check --error-format json`
//! (ADR 28.9.26e).
//!
//! One record per diagnostic, serialised as one line of JSONL. Everything
//! here is pure — the builders take the diagnostic and its resolved source and
//! return a value; `rendering::json_records` decides *which* diagnostics, and
//! the render entry points own the only `println!`. The field table and the
//! versioning rule (D9) are in the ADR; the short form is: consumers ignore
//! unknown fields, additions are not a bump, removals and renames are.
//!
//! # The kinds registry (ADR 29.9.26i D6)
//!
//! Every record any command writes carries `schema` and `kind`; everything
//! else is per kind. Schema 1's kinds, and their producers:
//!
//! | `kind` | Producer | Builder |
//! |---|---|---|
//! | [`KIND_DIAGNOSTIC`] | `check --error-format json`, `run --error-format json`, `compile --error-format json` (ADR 30.9.26j, on stderr), `test --format json` | this module; `compile`'s build failures via `compile_failure` |
//! | [`KIND_TEST`] | `test --format json`, one per test | `test_runner::json::test_record` (binary crate) |
//! | [`KIND_TEST_SUMMARY`] | `test --format json`, once, last | `test_runner::json::summary_record` |
//!
//! A new kind is additive under D9: add its constant here, beside
//! [`SCHEMA_VERSION`], so the registry stays in one place.

use super::hints::select_hints;
use super::{format_error_message, format_primary_label, type_mismatch_divergence_note};
use crate::driver::modules::SourceMap;
use crate::span::{LineIndex, Span};
use crate::{ElabError, Note, ParseError};
use serde::Serialize;
use std::path::Path;

/// The value of every record's `schema` field. See D9 before changing it.
pub const SCHEMA_VERSION: u32 = 1;

/// A parse, elaboration, pipeline or runtime diagnostic.
pub const KIND_DIAGNOSTIC: &str = "diagnostic";

/// One test's verdict (ADR 29.9.26i). Coverage records (T5) will join here.
pub const KIND_TEST: &str = "test";

/// A test run's totals. Not `summary`, so a later summary from another
/// producer cannot collide with it.
pub const KIND_TEST_SUMMARY: &str = "test_summary";

/// Which list a diagnostic was rendered from. Deliberately not derived from
/// the code's prefix: `--termination report` renders E-codes as warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
    /// Non-fatal and never counted: a package plan's legacy-manifest notes
    /// (ADR 20.10.26i D2).
    Note,
}

/// A span's location. Line/column fields are `None` when the span does not lie
/// inside the source it was resolved against; the byte offsets always survive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpanRecord {
    pub file: String,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line_start: Option<u32>,
    pub col_start: Option<u32>,
    pub line_end: Option<u32>,
    pub col_end: Option<u32>,
}

/// The primary span: a [`SpanRecord`] plus its label and the source it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrimarySpanRecord {
    #[serde(flatten)]
    pub location: SpanRecord,
    pub label: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteRecord {
    pub message: String,
    pub span: Option<SpanRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HintRecord {
    pub command: String,
    pub reason: String,
}

/// One schema-1 diagnostic. Field order is serde's declaration order, which
/// the goldens pin but D9 does not make contractual.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticRecord {
    pub schema: u32,
    pub kind: &'static str,
    pub level: Level,
    pub code: Option<String>,
    pub message: String,
    pub primary_span: Option<PrimarySpanRecord>,
    pub notes: Vec<NoteRecord>,
    pub help: Vec<String>,
    pub diagnostic_hints: Vec<HintRecord>,
}

/// A diagnostic's source as the human renderer resolved it: the text its span
/// indexes into, and the file name its header prints (D12).
pub struct ResolvedSource<'a> {
    pub source: &'a str,
    pub filename: &'a str,
}

/// 1-based line and 1-based *character* column of a byte offset (D8), or
/// `None` when the offset is past the end or not on a character boundary —
/// `is_char_boundary` is false for both, so one test covers them.
fn line_and_char_column(source: &str, index: &LineIndex, offset: u32) -> Option<(u32, u32)> {
    let at = offset as usize;
    if !source.is_char_boundary(at) {
        return None;
    }
    let location = index.location(offset);
    let line_start = at - (location.column as usize - 1);
    let column = source[line_start..at].chars().count() as u32 + 1;
    Some((location.line, column))
}

/// The location half of a span record, against `source`. The end is exclusive
/// (D8), so an end just after a `\n` is column 1 of the next line.
fn locate(file: &str, span: Span, source: Option<&str>) -> SpanRecord {
    let located = source.filter(|_| span.start <= span.end).and_then(|text| {
        let index = LineIndex::new(text);
        let start = line_and_char_column(text, &index, span.start)?;
        let end = line_and_char_column(text, &index, span.end)?;
        Some((start, end))
    });
    SpanRecord {
        file: file.to_string(),
        byte_start: span.start,
        byte_end: span.end,
        line_start: located.map(|(start, _)| start.0),
        col_start: located.map(|(start, _)| start.1),
        line_end: located.map(|(_, end)| end.0),
        col_end: located.map(|(_, end)| end.1),
    }
}

/// The source lines a located span covers, verbatim and without the trailing
/// newline. An exclusive end at column 1 of a later line stops at the line
/// before it, since that line contributes no character.
fn covered_text(source: &str, location: &SpanRecord) -> Option<String> {
    let (first, mut last) = (location.line_start?, location.line_end?);
    if location.col_end == Some(1) && last > first {
        last -= 1;
    }
    let lines: Vec<&str> = source.split('\n').collect();
    let covered = lines.get(first as usize - 1..last as usize)?;
    Some(covered.join("\n"))
}

/// The primary span against `source`, or with null lines when `source` is
/// `None` (a span the caller has already judged outside it).
fn primary_span(file: &str, span: Span, source: Option<&str>, label: String) -> PrimarySpanRecord {
    let location = locate(file, span, source);
    let text = source.and_then(|text| covered_text(text, &location));
    PrimarySpanRecord {
        location,
        label,
        text,
    }
}

/// A note's span: its own file for a cross-file note, else the error's. A
/// cross-file note locates against `source_map`; with none (the single-file
/// entry point) its line/column fields are `null`.
fn note_record(
    note: &Note,
    resolved: &ResolvedSource<'_>,
    source_map: Option<&SourceMap>,
) -> NoteRecord {
    let span = note.span.map(|span| match &note.file_path {
        Some(path) => locate(
            &path.display().to_string(),
            span,
            source_map.and_then(|map| map.get(path)),
        ),
        None => locate(resolved.filename, span, Some(resolved.source)),
    });
    NoteRecord {
        message: note.message.clone(),
        span,
    }
}

/// The human renderer's fallback predicate: a span it cannot hand to ariadne.
/// Such an error keeps its file and byte offsets and loses its lines (§2.2).
fn outside_source(span: Span, source: &str) -> bool {
    let (start, end) = (span.start as usize, span.end as usize);
    start >= source.len() || end > source.len() || start > end
}

/// Build the record for an elaboration error or warning.
///
/// Notes come in the human order: `error.notes`, then the structural-divergence
/// note. Hints are always computed for an error, independent of `--hints`
/// (D6), against the resolved filename exactly as the human renderer calls
/// `select_hints`; a warning gets none, because the human mode prints none.
pub fn elab_record(
    error: &ElabError,
    level: Level,
    resolved: &ResolvedSource<'_>,
    source_map: Option<&SourceMap>,
) -> DiagnosticRecord {
    let source = (!outside_source(error.span, resolved.source)).then_some(resolved.source);
    let span = primary_span(
        resolved.filename,
        error.span,
        source,
        format_primary_label(error),
    );
    let mut notes: Vec<NoteRecord> = error
        .notes
        .iter()
        .map(|note| note_record(note, resolved, source_map))
        .collect();
    if let Some(divergence) = type_mismatch_divergence_note(error) {
        notes.push(NoteRecord {
            message: divergence,
            span: None,
        });
    }
    let diagnostic_hints = match level {
        Level::Error => select_hints(&error.kind, Some(Path::new(resolved.filename)))
            .into_iter()
            .map(|hint| HintRecord {
                command: hint.command,
                reason: hint.reason,
            })
            .collect(),
        Level::Warning | Level::Note => Vec::new(),
    };
    DiagnosticRecord {
        schema: SCHEMA_VERSION,
        kind: KIND_DIAGNOSTIC,
        level,
        code: Some(error.kind.code().to_string()),
        message: format_error_message(error),
        primary_span: Some(span),
        notes,
        help: error.help.iter().cloned().collect(),
        diagnostic_hints,
    }
}

/// Build the record for a parse error: its kind's syntax-block code
/// (ADR 29.9.26f D5, replacing 28.9.26e D7's `null`), its message as the
/// label, its suggestion messages as `help`.
pub fn parse_record(error: &ParseError, resolved: &ResolvedSource<'_>) -> DiagnosticRecord {
    let message = error.to_diagnostic().message;
    DiagnosticRecord {
        schema: SCHEMA_VERSION,
        kind: KIND_DIAGNOSTIC,
        level: Level::Error,
        code: Some(error.kind.code().to_string()),
        primary_span: Some(primary_span(
            resolved.filename,
            error.span,
            Some(resolved.source),
            message.clone(),
        )),
        message,
        notes: Vec::new(),
        help: error
            .suggestions
            .iter()
            .map(|suggestion| suggestion.message.clone())
            .collect(),
        diagnostic_hints: Vec::new(),
    }
}

/// Build the record for a pipeline or I/O failure — no code, no location.
pub fn failure_record(message: &str) -> DiagnosticRecord {
    DiagnosticRecord {
        schema: SCHEMA_VERSION,
        kind: KIND_DIAGNOSTIC,
        level: Level::Error,
        code: None,
        message: message.to_string(),
        primary_span: None,
        notes: Vec::new(),
        help: Vec::new(),
        diagnostic_hints: Vec::new(),
    }
}

/// Build the record for a `run` whose evaluation stopped (ADR 29.9.26f D9):
/// an uncatalogued runtime condition, so `code: null` and no span, with the
/// stop's `Display` as the message and its note, when it has one.
pub fn stopped_record(message: &str, note: Option<&str>) -> DiagnosticRecord {
    DiagnosticRecord {
        notes: note.map(unlocated_note).into_iter().collect(),
        ..failure_record(message)
    }
}

/// Build the warning record for a `run` whose result is stuck (ADR 29.9.26f
/// D9): `code: null`, no span, the `stuck at:` diagnosis as its note.
pub fn stuck_record(message: &str, diagnosis: &str) -> DiagnosticRecord {
    DiagnosticRecord {
        level: Level::Warning,
        notes: vec![unlocated_note(diagnosis)],
        ..failure_record(message)
    }
}

fn unlocated_note(message: &str) -> NoteRecord {
    NoteRecord {
        message: message.to_string(),
        span: None,
    }
}

/// Serialise one record — of any kind — as one JSONL line, without the
/// terminating newline. `serde_json` escapes control characters, so the line
/// has no interior `\n`.
pub fn to_line<R: Serialize>(record: &R) -> String {
    serde_json::to_string(record).expect("a schema-1 record always serialises")
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_serialisation;
