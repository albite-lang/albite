//! Table mode's rows (ADR 3.10.26a D3, D4): a row's agreement verdict from
//! its two outcomes, the rendered table and the exit code. Pure, like
//! `table.rs`, which classifies each single run.

use std::fmt::Write as _;

use super::table::Outcome;

/// A row's agreement verdict (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    Agree,
    /// Carries what differs: which side accepted, or that counts differ.
    Diverge(&'static str),
    Fault,
}

impl Verdict {
    fn cell(&self) -> String {
        match self {
            Self::Agree => "agree".to_string(),
            Self::Diverge(why) => format!("diverge ({why})"),
            Self::Fault => "fault".to_string(),
        }
    }
}

/// Agree when both accept with equal counts or both refuse; any fault makes
/// the row a fault, never an agreement.
pub(super) fn classify_row(bootstrap: &Outcome, selfhost: &Outcome) -> Verdict {
    match (bootstrap, selfhost) {
        (Outcome::Fault(_), _) | (_, Outcome::Fault(_)) => Verdict::Fault,
        (Outcome::Accepted(b), Outcome::Accepted(s)) if b.definitions == s.definitions => {
            Verdict::Agree
        }
        (Outcome::Accepted(_), Outcome::Accepted(_)) => Verdict::Diverge("counts differ"),
        (Outcome::Refused(_), Outcome::Refused(_)) => Verdict::Agree,
        (Outcome::Accepted(_), Outcome::Refused(_)) => Verdict::Diverge("bootstrap accepts"),
        (Outcome::Refused(_), Outcome::Accepted(_)) => Verdict::Diverge("self-host accepts"),
    }
}

/// One table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) entry: String,
    pub(super) bootstrap: Outcome,
    pub(super) selfhost: Outcome,
    pub(super) verdict: Verdict,
}

impl Row {
    pub(super) fn new(entry: String, bootstrap: Outcome, selfhost: Outcome) -> Self {
        let verdict = classify_row(&bootstrap, &selfhost);
        Self {
            entry,
            bootstrap,
            selfhost,
            verdict,
        }
    }
}

/// The widest an outcome cell prints before it is truncated with `…`.
pub(super) const CELL_WIDTH: usize = 36;

fn truncate_cell(text: &str) -> String {
    if text.chars().count() <= CELL_WIDTH {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(CELL_WIDTH - 1).collect();
    cut.push('…');
    cut
}

fn pad(text: &str, width: usize) -> String {
    let len = text.chars().count();
    format!("{text}{}", " ".repeat(width.saturating_sub(len)))
}

/// Render the table, its summary line, and under `verbose` every row's full
/// cells below it.
pub(super) fn render_table(rows: &[Row], verbose: bool) -> String {
    let entry_width = rows
        .iter()
        .map(|row| row.entry.chars().count())
        .chain(std::iter::once("entry".len()))
        .max()
        .unwrap_or(0)
        + 2;
    let column = CELL_WIDTH + 2;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}{}{}verdict",
        pad("entry", entry_width),
        pad("bootstrap", column),
        pad("self-host", column)
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{}{}{}{}",
            pad(&row.entry, entry_width),
            pad(&truncate_cell(&row.bootstrap.cell()), column),
            pad(&truncate_cell(&row.selfhost.cell()), column),
            row.verdict.cell()
        );
    }
    let count = |want: fn(&Verdict) -> bool| rows.iter().filter(|r| want(&r.verdict)).count();
    let _ = writeln!(
        out,
        "{} entries: {} agree, {} diverge, {} fault",
        rows.len(),
        count(|v| *v == Verdict::Agree),
        count(|v| matches!(v, Verdict::Diverge(_))),
        count(|v| *v == Verdict::Fault)
    );
    if verbose {
        for row in rows {
            let _ = writeln!(out, "\n{}:", row.entry);
            let _ = writeln!(out, "  bootstrap: {}", row.bootstrap.cell());
            let _ = writeln!(out, "  self-host: {}", row.selfhost.cell());
        }
    }
    out
}

/// D4: 3 when any row faulted — a broken run proves nothing, so it outranks
/// a divergence — else 1 when any row diverged, else 0.
pub(super) fn table_exit_code(rows: &[Row]) -> u8 {
    if rows.iter().any(|row| row.verdict == Verdict::Fault) {
        3
    } else {
        u8::from(
            rows.iter()
                .any(|row| matches!(row.verdict, Verdict::Diverge(_))),
        )
    }
}

// Tests: table_verdict_tests.rs
#[cfg(test)]
#[path = "table_verdict_tests.rs"]
mod tests;
