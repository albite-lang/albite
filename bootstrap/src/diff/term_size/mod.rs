//! `albite diff term-size <before.json> <after.json>` — two saved
//! `term-size --json` reports compared by definition (ADR 16.10.26w).
//!
//! `term-size --baseline` always censuses the tree for its after side; this
//! is the front end for every comparison whose two sides are already on disk
//! (two committed bases, two binaries over one tree). Cost 1: it elaborates
//! nothing (D1).
//!
//! The parser, the join, the flag rule and both renderers are 15.10.26g's,
//! reused unchanged (D3), so the two commands cannot disagree. This file is
//! the shell: read both files, parse each naming its side, refuse a
//! population mismatch, compare, render, map the exit (D2, D4).

#[cfg(test)]
mod tests;

use std::path::Path;

use albite_bootstrap::doctor::checks::check_proof_term_size::delta::{
    delta_rows, parse_baseline, Baseline,
};
use albite_bootstrap::doctor::checks::check_proof_term_size::{
    exit_code, render_delta_json, render_delta_text, DeltaHeader, Population, Thresholds,
    DELTA_DEFAULT_TOP,
};
use serde_json::json;

/// Everything `diff term-size` was asked beside the two paths (D1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TermSizeDiffRequest {
    pub thresholds: Thresholds,
    pub top: Option<usize>,
    pub json: bool,
}

impl TermSizeDiffRequest {
    /// The request the flags spell, each absent knob at `term-size`'s
    /// default (15.10.26g D6).
    #[must_use]
    pub fn from_flags(
        max_growth: Option<f64>,
        min_delta: Option<usize>,
        top: Option<usize>,
        json: bool,
    ) -> TermSizeDiffRequest {
        let defaults = Thresholds::DEFAULT;
        TermSizeDiffRequest {
            thresholds: Thresholds {
                max_growth: max_growth.unwrap_or(defaults.max_growth),
                min_delta: min_delta.unwrap_or(defaults.min_delta),
            },
            top,
            json,
        }
    }
}

/// One side of the comparison: its path as printed, and its text or the
/// reason it could not be read.
pub type SavedReport<'a> = (&'a str, Result<String, String>);

/// Entry point for `albite diff term-size <before> <after>`: prints, and
/// returns the exit code as a `u8` the caller wraps in `ExitCode`, so the
/// code the shell sees is assertable (an `ExitCode` has no `PartialEq`).
#[must_use]
pub fn cmd_diff_term_size(before: &Path, after: &Path, request: &TermSizeDiffRequest) -> u8 {
    let (stdout, stderr, code) = run_diff_term_size(before, after, request);
    print!("{stdout}");
    eprint!("{stderr}");
    code
}

/// Read both files and compare them — `(stdout, stderr, code)`, so the whole
/// command, file reads included, is assertable.
#[must_use]
pub fn run_diff_term_size(
    before: &Path,
    after: &Path,
    request: &TermSizeDiffRequest,
) -> (String, String, u8) {
    let read = |path: &Path| std::fs::read_to_string(path).map_err(|e| e.to_string());
    let (before_name, after_name) = (before.display().to_string(), after.display().to_string());
    diff_term_size_output(
        (&before_name, read(before)),
        (&after_name, read(after)),
        request,
    )
}

/// The comparison over two reports' texts, pure (D2–D4).
#[must_use]
pub fn diff_term_size_output(
    before: SavedReport<'_>,
    after: SavedReport<'_>,
    request: &TermSizeDiffRequest,
) -> (String, String, u8) {
    compare(before, after, request).unwrap_or_else(|refusal| {
        let stdout = if request.json {
            format!("{}\n", json!({ "error": refusal }))
        } else {
            String::new()
        };
        (
            stdout,
            format!("error: {refusal}\n"),
            exit_code(true, true, 0),
        )
    })
}

/// Parse one side, naming it and its path in a refusal (D2).
fn parse_side(side: &str, (path, text): SavedReport<'_>) -> Result<Baseline, String> {
    let refuse = |reason: String| format!("{side} {path}: {reason}");
    parse_baseline(&text.map_err(refuse)?).map_err(refuse)
}

fn compare(
    before: SavedReport<'_>,
    after: SavedReport<'_>,
    request: &TermSizeDiffRequest,
) -> Result<(String, String, u8), String> {
    let (before_path, after_path) = (before.0, after.0);
    let old = parse_side("before", before)?;
    let new = parse_side("after", after)?;
    // A proof-only census against an all-definitions one would report every
    // partial definition as new (D2).
    if old.population != new.population {
        return Err(format!(
            "population differs: before {before_path} is \"{}\", after {after_path} is \"{}\"",
            old.population, new.population
        ));
    }
    let population = Population::from_label(&new.population).ok_or_else(|| {
        format!(
            "population \"{}\" in {before_path} and {after_path} is not one term-size writes",
            new.population
        )
    })?;
    let delta = delta_rows(&old, &new.rows, request.thresholds);
    let header = DeltaHeader {
        file: after_path,
        baseline: before_path,
        population,
        thresholds: request.thresholds,
        top: request.top.unwrap_or(DELTA_DEFAULT_TOP),
    };
    // The after side's rows, so the output is itself a baseline (D3).
    let report = if request.json {
        render_delta_json(&header, &delta, &new.rows)
    } else {
        render_delta_text(&header, &delta)
    };
    Ok((
        report,
        String::new(),
        exit_code(true, false, delta.flagged()),
    ))
}
