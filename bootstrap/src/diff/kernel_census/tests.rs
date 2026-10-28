//! ADR 4.10.26a — the census diff over inline report text.

use super::parse::{cause_of, ReportRow};
use super::*;

/// A report in the census's own `--all-rows` shape, its counts derived from
/// `rows` the way `CensusTally::record` derives them.
fn report(rows: &[&str]) -> String {
    let mut axioms = 0;
    let mut admitted = 0;
    let mut refused = 0;
    for row in rows {
        let primary = row.split(": ").nth(1).unwrap().split(" / ").next().unwrap();
        match primary {
            "axiom" => axioms += 1,
            "admitted" => admitted += 1,
            _ => refused += 1,
        }
    }
    let mut text = format!(
        "kernel census: src/compiler/main.tg\n  \
         population: {} proof-relevant (+0 total with --include-total), {axioms} axioms not checked\n  \
         admitted:   {admitted}\n  \
         refused:    {refused}   (first refusal per definition)\n",
        rows.len()
    );
    if refused > 0 {
        text.push_str("    cause                    primary   secondary\n");
        text.push_str("    hole                           1           0\n");
    }
    for row in rows {
        text.push_str("  ");
        text.push_str(row);
        text.push('\n');
    }
    text
}

/// `text` with only the lines `keep` accepts, each newline-terminated.
fn keep_lines(text: &str, keep: impl Fn(&str) -> bool) -> String {
    text.lines()
        .filter(|line| keep(line))
        .flat_map(|line| [line, "\n"])
        .collect()
}

pub(super) fn parsed(rows: &[&str]) -> CensusReport {
    parse_census_report(&report(rows)).expect("a well-formed report")
}

// ADR 4.10.26a AC1 — rows parse, with and without a message.
#[test]
fn a_census_row_parses_with_and_without_a_message() {
    let rows = parsed(&[
        "a: admitted / —",
        "b: hole / unclassified — sorry in proof",
        "c: unexplained-mismatch / — — expected A, found B — again",
        "m::d: axiom / —",
    ]);
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows["a"],
        ReportRow {
            primary: Cause::Admitted,
            secondary: None
        }
    );
    assert_eq!(
        rows["b"],
        ReportRow {
            primary: Cause::Hole,
            secondary: Some(Cause::Unclassified)
        }
    );
    assert_eq!(
        rows["c"],
        ReportRow {
            primary: Cause::UnexplainedMismatch,
            secondary: None
        }
    );
    assert_eq!(rows["m::d"].primary, Cause::Axiom);
}

// ADR 4.10.26a AC1 — header, count and cause-table lines are never rows.
#[test]
fn header_and_count_lines_are_not_rows() {
    let rows = parsed(&["x: hole / admitted — sorry"]);
    assert_eq!(rows.keys().collect::<Vec<_>>(), ["x"]);
    assert_eq!(
        cause_of("named-type-reference"),
        Some(Cause::NamedTypeReference)
    );
    assert_eq!(cause_of("cause"), None);
}

// ADR 4.10.26a AC1 — a malformed row-like line is refused, by line number.
#[test]
fn a_malformed_row_like_line_is_refused_with_its_line_number() {
    let mut text = report(&["a: admitted / —"]);
    text.push_str("  b: not-a-cause / —\n");
    // header, population, admitted, refused, a — then b is line 6.
    assert_eq!(
        parse_census_report(&text),
        Err(ReportError::MalformedRow {
            line: 6,
            text: "  b: not-a-cause / —".to_string()
        })
    );
    let no_slash = format!("{}  c: admitted\n", report(&["a: admitted / —"]));
    assert!(matches!(
        parse_census_report(&no_slash),
        Err(ReportError::MalformedRow { line: 6, .. })
    ));
    let bad_count = report(&["a: admitted / —"]).replace("admitted:   1", "admitted:   one");
    assert_eq!(
        parse_census_report(&bad_count),
        Err(ReportError::MalformedCountLine {
            line: 3,
            text: "admitted:   one".to_string()
        })
    );
}

// ADR 4.10.26a AC1 — a `-v` report prints refused rows only, so its row
// count falls below `admitted + refused + axioms`.
#[test]
fn a_verbose_report_without_all_rows_is_refused() {
    let full = report(&[
        "a: admitted / —",
        "b: hole / admitted — sorry",
        "c: axiom / —",
    ]);
    let verbose = keep_lines(&full, |line| {
        !line.starts_with("  a:") && !line.starts_with("  c:")
    });
    assert_eq!(
        parse_census_report(&verbose),
        Err(ReportError::RowCountMismatch {
            expected: 3,
            found: 1
        })
    );
    // With nothing refused, `-v` prints no row at all: still the mismatch.
    let clean = report(&["a: admitted / —"]);
    let clean_verbose = clean.replace("  a: admitted / —\n", "");
    assert_eq!(
        parse_census_report(&clean_verbose),
        Err(ReportError::RowCountMismatch {
            expected: 1,
            found: 0
        })
    );
}

// ADR 4.10.26a AC1 — a report cut short loses rows, or loses its counts.
#[test]
fn a_truncated_report_is_refused() {
    let full = report(&["a: admitted / —", "b: admitted / —"]);
    let cut = full.lines().take(5).collect::<Vec<_>>().join("\n");
    assert_eq!(
        parse_census_report(&cut),
        Err(ReportError::RowCountMismatch {
            expected: 2,
            found: 1
        })
    );
    let headless = full.lines().skip(1).collect::<Vec<_>>().join("\n");
    assert_eq!(
        parse_census_report(&headless),
        Err(ReportError::MissingHeader)
    );
    for keyword in ["population", "admitted", "refused"] {
        let prefix = format!("  {keyword}:");
        let without = keep_lines(&full, |line| !line.starts_with(&prefix));
        assert_eq!(
            parse_census_report(&without),
            Err(ReportError::MissingCountLine(keyword))
        );
    }
    let twice = format!("{full}{full}");
    assert!(matches!(
        parse_census_report(&twice),
        Err(ReportError::RepeatedLine {
            keyword: "kernel census",
            ..
        })
    ));
}

// ADR 4.10.26a AC1 — counts of zero and no rows is nothing to compare.
#[test]
fn a_report_with_no_rows_is_refused() {
    assert_eq!(parse_census_report(&report(&[])), Err(ReportError::NoRows));
}

// ADR 4.10.26a AC1 — a duplicate name is refused, naming the report.
#[test]
fn a_duplicate_name_is_refused_in_either_report() {
    let good = report(&["a: admitted / —", "b: admitted / —"]);
    let duplicated = report(&["a: admitted / —", "a: hole / — — sorry"]);
    let expected = |path: &str| DiffFailure::Invalid {
        path: path.to_string(),
        error: ReportError::DuplicateName("a".to_string()),
    };
    assert_eq!(
        diff_report_texts(("old.txt", &duplicated), ("new.txt", &good)),
        Err(expected("old.txt"))
    );
    assert_eq!(
        diff_report_texts(("old.txt", &good), ("new.txt", &duplicated)),
        Err(expected("new.txt"))
    );
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

// ADR 4.10.26a AC2 — moves are counted per (from, to) primary pair.
#[test]
fn moves_are_counted_by_primary_cause_pair() {
    let before = parsed(&[
        "a: named-type-reference / — — ty",
        "b: named-type-reference / — — ty",
        "c: named-type-reference / — — ty",
        "d: hole / admitted — sorry",
        "e: admitted / —",
    ]);
    let after = parsed(&[
        "a: admitted / —",
        "b: unclassified / — — other",
        "c: admitted / —",
        "d: admitted / —",
        "e: admitted / —",
    ]);
    let diff = join_reports(&before, &after);
    let groups: Vec<_> = diff
        .move_groups()
        .map(|(from, to, names)| (from, to, names.to_vec()))
        .collect();
    assert_eq!(
        groups,
        vec![
            (Cause::Hole, Cause::Admitted, names(&["d"])),
            (
                Cause::NamedTypeReference,
                Cause::Admitted,
                names(&["a", "c"])
            ),
            (
                Cause::NamedTypeReference,
                Cause::Unclassified,
                names(&["b"])
            ),
        ]
    );
    assert_eq!(diff.moved(), 4);
    assert!(diff.secondary_only.is_empty());
}

// ADR 4.10.26a AC2 — a secondary-only change is listed, never moved.
#[test]
fn a_secondary_only_change_is_not_a_move() {
    let before = parsed(&["a: hole / admitted — sorry", "b: hole / — — sorry"]);
    let after = parsed(&["a: hole / unclassified — sorry", "b: hole / — — sorry"]);
    let diff = join_reports(&before, &after);
    assert_eq!(diff.moved(), 0);
    assert_eq!(
        diff.secondary_only,
        vec![SecondaryChange {
            name: "a".to_string(),
            primary: Cause::Hole,
            before: Some(Cause::Admitted),
            after: Some(Cause::Unclassified),
        }]
    );
}

// ADR 4.10.26a AC2 — a name in one report only is listed apart.
#[test]
fn a_name_in_one_report_only_is_listed_apart() {
    let before = parsed(&[
        "gone: admitted / —",
        "kept: admitted / —",
        "also: hole / — — x",
    ]);
    let after = parsed(&[
        "kept: admitted / —",
        "new: hole / — — x",
        "fresh: admitted / —",
    ]);
    let diff = join_reports(&before, &after);
    assert_eq!(diff.only_before, names(&["also", "gone"]));
    assert_eq!(diff.only_after, names(&["fresh", "new"]));
    assert_eq!(diff.moved(), 0);
}

// ADR 4.10.26a AC1 — a second count line is refused, and every refusal
// renders a line a reader can act on.
#[test]
fn a_repeated_count_line_is_refused_and_each_refusal_reads_plainly() {
    let full = report(&["a: admitted / —"]);
    let twice = full.replace("  refused:", "  admitted:   1\n  refused:");
    assert_eq!(
        parse_census_report(&twice),
        Err(ReportError::RepeatedLine {
            line: 4,
            keyword: "admitted"
        })
    );
    let rendered: Vec<String> = [
        ReportError::MissingHeader,
        ReportError::MissingCountLine("refused"),
        ReportError::RepeatedLine {
            line: 4,
            keyword: "admitted",
        },
        ReportError::MalformedCountLine {
            line: 3,
            text: "admitted: x".to_string(),
        },
        ReportError::MalformedRow {
            line: 7,
            text: "  b: ?".to_string(),
        },
        ReportError::DuplicateName("a".to_string()),
        ReportError::NoRows,
        ReportError::RowCountMismatch {
            expected: 3,
            found: 1,
        },
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        rendered,
        [
            "no `kernel census: <file>` header",
            "no `refused:` count line",
            "line 4: a second `admitted` line (one report per file)",
            "line 3: count line does not parse: admitted: x",
            "line 7: not a census row:   b: ?",
            "definition `a` appears twice",
            "no rows — a census of nothing cannot be compared",
            "1 rows, but the counts say 3 — save the report with --all-rows, and in full",
        ]
    );
}
