//! The census's output, built as a value (ADR 30.9.26f §2.1).
//!
//! Built rather than printed so every branch is assertable, the
//! `check_termination` idiom.

use std::fmt::Write as _;

use super::row::CensusRow;
use super::CensusRows;
use super::{Cause, CensusTally};

/// The whole report for `file`: the population line, the admitted/refused
/// counts, the primary/secondary table over the causes that occurred, and one
/// line per row `shown` selects, in the rows' (name) order.
#[must_use]
pub fn render_census(
    file: &str,
    tally: &CensusTally,
    rows: &[CensusRow],
    shown: CensusRows,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "kernel census: {file}");
    let _ = writeln!(
        out,
        "  population: {} proof-relevant (+{} total with --include-total), {} axioms not checked",
        tally.proof_relevant, tally.total_only, tally.axioms
    );
    let _ = writeln!(out, "  admitted:   {}", tally.admitted);
    let _ = writeln!(
        out,
        "  refused:    {}   (first refusal per definition)",
        tally.refused
    );
    if tally.refused > 0 {
        render_cause_table(&mut out, tally);
    }
    if !tally.data_statements.is_empty() {
        let _ = writeln!(
            out,
            "  sort Type₀:  {}   (statements that state data, ADR 4.10.26i D6 (b))",
            tally.data_statements.len()
        );
    }
    for row in rows.iter().filter(|row| shown.shows(row)) {
        render_row(&mut out, row, tally.data_statements.contains(&row.name));
    }
    out
}

fn render_cause_table(out: &mut String, tally: &CensusTally) {
    let _ = writeln!(
        out,
        "    {:<24} {:>7}   {:>9}",
        "cause", "primary", "secondary"
    );
    for cause in Cause::ALL {
        let primary = tally.primary.get(&cause).copied().unwrap_or(0);
        let secondary = tally.secondary.get(&cause).copied().unwrap_or(0);
        if primary == 0 && secondary == 0 {
            continue;
        }
        let _ = writeln!(
            out,
            "    {:<24} {:>7}   {:>9}",
            cause.label(),
            primary,
            secondary
        );
    }
}

/// `  <name>: <primary> / <secondary>`, then ` — <refusal>` only when there
/// is one, so an admitted or axiom row has no dangling dash (1.10.26m D2, D3).
/// A statement that sorts `Type₀` carries ` [sort: Type₀]` (4.10.26i D6 (b)).
fn render_row(out: &mut String, row: &CensusRow, states_data: bool) {
    let secondary = row.secondary.map_or("—", Cause::label);
    let _ = write!(
        out,
        "  {}: {} / {}",
        row.name,
        row.primary.label(),
        secondary
    );
    if states_data {
        out.push_str(" [sort: Type₀]");
    }
    if let Some(refusal) = &row.refusal {
        let _ = write!(out, " — {}", refusal.replace('\n', " "));
    }
    out.push('\n');
}
