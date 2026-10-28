//! Tests for the source map's companion split (ADR 2.10.26j D4, D6):
//! relocating a combined parse's locations, and cache eligibility.
//!
//! Split from `companion.rs` for the 400-LOC file limit, along the seam
//! that means something: everything here reads one injected `SourceMap`.

use std::path::{Path, PathBuf};

use crate::driver::modules::companion::cache_eligible;
use crate::driver::modules::{relocate_error, CompanionSplit, SourceMap};
use crate::elaborate::{ElabError, ElabErrorKind, ExpectedContext, ExpectedReason};
use crate::span::Span;

fn split_map() -> SourceMap {
    let mut map = SourceMap::single(PathBuf::from("dir/main.tg"), "0123456789".to_string());
    map.insert(PathBuf::from("dir/main_test.tg"), "abcdef".to_string());
    map.companions.push(CompanionSplit {
        sibling: PathBuf::from("dir/main.tg"),
        base: 11,
        companion: PathBuf::from("dir/main_test.tg"),
    });
    map
}

// ADR 2.10.26j AC6: a span at or past `base` moves to the companion, less `base`.
#[test]
fn relocate_moves_a_span_past_base_to_the_companion() {
    let map = split_map();
    let (file, span) = map.relocate(Path::new("dir/main.tg"), Span::new(13, 16));
    assert_eq!(file, PathBuf::from("dir/main_test.tg"));
    assert_eq!(span, Span::new(2, 5));
    let (file, span) = map.relocate(Path::new("dir/main.tg"), Span::new(11, 11));
    assert_eq!(
        (file, span),
        (PathBuf::from("dir/main_test.tg"), Span::new(0, 0))
    );
}

/// `relocate` returns `(file, span)` exactly as given.
fn assert_unmoved(file: &str, span: Span) {
    let moved = split_map().relocate(Path::new(file), span);
    assert_eq!(moved, (PathBuf::from(file), span));
}

// ADR 2.10.26j AC6: a span before `base` stays on the sibling.
#[test]
fn relocate_leaves_a_sibling_span_alone() {
    assert_unmoved("dir/main.tg", Span::new(3, 10));
    assert_unmoved("dir/main.tg", Span::new(10, 10));
}

// ADR 2.10.26j AC6: a file with no companion is never moved.
#[test]
fn relocate_leaves_an_unpaired_file_alone() {
    assert_unmoved("dir/other.tg", Span::new(40, 50));
}

// ADR 2.10.26j AC6 (D4): every location an error carries is rewritten — the
// primary, a file-less note on either side, a cross-file note, the context
// and every trace frame.
#[test]
fn relocate_error_rewrites_notes_context_and_trace() {
    let map = split_map();
    let sibling = PathBuf::from("dir/main.tg");
    let companion = PathBuf::from("dir/main_test.tg");
    let mut error = ElabError::new(
        Span::new(12, 14),
        ElabErrorKind::DuplicateDefinition("x".into()),
    )
    .with_file_path(&sibling)
    .with_span_note(Span::new(2, 4), "on the sibling")
    .with_span_note(Span::new(15, 16), "in the companion")
    .with_cross_file_note(
        Span::new(20, 21),
        PathBuf::from("dir/other.tg"),
        "elsewhere",
    )
    .with_trace_frame(Span::new(11, 12), sibling.clone(), "frame")
    .with_context(ExpectedContext::new(
        ExpectedReason::ReturnType,
        Span::new(1, 2),
    ));
    relocate_error(&mut error, &map);

    assert_eq!(error.file_path.as_ref(), Some(&companion));
    assert_eq!(error.span, Span::new(1, 3));
    // A sibling-side note now names the sibling, which is no longer primary.
    assert_eq!(error.notes[0].file_path.as_ref(), Some(&sibling));
    assert_eq!(error.notes[0].span, Some(Span::new(2, 4)));
    // A companion-side note follows the primary and keeps no file of its own.
    assert_eq!(error.notes[1].file_path, None);
    assert_eq!(error.notes[1].span, Some(Span::new(4, 5)));
    // A cross-file note to an unpaired file is untouched.
    assert_eq!(
        error.notes[2].file_path,
        Some(PathBuf::from("dir/other.tg"))
    );
    assert_eq!(error.notes[2].span, Some(Span::new(20, 21)));
    let context = error.context.as_ref().unwrap();
    assert_eq!(
        (context.file_path.as_ref(), context.span),
        (Some(&sibling), Span::new(1, 2))
    );
    assert_eq!(error.trace[0].file_path, companion);
    assert_eq!(error.trace[0].span, Span::new(0, 1));
}

// ADR 2.10.26j AC6 (D4): with no companion attached, nothing changes.
#[test]
fn relocate_error_is_the_identity_without_companions() {
    let map = SourceMap::single(PathBuf::from("m.tg"), "x".repeat(40));
    let mut error = ElabError::new(
        Span::new(30, 32),
        ElabErrorKind::DuplicateDefinition("x".into()),
    )
    .with_span_note(Span::new(31, 32), "n");
    relocate_error(&mut error, &map);
    assert_eq!(error.file_path, None);
    assert_eq!(error.span, Span::new(30, 32));
    assert_eq!(error.notes[0].file_path, None);
}

// ADR 2.10.26j AC7 (D6): an attached sibling is never cached.
#[test]
fn cache_eligible_is_false_for_an_attached_sibling() {
    assert!(!cache_eligible(Path::new("dir/main.tg"), &split_map()));
}

// ADR 2.10.26j AC7 (D6): every other module is, the companion's path included.
#[test]
fn cache_eligible_is_true_otherwise() {
    let map = split_map();
    assert!(cache_eligible(Path::new("dir/other.tg"), &map));
    assert!(cache_eligible(Path::new("dir/main_test.tg"), &map));
    assert!(cache_eligible(Path::new("dir/main.tg"), &SourceMap::new()));
}
