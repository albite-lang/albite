//! Tests for table mode's rows, rendering and exit code (ADR 3.10.26a AC 3).

use super::*;
use crate::diff::bootstrap_selfhost::table::Accepted;

fn ok(n: usize) -> Outcome {
    Outcome::Accepted(Accepted {
        definitions: n,
        warning: None,
    })
}

fn refused() -> Outcome {
    Outcome::Refused("error[E1]: no".to_string())
}

fn fault() -> Outcome {
    Outcome::Fault("timeout after 1s".to_string())
}

fn row(bootstrap: Outcome, selfhost: Outcome) -> Row {
    Row::new("a.tg".to_string(), bootstrap, selfhost)
}

// 3.10.26a AC3
#[test]
fn both_accept_with_equal_counts_agree() {
    assert_eq!(classify_row(&ok(2), &ok(2)), Verdict::Agree);
    // A warning suffix never enters the comparison; only N does.
    let warned = Outcome::Accepted(Accepted {
        definitions: 2,
        warning: Some("contains sorry (1 authored, 0 synthesised)".to_string()),
    });
    assert_eq!(classify_row(&warned, &ok(2)), Verdict::Agree);
}

// 3.10.26a AC3
#[test]
fn unequal_counts_diverge() {
    assert_eq!(
        classify_row(&ok(2), &ok(3)),
        Verdict::Diverge("counts differ")
    );
}

// 3.10.26a AC3
#[test]
fn both_refuse_agree() {
    let other = Outcome::Refused("[E0003] Error: different text".to_string());
    assert_eq!(classify_row(&other, &refused()), Verdict::Agree);
}

// 3.10.26a AC3
#[test]
fn one_accept_diverges_and_names_it() {
    assert_eq!(
        classify_row(&ok(1), &refused()),
        Verdict::Diverge("bootstrap accepts")
    );
    assert_eq!(
        classify_row(&refused(), &ok(1)),
        Verdict::Diverge("self-host accepts")
    );
    assert_eq!(
        row(ok(1), refused()).verdict.cell(),
        "diverge (bootstrap accepts)"
    );
}

// 3.10.26a AC3
#[test]
fn any_fault_makes_the_row_a_fault() {
    for (b, s) in [
        (fault(), ok(1)),
        (ok(1), fault()),
        (fault(), refused()),
        (refused(), fault()),
        (fault(), fault()),
    ] {
        assert_eq!(classify_row(&b, &s), Verdict::Fault);
    }
}

// 3.10.26a AC3
#[test]
fn table_exit_code_is_zero_one_or_three() {
    assert_eq!(
        table_exit_code(&[row(ok(1), ok(1)), row(refused(), refused())]),
        0
    );
    assert_eq!(
        table_exit_code(&[row(ok(1), ok(1)), row(ok(1), refused())]),
        1
    );
    assert_eq!(
        table_exit_code(&[row(ok(1), ok(1)), row(fault(), ok(1))]),
        3
    );
    assert_eq!(table_exit_code(&[]), 0);
}

// 3.10.26a AC3
#[test]
fn a_fault_outranks_a_divergence() {
    assert_eq!(
        table_exit_code(&[row(ok(1), refused()), row(fault(), ok(1))]),
        3
    );
    assert_eq!(
        table_exit_code(&[row(fault(), ok(1)), row(ok(1), ok(2))]),
        3
    );
}

#[test]
fn render_table_prints_one_row_per_entry_and_the_summary() {
    let rows = [
        Row::new("boxes.tg".to_string(), ok(0), ok(0)),
        Row::new("sub/long_entry.tg".to_string(), ok(2), refused()),
        Row::new("c.tg".to_string(), fault(), ok(1)),
    ];
    let text = render_table(&rows, false);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5, "{text}");
    // Entry column: the widest label plus two spaces.
    let entry_width = "sub/long_entry.tg".len() + 2;
    let column = CELL_WIDTH + 2;
    assert_eq!(
        lines[0],
        format!(
            "{:<entry_width$}{:<column$}{:<column$}verdict",
            "entry", "bootstrap", "self-host"
        )
    );
    assert_eq!(
        lines[1],
        format!(
            "{:<entry_width$}{:<column$}{:<column$}agree",
            "boxes.tg", "ok, 0", "ok, 0"
        )
    );
    assert!(lines[2].ends_with("diverge (bootstrap accepts)"), "{text}");
    assert!(lines[3].contains("fault: timeout after 1s"), "{text}");
    assert!(lines[3].ends_with("fault"), "{text}");
    assert_eq!(lines[4], "3 entries: 1 agree, 1 diverge, 1 fault");
    assert!(!text.contains("\nboxes.tg:"), "no verbose block: {text}");
}

#[test]
fn render_table_truncates_long_cells_and_verbose_prints_them_in_full() {
    let long = "[E0003] Error: cannot find constructor `MkBox` in this scope";
    let rows = [Row::new(
        "a.tg".to_string(),
        Outcome::Refused(long.to_string()),
        refused(),
    )];
    let quiet = render_table(&rows, false);
    let cut: String = long.chars().take(CELL_WIDTH - 1).collect();
    assert!(quiet.contains(&format!("{cut}…")), "{quiet}");
    assert!(!quiet.contains(long), "{quiet}");

    let loud = render_table(&rows, true);
    assert!(
        loud.contains(&format!("\na.tg:\n  bootstrap: {long}\n")),
        "{loud}"
    );
    assert!(loud.contains("  self-host: error[E1]: no\n"), "{loud}");
}

#[test]
fn truncate_cell_keeps_a_cell_exactly_at_the_width() {
    let exact = "x".repeat(CELL_WIDTH);
    assert_eq!(truncate_cell(&exact), exact);
    let over = "x".repeat(CELL_WIDTH + 1);
    let cut = truncate_cell(&over);
    assert_eq!(cut.chars().count(), CELL_WIDTH);
    assert!(cut.ends_with('…'));
}

#[test]
fn the_entry_column_is_at_least_as_wide_as_its_heading() {
    let text = render_table(&[Row::new("a".to_string(), ok(0), ok(0))], false);
    assert!(text.starts_with("entry  bootstrap"), "{text}");
    assert!(text.contains("\na      ok, 0"), "{text}");
}
