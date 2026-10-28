//! `albite doctor check sorry-sites <dir>` — the census over every `.tg`
//! under a directory, one entry at a time (ADR 15.10.26a).
//!
//! - **Expansion** (D1): `utils::walk_tg_tree`, the walk `diff
//!   bootstrap-selfhost-check` uses, lifted to a shared helper so both read
//!   the same entries in the same order.
//! - **Each entry** (D2) elaborates on its own, cold and quiet
//!   (`driver::elaborate_project_cold`): nothing is written under the
//!   corpus's `.albite/`, and a failure carries its first error's header
//!   instead of rendering hundreds of diagnostics.
//! - **Totals** (D3) are sums of each entry's census, not distinct sites.
//! - **Exit** (D4): 0 on a complete census, 1 after the full report when an
//!   entry did not elaborate or a directory could not be listed, 2 when there
//!   is nothing to census.

use std::path::Path;

use serde::Serialize;

use super::{census, SorryCensus};
use crate::driver;
use crate::utils::{list_directory, walk_tg_tree, TgTree};

/// What became of one entry: its census, or the first error header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EntryOutcome {
    #[serde(rename = "census")]
    Censused(SorryCensus),
    #[serde(rename = "error")]
    DidNotElaborate(String),
}

/// One entry file and its outcome (D3a).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorpusEntry {
    pub entry: String,
    #[serde(flatten)]
    pub outcome: EntryOutcome,
}

/// A directory the walk could not list (D4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadableDir {
    pub path: String,
    pub error: String,
}

/// The corpus totals: what was read, and each class summed over entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CorpusTotals {
    pub read: usize,
    pub elaborated: usize,
    pub with_holes: usize,
    pub authored: usize,
    pub synthesised: usize,
    pub unclassified: usize,
}

/// The whole directory census, ready to render or serialise (D3a).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorpusReport {
    pub root: String,
    pub entries: Vec<CorpusEntry>,
    pub unreadable: Vec<UnreadableDir>,
    pub totals: CorpusTotals,
}

impl CorpusReport {
    /// D4: a report with a failed entry or an unreadable directory is
    /// incomplete, so exit 1; otherwise findings are not failures.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        let failed = self
            .entries
            .iter()
            .any(|e| matches!(e.outcome, EntryOutcome::DidNotElaborate(_)));
        u8::from(failed || !self.unreadable.is_empty())
    }
}

/// The real run: walk `root` on the filesystem and census each entry cold.
///
/// # Errors
///
/// As [`census_corpus`]: no `.tg` entry under `root`.
pub fn census_directory(root: &Path) -> Result<CorpusReport, String> {
    let tree = walk_tg_tree(root, &mut list_directory);
    census_corpus(root, &tree, &mut elaborate_entry)
}

/// One entry, elaborated in isolation (D2): its census, or the first line of
/// its first error.
fn elaborate_entry(entry: &Path) -> Result<SorryCensus, String> {
    driver::elaborate_project_cold(entry, 20)
        .map(|project| census(&project))
        .map_err(|why| why.to_string().lines().next().unwrap_or("").to_string())
}

/// Census every entry of `tree`, found under `root`, through the injected
/// `elaborate` — the real one is [`elaborate_entry`], a test's may be
/// anything — and sum the results.
///
/// # Errors
///
/// A tree with no `.tg` entry (D3): a census of nothing is not a clean one.
pub fn census_corpus(
    root: &Path,
    tree: &TgTree,
    elaborate: &mut dyn FnMut(&Path) -> Result<SorryCensus, String>,
) -> Result<CorpusReport, String> {
    if tree.files.is_empty() {
        let mut why = format!(
            "no `.tg` entries under {} — a census of nothing is not a clean one",
            root.display()
        );
        for (dir, error) in &tree.unreadable {
            why.push_str(&format!("\n  unreadable: {}: {error}", dir.display()));
        }
        return Err(why);
    }
    let mut totals = CorpusTotals::default();
    let mut entries = Vec::with_capacity(tree.files.len());
    for relative in &tree.files {
        let path = root.join(relative);
        let outcome = match elaborate(&path) {
            Ok(census) => {
                totals.elaborated += 1;
                totals.with_holes += usize::from(!census.rows.is_empty());
                totals.authored += census.authored;
                totals.synthesised += census.synthesised;
                totals.unclassified += census.unclassified;
                EntryOutcome::Censused(census)
            }
            Err(header) => EntryOutcome::DidNotElaborate(header),
        };
        entries.push(CorpusEntry {
            entry: path.display().to_string(),
            outcome,
        });
    }
    totals.read = entries.len();
    Ok(CorpusReport {
        root: root.display().to_string(),
        entries,
        unreadable: tree
            .unreadable
            .iter()
            .map(|(dir, error)| UnreadableDir {
                path: dir.display().to_string(),
                error: error.clone(),
            })
            .collect(),
        totals,
    })
}

/// The per-entry row: its holes by class, `clean`, or why it did not elaborate.
fn render_entry(entry: &CorpusEntry) -> String {
    match &entry.outcome {
        EntryOutcome::Censused(census) if census.rows.is_empty() => format!(
            "  {}  clean ({} definition(s))\n",
            entry.entry, census.examined
        ),
        EntryOutcome::Censused(census) => format!(
            "  {}  {} of {} definition(s) carry a sorry: {} authored, {} synthesised, {} unclassified\n",
            entry.entry,
            census.rows.len(),
            census.examined,
            census.authored,
            census.synthesised,
            census.unclassified
        ),
        EntryOutcome::DidNotElaborate(header) => {
            format!("  {}  did not elaborate: {header}\n", entry.entry)
        }
    }
}

/// Human-readable report: one row per entry, each unreadable directory, then
/// the `entries:` line and the summed totals (D3).
#[must_use]
pub fn render_corpus(report: &CorpusReport) -> String {
    let mut out = format!("sorry-sites over {}:\n\n", report.root);
    for entry in &report.entries {
        out.push_str(&render_entry(entry));
    }
    if !report.unreadable.is_empty() {
        out.push('\n');
        for dir in &report.unreadable {
            out.push_str(&format!("  unreadable: {}: {}\n", dir.path, dir.error));
        }
    }
    let totals = &report.totals;
    out.push_str(&format!(
        "\nentries: {} read, {} elaborated, {} carry a hole\n",
        totals.read, totals.elaborated, totals.with_holes
    ));
    out.push_str(&format!(
        "totals (summed over entries): {} authored, {} synthesised, {} unclassified\n",
        totals.authored, totals.synthesised, totals.unclassified
    ));
    out
}

// Tests: corpus_tests.rs — tempdir corpora, the injected walk, the JSON shape
#[cfg(test)]
#[path = "corpus_tests.rs"]
mod tests;
