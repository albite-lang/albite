//! `albite diff kernel-census <before> <after>` — two saved
//! `kernel-census --all-rows` reports joined by definition name (ADR
//! 4.10.26a).
//!
//! Each kernel ADR re-measures the census before and after its change; this
//! is the join that every one of them otherwise re-writes as a scratch
//! script. Cost 1: it elaborates nothing, so it compares runs from different
//! commits (D1).
//!
//! A move is a change of PRIMARY cause (D3), because the primary is what the
//! census tallies. A secondary-only change is listed apart and never counted.
//!
//! The parser, the join and both renderers are pure over strings (D5); the
//! shell below only reads two files and prints.

#[cfg(test)]
mod output_tests;
mod parse;
mod render;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use albite_bootstrap::elaborate::kernel::Cause;

pub use parse::{parse_census_report, CensusReport, ReportError};
pub use render::{render_diff_json, render_diff_text};

/// Two reports, joined (D3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CensusDiff {
    /// Names whose primary cause changed, per (from, to) pair, each sorted.
    /// Keyed by the causes' positions in `Cause::ALL`, so iteration is D4's
    /// order.
    pub moves: BTreeMap<(usize, usize), Vec<String>>,
    /// Names whose primary held and whose secondary changed, sorted.
    pub secondary_only: Vec<SecondaryChange>,
    /// Names in the before report only, sorted.
    pub only_before: Vec<String>,
    /// Names in the after report only, sorted.
    pub only_after: Vec<String>,
}

/// A row whose primary cause held and whose secondary did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecondaryChange {
    pub name: String,
    pub primary: Cause,
    pub before: Option<Cause>,
    pub after: Option<Cause>,
}

impl CensusDiff {
    /// Every (from, to, names) move, in D4's order.
    pub fn move_groups(&self) -> impl Iterator<Item = (Cause, Cause, &[String])> {
        self.moves
            .iter()
            .map(|(&(from, to), names)| (Cause::ALL[from], Cause::ALL[to], names.as_slice()))
    }

    /// How many rows changed primary cause.
    #[must_use]
    pub fn moved(&self) -> usize {
        self.moves.values().map(Vec::len).sum()
    }
}

/// The position of `cause` in `Cause::ALL` — the census's own table order.
fn cause_order(cause: Cause) -> usize {
    Cause::ALL
        .iter()
        .position(|&c| c == cause)
        .expect("Cause::ALL lists every cause")
}

/// Join two parsed reports by name (D3). Both maps are name-ordered, so every
/// list comes out sorted.
#[must_use]
pub fn join_reports(before: &CensusReport, after: &CensusReport) -> CensusDiff {
    let mut diff = CensusDiff::default();
    for (name, old) in before {
        let Some(new) = after.get(name) else {
            diff.only_before.push(name.clone());
            continue;
        };
        if old.primary != new.primary {
            diff.moves
                .entry((cause_order(old.primary), cause_order(new.primary)))
                .or_default()
                .push(name.clone());
        } else if old.secondary != new.secondary {
            diff.secondary_only.push(SecondaryChange {
                name: name.clone(),
                primary: old.primary,
                before: old.secondary,
                after: new.secondary,
            });
        }
    }
    diff.only_after = after
        .keys()
        .filter(|name| !before.contains_key(*name))
        .cloned()
        .collect();
    diff
}

/// Why no comparison was made. Both arms are exit 2 (D2, D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffFailure {
    /// A file could not be read.
    Unreadable { path: String, reason: String },
    /// A file was read but is not a complete `--all-rows` report.
    Invalid { path: String, error: ReportError },
}

impl std::fmt::Display for DiffFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffFailure::Unreadable { path, reason } => {
                write!(f, "error: cannot read {path}: {reason}")
            }
            DiffFailure::Invalid { path, error } => {
                write!(
                    f,
                    "error: {path} is not a complete kernel census report: {error}"
                )
            }
        }
    }
}

/// The exit code for an outcome: 0 for any comparison, however much moved —
/// this is a census, not a gate — and 2 when there was nothing sound to
/// compare, so bad input never reads as "nothing moved" (D5).
#[must_use]
pub fn exit_code(outcome: &Result<CensusDiff, DiffFailure>) -> u8 {
    match outcome {
        Ok(_) => 0,
        Err(DiffFailure::Unreadable { .. } | DiffFailure::Invalid { .. }) => 2,
    }
}

/// Parse one report's text, naming `path` in a refusal.
///
/// # Errors
///
/// [`DiffFailure::Invalid`] when the text is not a complete report.
pub fn parse_named(path: &str, text: &str) -> Result<CensusReport, DiffFailure> {
    parse_census_report(text).map_err(|error| DiffFailure::Invalid {
        path: path.to_string(),
        error,
    })
}

/// Join two reports' texts, before first.
///
/// # Errors
///
/// The first report that does not parse, before checked first.
pub fn diff_report_texts(
    (before_path, before_text): (&str, &str),
    (after_path, after_text): (&str, &str),
) -> Result<CensusDiff, DiffFailure> {
    let before = parse_named(before_path, before_text)?;
    let after = parse_named(after_path, after_text)?;
    Ok(join_reports(&before, &after))
}

fn read_report(path: &Path) -> Result<String, DiffFailure> {
    std::fs::read_to_string(path).map_err(|e| DiffFailure::Unreadable {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

/// Entry point for `albite diff kernel-census <before> <after> [--json]`.
pub fn cmd_diff_kernel_census(before: &Path, after: &Path, json: bool) -> ExitCode {
    let before_name = before.display().to_string();
    let after_name = after.display().to_string();
    let outcome = read_report(before).and_then(|before_text| {
        let after_text = read_report(after)?;
        diff_report_texts((&before_name, &before_text), (&after_name, &after_text))
    });
    match &outcome {
        Ok(diff) if json => println!("{}", render_diff_json(diff)),
        Ok(diff) => print!("{}", render_diff_text(&before_name, &after_name, diff)),
        Err(failure) => eprintln!("{failure}"),
    }
    ExitCode::from(exit_code(&outcome))
}
