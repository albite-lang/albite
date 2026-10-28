//! The statistics, the comparison and the plan behind the gate (D3, D4, §2.1).

use crate::policy::Field;
use crate::record::RecordState;
use crate::{Bound, Samples, Verdict};

/// The median of `xs`; the mean of the middle two for an even count, and 0
/// for none (a caller never compares an empty arm: `compare` refuses it).
#[must_use]
pub fn median(xs: &[f64]) -> f64 {
    let sorted = sorted(xs);
    let n = sorted.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => sorted[n / 2],
        _ => f64::midpoint(sorted[n / 2 - 1], sorted[n / 2]),
    }
}

/// The interquartile spread over the median, in percent (D1): with five
/// sorted samples, `(x[3] − x[1]) / x[2]`. The quartiles sit one in from each
/// end per four samples, so one stray run cannot trip it.
#[must_use]
pub fn spread_pct(xs: &[f64]) -> f64 {
    let sorted = sorted(xs);
    let n = sorted.len();
    let mid = median(&sorted);
    if n == 0 || mid == 0.0 {
        return 0.0;
    }
    let inset = (n - 1) / 4;
    (sorted[n - 1 - inset] - sorted[inset]) / mid * 100.0
}

/// `albite-bench`'s `compare_memory` formula: the candidate's median over the
/// reference's, as a signed percentage.
#[must_use]
pub fn delta_pct(reference: &[f64], candidate: &[f64]) -> f64 {
    let base = median(reference);
    if base == 0.0 {
        return 0.0;
    }
    (median(candidate) - base) / base * 100.0
}

fn sorted(xs: &[f64]) -> Vec<f64> {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    v
}

/// One comparison: its verdict, its two deltas and the wider of the two arms'
/// spreads on each metric.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    pub verdict: Verdict,
    pub wall_pct: f64,
    pub rss_pct: f64,
    pub wall_spread_pct: f64,
    pub rss_spread_pct: f64,
}

/// D4: `check-failed` over `noisy` over `regressed` over `pass`. A failing run
/// is not a cost, and a noisy arm is not a measurement, so neither reaches the
/// bound.
#[must_use]
pub fn compare(reference: &Samples, candidate: &Samples, bound: &Bound) -> Comparison {
    let (ref_rss, cand_rss) = (reference.rss_f64(), candidate.rss_f64());
    let wall_spread_pct = spread_pct(&reference.wall_s).max(spread_pct(&candidate.wall_s));
    let rss_spread_pct = spread_pct(&ref_rss).max(spread_pct(&cand_rss));
    let wall_pct = delta_pct(&reference.wall_s, &candidate.wall_s);
    let rss_pct = delta_pct(&ref_rss, &cand_rss);
    let empty = reference.wall_s.is_empty() || candidate.wall_s.is_empty();
    let verdict = if empty || reference.any_failed() || candidate.any_failed() {
        Verdict::CheckFailed
    } else if wall_spread_pct > bound.noise_pct || rss_spread_pct > bound.noise_pct {
        Verdict::Noisy
    } else if wall_pct > bound.wall_pct || rss_pct > bound.rss_pct {
        Verdict::Regressed
    } else {
        Verdict::Pass
    };
    Comparison {
        verdict,
        wall_pct,
        rss_pct,
        wall_spread_pct,
        rss_spread_pct,
    }
}

/// Where a stored comparison leaves the gate (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredClass {
    /// Within the bound and below the margin: a pass on its own.
    Pass,
    /// At or above `wall_pct − stored_wall_margin_pct`, within the bound.
    Inconclusive,
    /// Past the bound, within the drift ceiling.
    PastBound,
    /// Past the drift ceiling on either metric.
    PastDriftCeiling,
    /// Noisy or failing: the comparison's own verdict stands.
    Stops(Verdict),
}

#[must_use]
pub fn classify_stored(stored: &Comparison, bound: &Bound) -> StoredClass {
    match stored.verdict {
        Verdict::Noisy | Verdict::CheckFailed => return StoredClass::Stops(stored.verdict),
        _ => {}
    }
    if stored.wall_pct > bound.drift_ceiling_pct || stored.rss_pct > bound.drift_ceiling_pct {
        StoredClass::PastDriftCeiling
    } else if stored.verdict == Verdict::Regressed {
        StoredClass::PastBound
    } else if stored.wall_pct >= bound.wall_pct - bound.stored_wall_margin_pct {
        StoredClass::Inconclusive
    } else {
        StoredClass::Pass
    }
}

/// What the gate does next (§2.1's arrows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Decided(Verdict),
    MeasureStored,
    RunAb,
}

/// The facts `plan` reads; every one is computed by an effect elsewhere.
#[derive(Debug, Clone, Copy)]
pub struct PlanInputs<'a> {
    pub applies: bool,
    pub record: &'a RecordState,
    pub stale: &'a [Field],
    /// `None` before `measure` has run.
    pub stored: Option<StoredClass>,
    /// `--no-ab`: the stored (file) comparison is the verdict.
    pub no_ab: bool,
    /// `--force-ab`: the A/B runs even after a stored pass (the A/A).
    pub force_ab: bool,
}

/// §2.1: not applicable, then a stored comparison when the record is present
/// and fresh, then the A/B for anything the stored arm cannot settle.
#[must_use]
pub fn plan(inputs: &PlanInputs<'_>) -> Next {
    if !inputs.applies {
        return Next::Decided(Verdict::NotApplicable);
    }
    let stored_usable = matches!(inputs.record, RecordState::Present(_)) && inputs.stale.is_empty();
    if !stored_usable {
        return Next::RunAb;
    }
    let Some(stored) = inputs.stored else {
        return Next::MeasureStored;
    };
    match stored {
        StoredClass::Stops(verdict) => Next::Decided(verdict),
        _ if inputs.force_ab => Next::RunAb,
        StoredClass::Pass => Next::Decided(Verdict::Pass),
        // Under `--no-ab` the file comparison is its own verdict: inside the
        // bound passes, past it regresses.
        StoredClass::Inconclusive if inputs.no_ab => Next::Decided(Verdict::Pass),
        _ if inputs.no_ab => Next::Decided(Verdict::Regressed),
        _ => Next::RunAb,
    }
}

/// D5's attribution: the A/B decides whose cost it is, and a clean A/B over a
/// failed stored comparison is drift, past the ceiling drift-exceeded.
#[must_use]
pub fn resolve_ab(stored: Option<StoredClass>, ab: &Comparison) -> Verdict {
    if ab.verdict != Verdict::Pass {
        return ab.verdict;
    }
    match stored {
        Some(StoredClass::PastBound) => Verdict::Drift,
        Some(StoredClass::PastDriftCeiling) => Verdict::DriftExceeded,
        _ => Verdict::Pass,
    }
}

/// What a verdict was decided against (D2, D11): only `stored` and `ab` are
/// close-out evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Stored,
    Ab,
    File,
}

impl Basis {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Basis::Stored => "stored",
            Basis::Ab => "ab",
            Basis::File => "file",
        }
    }
}

/// The one machine-readable line every run prints first (D4).
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictLine {
    pub verdict: Verdict,
    /// `None` for a verdict decided before any comparison ran.
    pub basis: Option<Basis>,
    /// The deciding comparison.
    pub decided_by: Option<Comparison>,
    /// The stored comparison, printed beside an A/B that followed it.
    pub stored: Option<Comparison>,
    pub record: Option<&'static str>,
    pub stale: Vec<Field>,
    pub base: String,
    pub tree: String,
}

impl VerdictLine {
    /// D4: owed for drift, drift-exceeded, an absent or malformed record, and
    /// a stale record that then passed.
    #[must_use]
    pub fn rebless_owed(&self) -> bool {
        match self.verdict {
            Verdict::Drift | Verdict::DriftExceeded => true,
            Verdict::Pass => self.record.is_some() || !self.stale.is_empty(),
            _ => self.record.is_some(),
        }
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut fields = vec![format!("verdict={}", self.verdict.name())];
        if let Some(basis) = self.basis {
            fields.push(format!("basis={}", basis.name()));
        }
        if let Some(c) = &self.decided_by {
            fields.push(format!("wall={}", signed(c.wall_pct)));
            fields.push(format!("rss={}", signed(c.rss_pct)));
        }
        if let (Some(Basis::Ab), Some(s)) = (self.basis, &self.stored) {
            fields.push(format!(
                "stored={}/{}",
                signed(s.wall_pct),
                signed(s.rss_pct)
            ));
        }
        if let Some(c) = &self.decided_by {
            fields.push(format!(
                "spread={:.1}%/{:.1}%",
                c.wall_spread_pct, c.rss_spread_pct
            ));
        }
        if let Some(label) = self.record {
            fields.push(format!("record={label}"));
        }
        if !self.stale.is_empty() {
            let names: Vec<&str> = self.stale.iter().map(|f| f.name()).collect();
            fields.push(format!("stale={}", names.join(",")));
        }
        fields.push(format!("base={}", self.base));
        fields.push(format!("tree={}", self.tree));
        if self.rebless_owed() {
            fields.push("rebless-owed".into());
        }
        format!("check-cost: {}", fields.join(" "))
    }
}

fn signed(pct: f64) -> String {
    format!("{pct:+.1}%")
}
