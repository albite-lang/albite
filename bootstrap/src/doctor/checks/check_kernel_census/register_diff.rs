//! The corpus census's register diff (ADR 2.10.26o D4): the gate's own
//! verdict per term-mode theorem, and the register's rows, in; one category
//! per row or disagreement, and the verdict line, out.
//!
//! Pure. The verdicts come from `elaborate::kernel::term_verdicts`, which is
//! the build's gate code, so the diff cannot disagree with what a build
//! enforces (ADR 2.10.26d D10); this file only maps each [`GateVerdict`] onto
//! D4's table and accounts for the rows no verdict reached.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::elaborate::kernel::{Exemption, GateVerdict, Violation};

/// A theorem as the diff keys it: its defining file's register key and its
/// name.
pub type TheoremKey = (String, String);

/// What the corpus run saw of one theorem from one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// A term-mode theorem: the gate's verdict over it.
    Decided(GateVerdict),
    /// A `by` block: the gate never consults a row for one (D4).
    Tactic,
}

/// What became of a file a register row names (D3, D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// A built file of an entry that elaborated.
    Censused,
    /// Only reached by entries that did not elaborate.
    DoesNotElaborate,
    /// A `<stem>_test.tg` companion: never an entry (D2).
    Companion,
    /// Only reached by entries `--proof-bearing` skipped (ADR 3.10.26s D4).
    Skipped,
}

/// Why a row was not examined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotExaminedWhy {
    OutsideCorpus,
    DoesNotElaborate,
    Companion,
    Hole,
    Tactic,
    Skipped,
}

impl NotExaminedWhy {
    fn label(self) -> &'static str {
        match self {
            NotExaminedWhy::OutsideCorpus => "outside the corpus",
            NotExaminedWhy::DoesNotElaborate => "does not elaborate",
            NotExaminedWhy::Companion => "companion",
            NotExaminedWhy::Hole => "hole",
            NotExaminedWhy::Tactic => "tactic",
            NotExaminedWhy::Skipped => "skipped: no proof item recognised",
        }
    }
}

/// D4's categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiffCategory {
    Matched,
    MissingRow,
    Stale,
    ClassMismatch,
    Absent,
    NotExamined(NotExaminedWhy),
    /// Two entries reached the theorem and the gate decided them differently.
    EntriesDisagree,
}

impl DiffCategory {
    fn label(self) -> &'static str {
        match self {
            DiffCategory::Matched => "matched",
            DiffCategory::MissingRow => "missing row",
            DiffCategory::Stale => "stale",
            DiffCategory::ClassMismatch => "class mismatch",
            DiffCategory::Absent => "absent",
            DiffCategory::NotExamined(_) => "not examined",
            DiffCategory::EntriesDisagree => "entries disagree",
        }
    }
}

/// One line of the diff: a theorem, its category and what was observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub file: String,
    pub theorem: String,
    pub category: DiffCategory,
    pub detail: String,
}

impl DiffLine {
    /// `<file>::<theorem> → <category> (<detail>)` (D5).
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{}::{} → {} ({})",
            self.file,
            self.theorem,
            self.category.label(),
            self.detail
        )
    }
}

/// The diff: every line, and how many register rows it was taken over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegisterDiff {
    pub rows: usize,
    pub lines: Vec<DiffLine>,
}

/// Map one verdict onto D4's table: `None` for the arms that are in no
/// category (`Pass`, and a `SorryStatus` — the caller names a hole's row).
#[must_use]
pub fn categorise(verdict: &GateVerdict, has_row: bool) -> Option<(DiffCategory, String)> {
    match verdict {
        GateVerdict::Pass | GateVerdict::SorryStatus => None,
        GateVerdict::Exempt(class) => Some((DiffCategory::Matched, class.label().to_string())),
        GateVerdict::Refused { cause, class } => {
            let label = GateVerdict::refusal_label(*cause, *class).to_string();
            let category = if has_row {
                DiffCategory::ClassMismatch
            } else {
                DiffCategory::MissingRow
            };
            Some((category, label))
        }
        GateVerdict::RegisterViolation(violation) => Some(categorise_violation(violation)),
    }
}

fn categorise_violation(violation: &Violation) -> (DiffCategory, String) {
    match violation {
        Violation::Drifted { observed, .. } => (DiffCategory::ClassMismatch, observed.clone()),
        Violation::CitesMismatch { cited, .. } => (
            DiffCategory::ClassMismatch,
            format!("cites {}", cited.join(", ")),
        ),
        Violation::CitesWithoutRow { cited } => (
            DiffCategory::MissingRow,
            format!("cites {}", cited.join(", ")),
        ),
        Violation::Stale => (DiffCategory::Stale, "admitted".to_string()),
        Violation::CitesCleared => (
            DiffCategory::Stale,
            "admitted, cites nothing exempt".to_string(),
        ),
    }
}

/// The diff over every observation and every row (D4).
///
/// `observed` holds, per theorem, each entry that reached it with what that
/// entry saw; `files` the status of every file the run reached.
#[must_use]
pub fn register_diff(
    rows: &[Exemption],
    observed: &BTreeMap<TheoremKey, Vec<(String, Observation)>>,
    files: &BTreeMap<String, FileStatus>,
) -> RegisterDiff {
    let row_keys: BTreeSet<(&str, &str)> = rows
        .iter()
        .map(|row| (row.file.as_str(), row.theorem.as_str()))
        .collect();
    let mut lines = Vec::new();
    for ((file, theorem), seen) in observed {
        let has_row = row_keys.contains(&(file.as_str(), theorem.as_str()));
        let line = |category, detail| DiffLine {
            file: file.clone(),
            theorem: theorem.clone(),
            category,
            detail,
        };
        let per_entry: Vec<(&String, Option<(DiffCategory, String)>)> = seen
            .iter()
            .map(|(entry, observation)| (entry, theorem_category(observation, has_row)))
            .collect();
        let distinct: BTreeSet<&Option<(DiffCategory, String)>> =
            per_entry.iter().map(|(_, category)| category).collect();
        if distinct.len() > 1 {
            for (entry, category) in &per_entry {
                let said = category
                    .as_ref()
                    .map_or("no category".to_string(), |(c, d)| {
                        format!("{}: {d}", c.label())
                    });
                lines.push(line(
                    DiffCategory::EntriesDisagree,
                    format!("via {entry}, {said}"),
                ));
            }
        } else if let Some(Some((category, detail))) = distinct.into_iter().next() {
            lines.push(line(*category, detail.clone()));
        }
    }
    for row in rows {
        let key = (row.file.clone(), row.theorem.clone());
        if observed.contains_key(&key) {
            continue;
        }
        let category = match files.get(&row.file) {
            Some(FileStatus::Censused) => DiffCategory::Absent,
            Some(FileStatus::DoesNotElaborate) => {
                DiffCategory::NotExamined(NotExaminedWhy::DoesNotElaborate)
            }
            Some(FileStatus::Companion) => DiffCategory::NotExamined(NotExaminedWhy::Companion),
            Some(FileStatus::Skipped) => DiffCategory::NotExamined(NotExaminedWhy::Skipped),
            None => DiffCategory::NotExamined(NotExaminedWhy::OutsideCorpus),
        };
        let detail = match category {
            DiffCategory::NotExamined(why) => why.label().to_string(),
            _ => "no such theorem".to_string(),
        };
        lines.push(DiffLine {
            file: row.file.clone(),
            theorem: row.theorem.clone(),
            category,
            detail,
        });
    }
    lines.sort_by(|a, b| (&a.file, &a.theorem, &a.detail).cmp(&(&b.file, &b.theorem, &b.detail)));
    RegisterDiff {
        rows: rows.len(),
        lines,
    }
}

/// One entry's observation of a theorem, as a category: a hole or a tactic
/// proof with a row is *not examined*; without one, in no category.
fn theorem_category(observation: &Observation, has_row: bool) -> Option<(DiffCategory, String)> {
    let skipped = |why: NotExaminedWhy| {
        has_row.then(|| (DiffCategory::NotExamined(why), why.label().to_string()))
    };
    match observation {
        Observation::Tactic => skipped(NotExaminedWhy::Tactic),
        Observation::Decided(GateVerdict::SorryStatus) => skipped(NotExaminedWhy::Hole),
        Observation::Decided(verdict) => categorise(verdict, has_row),
    }
}

impl RegisterDiff {
    fn count(&self, wanted: impl Fn(DiffCategory) -> bool) -> usize {
        self.lines
            .iter()
            .filter(|line| wanted(line.category))
            .count()
    }

    fn count_of(&self, category: DiffCategory) -> usize {
        self.count(|c| c == category)
    }

    fn not_examined(&self) -> usize {
        self.count(|c| matches!(c, DiffCategory::NotExamined(_)))
    }

    /// The counts line, then the verdict line, then — under MISMATCH or
    /// INCOMPLETE — one line per non-matched row (D5).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "  register:  rows {} · matched {} · missing {} · stale {} · absent {} · class mismatch {} · not examined {}",
            self.rows,
            self.count_of(DiffCategory::Matched),
            self.count_of(DiffCategory::MissingRow),
            self.count_of(DiffCategory::Stale),
            self.count_of(DiffCategory::Absent),
            self.count_of(DiffCategory::ClassMismatch),
            self.not_examined(),
        );
        let disagree = self.count_of(DiffCategory::EntriesDisagree);
        if disagree > 0 {
            let _ = write!(out, " · entries disagree {disagree}");
        }
        out.push('\n');
        let verdict = self.verdict_line();
        let _ = writeln!(out, "  {verdict}");
        if verdict != "register: matches" {
            for line in self
                .lines
                .iter()
                .filter(|l| l.category != DiffCategory::Matched)
            {
                let _ = writeln!(out, "    {}", line.render());
            }
        }
        out
    }

    /// `register: matches`, `register: MISMATCH (…)` — which outranks — or
    /// `register: INCOMPLETE (n row(s) not examined)` (D5).
    #[must_use]
    pub fn verdict_line(&self) -> String {
        let mismatches: Vec<String> = [
            (DiffCategory::MissingRow, "missing"),
            (DiffCategory::Stale, "stale"),
            (DiffCategory::Absent, "absent"),
            (DiffCategory::ClassMismatch, "class mismatch"),
            (DiffCategory::EntriesDisagree, "entries disagree"),
        ]
        .into_iter()
        .filter_map(|(category, name)| {
            let n = self.count_of(category);
            (n > 0).then(|| format!("{n} {name}"))
        })
        .collect();
        if !mismatches.is_empty() {
            return format!("register: MISMATCH ({})", mismatches.join(", "));
        }
        match self.not_examined() {
            0 => "register: matches".to_string(),
            n => format!("register: INCOMPLETE ({n} row(s) not examined)"),
        }
    }
}
