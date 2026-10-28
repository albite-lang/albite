//! The decision logic of `albite-dev check-cost` (ADR 15.10.26f D8).
//!
//! `albite-dev` is container-only, so neither `coverage-diff-gate` nor
//! `mutants-diff` scores it. Everything that DECIDES lives here instead, with
//! every input injected — record text, sample vectors, path lists, epochs and
//! environment structs — and `albite-dev` keeps only the effects: spawning,
//! `setrlimit`, `git`, the phase file and reading `/proc`.

pub mod phase;
pub mod policy;
pub mod record;
pub mod verdict;

#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod verdict_line_tests;
#[cfg(test)]
mod verdict_tests;

use serde::{Deserialize, Serialize};

/// One arm's measurements: a wall time, a peak RSS and an exit code per run.
///
/// A stored record keeps no exit codes — a bless refuses a failing sample —
/// so `exit_codes` may be empty, which reads as "every run exited 0".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Samples {
    pub wall_s: Vec<f64>,
    pub max_rss_kb: Vec<u64>,
    pub exit_codes: Vec<i32>,
}

impl Samples {
    /// The RSS samples as floats, for the shared statistics.
    #[must_use]
    pub fn rss_f64(&self) -> Vec<f64> {
        #[allow(clippy::cast_precision_loss)] // KiB counts are far below 2^52
        self.max_rss_kb.iter().map(|&kb| kb as f64).collect()
    }

    /// True when any run exited non-zero (D1: such a sample is not a cost).
    #[must_use]
    pub fn any_failed(&self) -> bool {
        self.exit_codes.iter().any(|&code| code != 0)
    }
}

/// The record's `[bound]` table (D3). Hand-edited, and only by an ADR (D6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bound {
    pub wall_pct: f64,
    pub rss_pct: f64,
    pub noise_pct: f64,
    pub drift_ceiling_pct: f64,
    pub stored_wall_margin_pct: f64,
    pub runs: u32,
    pub adr: String,
}

/// The record's `[environment]` table: what the stored numbers describe (D5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub kernel: String,
    pub os: String,
    pub cpu_model: String,
    pub llc: String,
    pub rustc: String,
    pub nproc: u32,
    pub mem_total_kb: u64,
}

/// The gate's verdict (D4). The line, not the exit, is what a child records:
/// GNU make collapses every failing exit to 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Drift,
    DriftExceeded,
    Regressed,
    Noisy,
    CheckFailed,
    Deadline,
    NotApplicable,
}

impl Verdict {
    /// The spelling on the `check-cost: verdict=` line.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Drift => "drift",
            Verdict::DriftExceeded => "drift-exceeded",
            Verdict::Regressed => "regressed",
            Verdict::Noisy => "noisy",
            Verdict::CheckFailed => "check-failed",
            Verdict::Deadline => "deadline",
            Verdict::NotApplicable => "not-applicable",
        }
    }

    /// D4's exit column: 0 for the three verdicts nothing has to answer.
    #[must_use]
    pub fn exit_code(self) -> i32 {
        match self {
            Verdict::Pass | Verdict::Drift | Verdict::NotApplicable => 0,
            _ => 1,
        }
    }
}
