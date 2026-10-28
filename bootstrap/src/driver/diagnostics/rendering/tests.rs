//! Tests for the diagnostic rendering pipeline — split from `mod.rs`
//! (ADR 14.8.26g: D7's truncation tests took the file over the size limit).

use super::*;
use crate::elaborate::ElabErrorKind;
use crate::span::Span;

fn elab_error(start: u32, end: u32) -> ElabError {
    ElabError::new(
        Span::new(start, end),
        ElabErrorKind::UndefinedVariable("x".to_string()),
    )
}

fn parse_err(start: u32, end: u32) -> ParseError {
    ParseError::new(
        Span::new(start, end),
        crate::error::ParseErrorKind::UnexpectedEof,
    )
}

fn src() -> SourceRef<'static> {
    SourceRef {
        source: "fn f() -> Nat { x }",
        filename: "t.tg",
    }
}

// ─────────────────────────────────────────────────────────────────────
// display_budget
// ─────────────────────────────────────────────────────────────────────

#[test]
fn zero_max_errors_means_no_limit() {
    assert_eq!(display_budget(7, 0), (7, 0));
}

#[test]
fn a_limit_below_the_total_withholds_the_remainder() {
    assert_eq!(display_budget(7, 3), (3, 4));
}

#[test]
fn a_limit_above_the_total_withholds_nothing() {
    assert_eq!(display_budget(2, 10), (10, 0));
}

#[test]
fn a_limit_equal_to_the_total_withholds_nothing() {
    assert_eq!(display_budget(4, 4), (4, 0));
}

#[test]
fn no_diagnostics_needs_no_budget() {
    assert_eq!(display_budget(0, 0), (0, 0));
}

// ─────────────────────────────────────────────────────────────────────
// split_budget
// ─────────────────────────────────────────────────────────────────────

#[test]
fn an_ample_budget_renders_everything() {
    assert_eq!(split_budget(2, 3, 5), RenderPlan { parse: 2, elab: 3 });
}

/// Parse errors render first, so they take the budget first — an elab-first
/// split would show the wrong diagnostics under `--max-errors=1`.
#[test]
fn parse_errors_consume_the_budget_first() {
    assert_eq!(split_budget(2, 3, 1), RenderPlan { parse: 1, elab: 0 });
}

#[test]
fn elaboration_errors_get_what_parse_errors_leave() {
    assert_eq!(split_budget(2, 3, 4), RenderPlan { parse: 2, elab: 2 });
}

#[test]
fn a_zero_budget_renders_nothing() {
    assert_eq!(split_budget(2, 3, 0), RenderPlan { parse: 0, elab: 0 });
}

#[test]
fn no_parse_errors_leaves_the_whole_budget_to_elaboration() {
    assert_eq!(split_budget(0, 3, 2), RenderPlan { parse: 0, elab: 2 });
}

// ─────────────────────────────────────────────────────────────────────
// raw_diagnostic_count
// ─────────────────────────────────────────────────────────────────────

#[test]
fn the_raw_count_sums_both_kinds() {
    assert_eq!(
        raw_diagnostic_count(&[parse_err(0, 1), parse_err(2, 3)], &[elab_error(4, 5)]),
        3
    );
}

#[test]
fn the_raw_count_of_nothing_is_zero() {
    assert_eq!(raw_diagnostic_count(&[], &[]), 0);
}

/// It counts *raw* diagnostics — two errors at one span are two, even
/// though deduplication will fold them to one.
#[test]
fn the_raw_count_precedes_deduplication() {
    let same_span = [elab_error(0, 5), elab_error(0, 5)];
    assert_eq!(raw_diagnostic_count(&[], &same_span), 2);
    assert_eq!(deduplicate_errors(&same_span).len(), 1);
}

// ─────────────────────────────────────────────────────────────────────
// truncation_plan (ADR 14.8.26g D7)
// ─────────────────────────────────────────────────────────────────────

fn error_in_file(file: &str) -> ElabError {
    elab_error(0, 1).with_file_path(std::path::PathBuf::from(file))
}

#[test]
fn an_ample_budget_selects_everything_in_order() {
    let errors = [error_in_file("a.tg"), error_in_file("b.tg")];
    let refs: Vec<&ElabError> = errors.iter().collect();
    assert_eq!(truncation_plan(&refs, 5), vec![0, 1]);
}

/// Truncation keeps at least one diagnostic per failing file: five errors
/// in `a.tg` followed by one in `b.tg`, budget 2 → the first of each,
/// not the first two of `a.tg`.
#[test]
fn truncation_keeps_one_diagnostic_per_failing_file() {
    let errors = [
        error_in_file("a.tg"),
        error_in_file("a.tg"),
        error_in_file("a.tg"),
        error_in_file("a.tg"),
        error_in_file("a.tg"),
        error_in_file("b.tg"),
    ];
    let refs: Vec<&ElabError> = errors.iter().collect();
    assert_eq!(truncation_plan(&refs, 2), vec![0, 5]);
}

/// Leftover budget after the per-file pass fills in list order.
#[test]
fn leftover_budget_fills_in_list_order() {
    let errors = [
        error_in_file("a.tg"),
        error_in_file("a.tg"),
        error_in_file("a.tg"),
        error_in_file("b.tg"),
    ];
    let refs: Vec<&ElabError> = errors.iter().collect();
    assert_eq!(truncation_plan(&refs, 3), vec![0, 1, 3]);
}

/// More failing files than budget: the first `limit` files get one each —
/// no selection can do better.
#[test]
fn more_files_than_budget_takes_the_first_files() {
    let errors = [
        error_in_file("a.tg"),
        error_in_file("b.tg"),
        error_in_file("c.tg"),
    ];
    let refs: Vec<&ElabError> = errors.iter().collect();
    assert_eq!(truncation_plan(&refs, 2), vec![0, 1]);
}

// ─────────────────────────────────────────────────────────────────────
// the render entry points' verdict
//
// These write to stderr; what is asserted is only the returned
// "were there errors" verdict, which is what the driver's exit code reads.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn rendering_nothing_reports_no_errors() {
    assert!(!render_diagnostics_limited(&src(), &[], &[], &[], 0));
}

#[test]
fn rendering_an_elab_error_reports_errors() {
    assert!(render_diagnostics_limited(
        &src(),
        &[elab_error(16, 17)],
        &[],
        &[],
        0
    ));
}

#[test]
fn rendering_a_parse_error_reports_errors() {
    assert!(render_diagnostics_limited(
        &src(),
        &[],
        &[parse_err(16, 17)],
        &[],
        0
    ));
}

#[test]
fn rendering_only_warnings_reports_no_errors() {
    assert!(!render_diagnostics_limited(
        &src(),
        &[],
        &[],
        &[elab_error(16, 17)],
        0
    ));
}

#[test]
fn the_source_map_entry_point_reports_no_errors_when_there_are_none() {
    let map = crate::driver::modules::SourceMap::default();
    assert!(!render_diagnostics_with_source_map_limited(
        &src(),
        &map,
        &[],
        &[],
        0
    ));
}

#[test]
fn the_source_map_entry_point_reports_errors_when_there_are_some() {
    let map = crate::driver::modules::SourceMap::default();
    assert!(render_diagnostics_with_source_map_limited(
        &src(),
        &map,
        &[elab_error(16, 17)],
        &[],
        0
    ));
}

// ─────────────────────────────────────────────────────────────────────
// json_records (ADR 28.9.26e AC4)
// ─────────────────────────────────────────────────────────────────────

fn warning(start: u32, end: u32) -> ElabError {
    ElabError::new(Span::new(start, end), ElabErrorKind::UnreachableArm)
}

#[test]
fn json_records_report_every_diagnostic_in_human_order() {
    let parse = [parse_err(0, 2)];
    let elab_errors: Vec<ElabError> = (0..5).map(|i| elab_error(i, i + 1)).collect();
    let warnings = [warning(3, 4)];
    let parse_refs: Vec<&ParseError> = parse.iter().collect();
    let elab_refs = deduplicate_errors(&elab_errors);
    let set = DiagnosticSet {
        parse: &parse_refs,
        elab: &elab_refs,
        warnings: &warnings,
    };
    let records = json_records(&src(), None, &set);
    let shape: Vec<(Level, Option<&str>)> = records
        .iter()
        .map(|r| (r.level, r.code.as_deref()))
        .collect();
    // Seven records, where the human mode at its default budget would stop
    // short: no display budget, no truncation plan.
    assert_eq!(
        shape,
        [
            (Level::Error, Some("E0111")), // the UnexpectedEof fixture (29.9.26f)
            (Level::Error, Some("E0001")),
            (Level::Error, Some("E0001")),
            (Level::Error, Some("E0001")),
            (Level::Error, Some("E0001")),
            (Level::Error, Some("E0001")),
            (Level::Warning, Some("W0001")),
        ]
    );
}

#[test]
fn json_records_take_the_deduplicated_list() {
    let duplicated = [elab_error(4, 5), elab_error(4, 5)];
    let elab_refs = deduplicate_errors(&duplicated);
    let set = DiagnosticSet {
        parse: &[],
        elab: &elab_refs,
        warnings: &[],
    };
    assert_eq!(json_records(&src(), None, &set).len(), 1);
}

#[test]
fn json_records_resolve_each_file_through_the_source_map() {
    let mut source_map = SourceMap::new();
    source_map.insert(
        std::path::PathBuf::from("lib.tg"),
        "fn g() -> Nat {\n  x\n}".to_string(),
    );
    let in_lib = elab_error(18, 19).with_file_path("lib.tg");
    let in_main = elab_error(16, 17);
    let errors = [&in_lib, &in_main];
    let set = DiagnosticSet {
        parse: &[],
        elab: &errors,
        warnings: &[],
    };
    let records = json_records(&src(), Some(&source_map), &set);
    let lib_span = &records[0].primary_span.as_ref().unwrap().location;
    assert_eq!(lib_span.file, "lib.tg");
    assert_eq!(
        (lib_span.line_start, lib_span.col_start),
        (Some(2), Some(3))
    );
    let main_span = &records[1].primary_span.as_ref().unwrap().location;
    assert_eq!(main_span.file, "t.tg");
    assert_eq!(
        (main_span.line_start, main_span.col_start),
        (Some(1), Some(17))
    );
}

// ─────────────────────────────────────────────────────────────────────
// JSON arm's return value (ADR 28.9.26e): `has_errors` must mean what it
// means in human mode, or `check` exits 0 on an error in JSON mode.
// ─────────────────────────────────────────────────────────────────────

/// Render in JSON mode on this test's own thread (the format is a
/// thread-local), returning `has_errors`.
fn json_mode<T>(render: impl FnOnce() -> T) -> T {
    super::super::set_error_format(ErrorFormat::Json);
    let has_errors = render();
    super::super::set_error_format(ErrorFormat::Human);
    has_errors
}

#[test]
fn the_json_arm_reports_errors_from_either_list() {
    let (parse, elab) = ([parse_err(0, 2)], [elab_error(0, 1)]);
    let src = src();
    assert!(json_mode(|| render_diagnostics_limited(
        &src,
        &[],
        &parse,
        &[],
        20
    )));
    assert!(json_mode(|| render_diagnostics_limited(
        &src,
        &elab,
        &[],
        &[],
        20
    )));
    assert!(json_mode(|| render_diagnostics_limited(
        &src,
        &elab,
        &parse,
        &[],
        20
    )));
}

#[test]
fn the_json_arm_reports_no_errors_for_warnings_alone() {
    let warnings = [warning(3, 4)];
    let src = src();
    assert!(!json_mode(|| render_diagnostics_limited(
        &src,
        &[],
        &[],
        &warnings,
        20
    )));
    assert!(!json_mode(|| render_diagnostics_limited(
        &src,
        &[],
        &[],
        &[],
        20
    )));
    let map = SourceMap::new();
    assert!(!json_mode(|| {
        render_diagnostics_with_source_map_limited(&src, &map, &[], &warnings, 20)
    }));
}

#[test]
fn the_json_arm_with_a_source_map_reports_errors() {
    let (elab, map, src) = ([elab_error(0, 1)], SourceMap::new(), src());
    assert!(json_mode(|| {
        render_diagnostics_with_source_map_limited(&src, &map, &elab, &[], 20)
    }));
}
