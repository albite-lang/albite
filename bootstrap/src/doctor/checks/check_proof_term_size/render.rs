//! What `term-size` prints: the census (ADR 13.10.26c D6, widened by
//! 15.10.26g D2, D3, D6) and the baseline delta (15.10.26g D6), each as text
//! and as one `--json` object.

use std::fmt::Write as _;

use serde_json::{json, Value};

use super::delta::{by_size, Delta, MatchedRow, Thresholds};
use super::{axiom_count, totals, Population, ProofTermSize, SizeRow};

/// The rows to print: every row in `(module, name)` order, or with `top` the
/// first `n` by size (D6).
fn shown(rows: &[SizeRow], top: Option<usize>) -> Vec<SizeRow> {
    match top {
        Some(n) => by_size(rows).into_iter().take(n).collect(),
        None => rows.to_vec(),
    }
}

fn row_json(row: &SizeRow) -> Value {
    let count = |field: fn(ProofTermSize) -> usize| -> Value {
        row.size.map_or(Value::Null, |s| json!(field(s)))
    };
    json!({
        "module": row.module,
        "name": row.name,
        "axiom": row.size.is_none(),
        "subst": count(|s| s.subst),
        "refl": count(|s| s.refl),
        "global_occurrences": count(|s| s.global_occurrences),
        "nodes": count(|s| s.nodes),
        "case_arms": count(|s| s.case_arms),
    })
}

fn rows_json(rows: &[SizeRow]) -> Vec<Value> {
    rows.iter().map(row_json).collect()
}

/// D6's object: the population, the rows, `"top"` under `--top`, then the
/// non-axiom totals.
#[must_use]
pub fn render_json(
    file: &str,
    population: Population,
    rows: &[SizeRow],
    top: Option<usize>,
) -> String {
    let sum = totals(rows);
    let mut object = json!({
        "file": file,
        "population": population.label(),
        "rows": rows_json(rows),
        "totals": {
            "definitions": rows.len(),
            "axioms": axiom_count(rows),
            "subst": sum.subst,
            "refl": sum.refl,
            "global_occurrences": sum.global_occurrences,
            "nodes": sum.nodes,
            "case_arms": sum.case_arms,
        },
    });
    if top.is_some() {
        object["top"] = Value::Array(rows_json(&shown(rows, top)));
    }
    format!("{object}\n")
}

fn counts_text(size: Option<ProofTermSize>) -> String {
    size.map_or_else(
        || {
            format!(
                "{:>6} {:>6} {:>10} {:>7} {:>9}",
                "axiom", "-", "-", "-", "-"
            )
        },
        |s| {
            format!(
                "{:>6} {:>6} {:>10} {:>7} {:>9}",
                s.subst, s.refl, s.global_occurrences, s.nodes, s.case_arms
            )
        },
    )
}

/// The text form (§2): one line per shown row, then totals over every row.
#[must_use]
pub fn render_text(file: &str, rows: &[SizeRow], top: Option<usize>) -> String {
    let mut report = format!("proof term size: {file}\n");
    let _ = writeln!(
        report,
        "  {:<32} {:<24} {:>6} {:>6} {:>10} {:>7} {:>9}",
        "definition", "module", "subst", "refl", "global-occ", "nodes", "case-arms"
    );
    for row in shown(rows, top) {
        let counts = counts_text(row.size);
        let _ = writeln!(report, "  {:<32} {:<24} {counts}", row.name, row.module);
    }
    let sum = totals(rows);
    let proof_relevant = rows.iter().filter(|row| row.proof).count();
    let _ = writeln!(
        report,
        "  {} definition(s), {proof_relevant} proof-relevant, {} axiom(s); totals: subst {}, refl {}, global-occ {}, nodes {}, case-arms {}",
        rows.len(),
        axiom_count(rows),
        sum.subst,
        sum.refl,
        sum.global_occurrences,
        sum.nodes,
        sum.case_arms
    );
    report.push_str("  (global-occ counts every citation, not distinct dependencies)\n");
    report
}

/// What the delta is rendered against: the paths, population and knobs.
pub struct DeltaHeader<'a> {
    pub file: &'a str,
    pub baseline: &'a str,
    pub population: Population,
    pub thresholds: Thresholds,
    pub top: usize,
}

fn nodes_of(size: Option<ProofTermSize>) -> String {
    size.map_or_else(|| "axiom".into(), |s| s.nodes.to_string())
}

fn arms_of(size: Option<ProofTermSize>) -> String {
    size.map_or_else(|| "-".into(), |s| s.case_arms.to_string())
}

fn matched_text(row: &MatchedRow) -> String {
    let delta = row.delta().map_or_else(|| "-".into(), |d| format!("{d:+}"));
    let ratio = row
        .ratio()
        .map_or_else(|| "-".into(), |r| format!("×{r:.2}"));
    let flag = if row.flagged { "  FLAGGED" } else { "" };
    format!(
        "  {:<32} {:<32} {:>8} {:>8} {:>8} {:>7}  {} → {}{flag}\n",
        row.name,
        row.module,
        nodes_of(row.before),
        nodes_of(row.after),
        delta,
        ratio,
        arms_of(row.before),
        arms_of(row.after)
    )
}

fn sized_text(row: &SizeRow, flagged: bool) -> String {
    let flag = if flagged { "  FLAGGED" } else { "" };
    format!(
        "  {:<32} {:<32} {:>8} {:>9}{flag}\n",
        row.name,
        row.module,
        nodes_of(row.size),
        arms_of(row.size)
    )
}

/// The delta's text form (D6): the top matched rows, every flagged row not
/// already shown, `new` and `removed`, `largest now:`, then the totals.
#[must_use]
pub fn render_delta_text(header: &DeltaHeader<'_>, delta: &Delta) -> String {
    let top = header.top;
    let mut report = format!(
        "core size: {}  vs  {}   (population: {})\n",
        header.file,
        header.baseline,
        header.population.label()
    );
    let _ = writeln!(
        report,
        "  {:<32} {:<32} {:>8} {:>8} {:>8} {:>7}  case-arms",
        "definition", "module", "before", "after", "delta", "ratio"
    );
    for row in delta.matched.iter().take(top) {
        report.push_str(&matched_text(row));
    }
    for row in delta.matched.iter().skip(top).filter(|row| row.flagged) {
        report.push_str(&matched_text(row));
    }
    let largest_new = delta.new.first().map_or_else(String::new, |(row, _)| {
        format!(" (largest: {} {})", row.name, nodes_of(row.size))
    });
    let _ = writeln!(
        report,
        "new: {}{largest_new}   removed: {}",
        delta.new.len(),
        delta.removed.len()
    );
    for (index, (row, flagged)) in delta.new.iter().enumerate() {
        if index < top || *flagged {
            report.push_str(&sized_text(row, *flagged));
        }
    }
    for row in delta.removed.iter().take(top) {
        let _ = writeln!(report, "  removed: {}::{}", row.module, row.name);
    }
    report.push_str("largest now:\n");
    for row in delta.largest.iter().take(top) {
        report.push_str(&sized_text(row, false));
    }
    let _ = writeln!(
        report,
        "nodes {} → {}; {} flagged (max-growth {}, min-delta {})",
        delta.nodes_before,
        delta.nodes_after,
        delta.flagged(),
        header.thresholds.max_growth,
        header.thresholds.min_delta
    );
    report
}

fn matched_json(row: &MatchedRow) -> Value {
    let field = |size: Option<ProofTermSize>, pick: fn(ProofTermSize) -> usize| {
        size.map_or(Value::Null, |s| json!(pick(s)))
    };
    json!({
        "module": row.module,
        "name": row.name,
        "before": field(row.before, |s| s.nodes),
        "after": field(row.after, |s| s.nodes),
        "delta": row.delta(),
        "ratio": row.ratio().filter(|r| r.is_finite()),
        "case_arms_before": field(row.before, |s| s.case_arms),
        "case_arms_after": field(row.after, |s| s.case_arms),
        "flagged": row.flagged,
    })
}

/// The delta's `--json` object (D6). `"rows"` is this run's own census, so
/// the report is itself a baseline for the next window (D5).
#[must_use]
pub fn render_delta_json(header: &DeltaHeader<'_>, delta: &Delta, rows: &[SizeRow]) -> String {
    let new: Vec<Value> = delta
        .new
        .iter()
        .map(|(row, flagged)| {
            let mut value = row_json(row);
            value["flagged"] = json!(flagged);
            value
        })
        .collect();
    let largest: Vec<SizeRow> = delta.largest.iter().take(header.top).cloned().collect();
    let object = json!({
        "file": header.file,
        "baseline": header.baseline,
        "population": header.population.label(),
        "thresholds": {
            "max_growth": header.thresholds.max_growth,
            "min_delta": header.thresholds.min_delta,
        },
        "matched": delta.matched.iter().map(matched_json).collect::<Vec<_>>(),
        "new": new,
        "removed": rows_json(&delta.removed),
        "largest": rows_json(&largest),
        "rows": rows_json(rows),
        "totals": {
            "nodes_before": delta.nodes_before,
            "nodes_after": delta.nodes_after,
            "matched": delta.matched.len(),
            "new": delta.new.len(),
            "removed": delta.removed.len(),
            "flagged": delta.flagged(),
        },
    });
    format!("{object}\n")
}
