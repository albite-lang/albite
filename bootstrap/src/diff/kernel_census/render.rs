//! The comparison's two renderings (ADR 4.10.26a D4), built as values so
//! their order is assertable.

use std::fmt::Write as _;

use serde_json::{json, Value};

use albite_bootstrap::elaborate::kernel::Cause;

use super::CensusDiff;

fn secondary_label(cause: Option<Cause>) -> &'static str {
    cause.map_or("—", Cause::label)
}

/// The text report: the move matrix, the moved rows grouped by move, the
/// secondary-only changes, then the names only one report has.
#[must_use]
pub fn render_diff_text(before: &str, after: &str, diff: &CensusDiff) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "kernel census diff: {before} -> {after}");
    let _ = writeln!(out, "  moved:          {}", diff.moved());
    let _ = writeln!(out, "  secondary only: {}", diff.secondary_only.len());
    let _ = writeln!(out, "  only before:    {}", diff.only_before.len());
    let _ = writeln!(out, "  only after:     {}", diff.only_after.len());
    if !diff.moves.is_empty() {
        out.push_str("\nmoves (primary cause):\n");
        for (from, to, names) in diff.move_groups() {
            let pair = format!("{} -> {}", from.label(), to.label());
            let _ = writeln!(out, "  {pair:<50} {:>6}", names.len());
        }
        for (from, to, names) in diff.move_groups() {
            let _ = writeln!(out, "\n{} -> {}:", from.label(), to.label());
            push_names(&mut out, names);
        }
    }
    if !diff.secondary_only.is_empty() {
        out.push_str("\nsecondary only:\n");
        for change in &diff.secondary_only {
            let _ = writeln!(
                out,
                "  {}: {} / {} -> {}",
                change.name,
                change.primary.label(),
                secondary_label(change.before),
                secondary_label(change.after)
            );
        }
    }
    for (heading, names) in [
        ("only in before", &diff.only_before),
        ("only in after", &diff.only_after),
    ] {
        if !names.is_empty() {
            let _ = writeln!(out, "\n{heading}:");
            push_names(&mut out, names);
        }
    }
    out
}

fn push_names(out: &mut String, names: &[String]) {
    for name in names {
        let _ = writeln!(out, "  {name}");
    }
}

/// D4's `--json` object, in the text report's orders. A secondary that the
/// census did not compute is `null`.
#[must_use]
pub fn render_diff_json(diff: &CensusDiff) -> Value {
    let moves: Vec<Value> = diff
        .move_groups()
        .map(|(from, to, names)| {
            json!({
                "from": from.label(),
                "to": to.label(),
                "count": names.len(),
                "names": names,
            })
        })
        .collect();
    let secondary_only: Vec<Value> = diff
        .secondary_only
        .iter()
        .map(|change| {
            json!({
                "name": change.name,
                "before": change.before.map(Cause::label),
                "after": change.after.map(Cause::label),
            })
        })
        .collect();
    json!({
        "moves": moves,
        "secondary_only": secondary_only,
        "only_before": diff.only_before,
        "only_after": diff.only_after,
    })
}
