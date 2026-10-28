//! `term-size --baseline` (ADR 15.10.26g D5, D6): a saved census joined to
//! this run's rows by `(module, name)` and ranked by growth.
//!
//! Pure over its inputs — the baseline's text and this run's rows — so every
//! refusal, the order and the flag rule are assertable without elaborating
//! anything; the shell in `mod.rs` only reads the file.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{ProofTermSize, SizeRow};

/// The flag thresholds (D6). A matched row is flagged when its node delta is
/// at least `min_delta` AND its ratio at least `max_growth`; a `new` row when
/// its node count is at least `min_delta`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub max_growth: f64,
    pub min_delta: usize,
}

impl Thresholds {
    /// The defaults Step 3a calibrated against committed windows of `main.tg`
    /// (§6 records the measured noise beside them).
    pub const DEFAULT: Thresholds = Thresholds {
        max_growth: 1.5,
        min_delta: 2000,
    };
}

/// A saved report this command wrote with `--json`: a plain census, or a
/// delta report, whose `"rows"` are its own run's census (D5).
#[derive(Debug, Clone, PartialEq)]
pub struct Baseline {
    pub population: String,
    pub rows: Vec<SizeRow>,
}

/// Parse a baseline report, refusing anything a comparison would misread as
/// "nothing grew" (D5, after 4.10.26a D2).
///
/// # Errors
///
/// Not one JSON object; an `{"error": …}` report; no `"population"` (a
/// 13.10.26c-era report) or no `"rows"`; a row missing a field; a duplicated
/// `(module, name)`. Each reason names the offending row.
pub fn parse_baseline(text: &str) -> Result<Baseline, String> {
    let object: Value =
        serde_json::from_str(text).map_err(|e| format!("not one JSON object ({e})"))?;
    let Some(object) = object.as_object() else {
        return Err("not one JSON object".into());
    };
    if let Some(error) = object.get("error") {
        return Err(format!("it is an error report ({error})"));
    }
    let population = object
        .get("population")
        .and_then(Value::as_str)
        .ok_or("it has no \"population\" (a report from before ADR 15.10.26g?)")?
        .to_string();
    let rows_json = object
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("it has no \"rows\" array")?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(rows_json.len());
    for (index, row_json) in rows_json.iter().enumerate() {
        let row = parse_row(row_json).map_err(|reason| format!("row {index}: {reason}"))?;
        if !seen.insert((row.module.clone(), row.name.clone())) {
            return Err(format!(
                "row {index}: {}::{} appears twice",
                row.module, row.name
            ));
        }
        rows.push(row);
    }
    Ok(Baseline { population, rows })
}

/// One row in 13.10.26c D6's shape, `case_arms` included. A row's `proof`
/// flag is not in the JSON and is never read by the delta.
fn parse_row(row: &Value) -> Result<SizeRow, String> {
    let text = |field: &str| {
        row.get(field)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(format!("missing \"{field}\""))
    };
    let module = text("module")?;
    let name = text("name")?;
    let named = |reason: String| format!("{module}::{name}: {reason}");
    let axiom = row
        .get("axiom")
        .and_then(Value::as_bool)
        .ok_or_else(|| named("missing \"axiom\"".into()))?;
    let count = |field: &str| {
        row.get(field)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| named(format!("missing \"{field}\"")))
    };
    let size = if axiom {
        None
    } else {
        Some(ProofTermSize {
            subst: count("subst")?,
            refl: count("refl")?,
            global_occurrences: count("global_occurrences")?,
            nodes: count("nodes")?,
            case_arms: count("case_arms")?,
        })
    };
    Ok(SizeRow {
        module,
        name,
        proof: false,
        size,
    })
}

/// A definition present on both sides. An axiom on either side is matched
/// but carries no delta (D5).
#[derive(Debug, Clone, PartialEq)]
pub struct MatchedRow {
    pub module: String,
    pub name: String,
    pub before: Option<ProofTermSize>,
    pub after: Option<ProofTermSize>,
    pub flagged: bool,
}

impl MatchedRow {
    /// `after.nodes − before.nodes`, signed.
    #[must_use]
    pub fn delta(&self) -> Option<i64> {
        let (before, after) = (self.before?, self.after?);
        Some(signed(after.nodes) - signed(before.nodes))
    }

    /// `after.nodes / before.nodes`.
    #[must_use]
    pub fn ratio(&self) -> Option<f64> {
        let (before, after) = (self.before?, self.after?);
        Some(as_float(after.nodes) / as_float(before.nodes))
    }
}

fn signed(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

#[allow(clippy::cast_precision_loss)] // Reason: node counts stay far below 2^52, so the ratio is exact
fn as_float(count: usize) -> f64 {
    count as f64
}

/// The comparison (D5, D6): matched rows in growth order, rows on one side
/// only, and this run's rows by size for `largest now:`.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    pub matched: Vec<MatchedRow>,
    /// This run's rows the baseline lacks, by `nodes` descending, with
    /// whether each is flagged.
    pub new: Vec<(SizeRow, bool)>,
    /// The baseline's rows this run lacks, by `(module, name)`.
    pub removed: Vec<SizeRow>,
    /// This run's rows by `nodes` descending; never flagged (D6).
    pub largest: Vec<SizeRow>,
    pub nodes_before: usize,
    pub nodes_after: usize,
}

impl Delta {
    /// Matched and new rows flagged; the exit code reads only this.
    #[must_use]
    pub fn flagged(&self) -> usize {
        self.matched.iter().filter(|row| row.flagged).count()
            + self.new.iter().filter(|(_, flagged)| *flagged).count()
    }
}

/// Join `after` to the baseline by `(module, name)` and rank it.
#[must_use]
pub fn delta_rows(baseline: &Baseline, after: &[SizeRow], thresholds: Thresholds) -> Delta {
    let key = |row: &SizeRow| (row.module.clone(), row.name.clone());
    let before: BTreeMap<_, _> = baseline.rows.iter().map(|row| (key(row), row)).collect();
    let after_keys: BTreeSet<_> = after.iter().map(key).collect();
    let mut matched = Vec::new();
    let mut new = Vec::new();
    for row in after {
        match before.get(&key(row)) {
            Some(old) => {
                let mut joined = MatchedRow {
                    module: row.module.clone(),
                    name: row.name.clone(),
                    before: old.size,
                    after: row.size,
                    flagged: false,
                };
                joined.flagged = matched_is_flagged(&joined, thresholds);
                matched.push(joined);
            }
            None => {
                let flagged = row.size.is_some_and(|s| s.nodes >= thresholds.min_delta);
                new.push((row.clone(), flagged));
            }
        }
    }
    matched.sort_by(growth_order);
    new.sort_by(|(a, _), (b, _)| size_order(a, b));
    let removed = baseline
        .rows
        .iter()
        .filter(|row| !after_keys.contains(&key(row)))
        .cloned()
        .collect();
    Delta {
        matched,
        new,
        removed,
        largest: by_size(after),
        nodes_before: nodes(&baseline.rows),
        nodes_after: nodes(after),
    }
}

/// Both thresholds, together: a ratio alone flags a three-node helper that
/// became nine, a delta alone a huge definition that grew 2% (D6).
fn matched_is_flagged(row: &MatchedRow, thresholds: Thresholds) -> bool {
    match (row.delta(), row.ratio()) {
        (Some(delta), Some(ratio)) => {
            delta >= signed(thresholds.min_delta) && ratio >= thresholds.max_growth
        }
        _ => false,
    }
}

/// `delta` descending, then `ratio` descending, then `(module, name)`; an
/// axiom row, with no delta, sorts after every counted one.
fn growth_order(a: &MatchedRow, b: &MatchedRow) -> Ordering {
    let rank = |row: &MatchedRow| (row.delta(), row.ratio());
    let (a_rank, b_rank) = (rank(a), rank(b));
    b_rank
        .0
        .cmp(&a_rank.0)
        .then_with(|| b_rank.1.partial_cmp(&a_rank.1).unwrap_or(Ordering::Equal))
        .then_with(|| (&a.module, &a.name).cmp(&(&b.module, &b.name)))
}

/// `nodes` descending, then `(module, name)`; an axiom counts as zero.
pub(super) fn size_order(a: &SizeRow, b: &SizeRow) -> Ordering {
    let nodes = |row: &SizeRow| row.size.map_or(0, |s| s.nodes);
    nodes(b)
        .cmp(&nodes(a))
        .then_with(|| (&a.module, &a.name).cmp(&(&b.module, &b.name)))
}

/// The rows by [`size_order`], for `--top` and `largest now:`.
#[must_use]
pub fn by_size(rows: &[SizeRow]) -> Vec<SizeRow> {
    let mut sorted = rows.to_vec();
    sorted.sort_by(size_order);
    sorted
}

fn nodes(rows: &[SizeRow]) -> usize {
    rows.iter()
        .filter_map(|row| row.size)
        .map(|s| s.nodes)
        .sum()
}
