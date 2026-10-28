//! The gate's foreground phases and the deadline each must fit (D7).
//!
//! A batch child may not background anything and calls Bash at a 600 s cap,
//! so the gate is five phases, each resumable from the phase file.

use crate::verdict::Next;
use crate::Samples;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Below this, a phase reused earlier work and its duration is no estimate.
pub const REUSED_PHASE_S: u64 = 10;

/// The container-local phase file: what `prepare` decided, what `measure`
/// measured and what each phase last cost, keyed by base sha and tree stamp.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PhaseState {
    pub base: String,
    pub tree: String,
    #[serde(default)]
    pub measured: bool,
    #[serde(default)]
    pub stored_wall_s: Vec<f64>,
    #[serde(default)]
    pub stored_max_rss_kb: Vec<u64>,
    #[serde(default)]
    pub stored_exit_codes: Vec<i32>,
    /// The base sha whose `./albite1` the kept worktree holds.
    #[serde(default)]
    pub ab_built_base: String,
    #[serde(default)]
    pub durations_s: BTreeMap<String, u64>,
}

impl PhaseState {
    /// A missing or unreadable file is a fresh state: nothing is resumed.
    #[must_use]
    pub fn parse(text: Option<&str>) -> PhaseState {
        text.and_then(|t| toml::from_str(t).ok())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn render(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// D7: a phase after `prepare` refuses when its inputs moved since.
    #[must_use]
    pub fn inputs_moved(&self, base: &str, tree: &str) -> bool {
        self.base != base || self.tree != tree
    }

    /// The last measured duration of `phase`, else P0.3's number.
    #[must_use]
    pub fn estimate_s(&self, phase: Phase) -> u64 {
        self.durations_s
            .get(phase.name())
            .copied()
            .unwrap_or_else(|| phase.default_estimate_s())
    }

    /// Keep a phase's duration as its next estimate — unless the phase reused
    /// earlier work (a warm `base-compile` takes seconds), which would leave a
    /// near-zero estimate in front of the next cold build.
    pub fn record_duration(&mut self, phase: Phase, secs: u64) {
        if secs >= REUSED_PHASE_S {
            self.durations_s.insert(phase.name().into(), secs);
        }
    }

    /// The candidate samples `measure` recorded, for the A/B that may follow.
    #[must_use]
    pub fn stored_samples(&self) -> Option<Samples> {
        self.measured.then(|| Samples {
            wall_s: self.stored_wall_s.clone(),
            max_rss_kb: self.stored_max_rss_kb.clone(),
            exit_codes: self.stored_exit_codes.clone(),
        })
    }

    pub fn record_stored(&mut self, samples: &Samples) {
        self.measured = true;
        self.stored_wall_s.clone_from(&samples.wall_s);
        self.stored_max_rss_kb.clone_from(&samples.max_rss_kb);
        self.stored_exit_codes.clone_from(&samples.exit_codes);
    }

    /// `prepare` starts a new run: the worktree's build and the durations
    /// survive, because they describe the kept worktree, not this tree.
    #[must_use]
    pub fn restart(&self, base: &str, tree: &str) -> PhaseState {
        PhaseState {
            base: base.to_string(),
            tree: tree.to_string(),
            ab_built_base: self.ab_built_base.clone(),
            durations_s: self.durations_s.clone(),
            ..PhaseState::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Prepare,
    Measure,
    BaseTools,
    BaseCompile,
    Ab,
}

impl Phase {
    pub const ALL: [Phase; 5] = [
        Phase::Prepare,
        Phase::Measure,
        Phase::BaseTools,
        Phase::BaseCompile,
        Phase::Ab,
    ];

    /// The `--phase` spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Phase::Prepare => "prepare",
            Phase::Measure => "measure",
            Phase::BaseTools => "base-tools",
            Phase::BaseCompile => "base-compile",
            Phase::Ab => "ab",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Phase> {
        Phase::ALL.into_iter().find(|p| p.name() == name)
    }

    /// The estimate used before a phase has a measured duration of its own
    /// (P0.3's numbers, rounded up; ADR 15.10.26f § Outcome).
    #[must_use]
    pub fn default_estimate_s(self) -> u64 {
        match self {
            Phase::Prepare | Phase::BaseTools | Phase::BaseCompile => 420,
            Phase::Measure => 200,
            Phase::Ab => 360,
        }
    }
}

/// The phase after `current`, given where the plan now stands; `None` when the
/// gate has its verdict. The A/B path is always base-tools → base-compile → ab.
#[must_use]
pub fn next_phase(current: Phase, plan: Next) -> Option<Phase> {
    match (current, plan) {
        (_, Next::Decided(_)) | (Phase::Ab, _) => None,
        (Phase::Prepare, Next::MeasureStored) => Some(Phase::Measure),
        (Phase::Prepare | Phase::Measure, _) => Some(Phase::BaseTools),
        (Phase::BaseTools, _) => Some(Phase::BaseCompile),
        (Phase::BaseCompile, _) => Some(Phase::Ab),
    }
}

/// D7: a phase starts only if its estimate fits the time left before
/// `ACTION_BATCH_DEADLINE`. No deadline is no ceiling.
#[must_use]
pub fn deadline_fits(estimate_s: u64, deadline_epoch: Option<u64>, now_epoch: u64) -> bool {
    match deadline_epoch {
        None => true,
        Some(deadline) => deadline.saturating_sub(now_epoch) >= estimate_s,
    }
}

/// The command a phase prints for the next one (`check-cost: next="…"`).
#[must_use]
pub fn next_command(phase: Phase, cost_args: &str) -> String {
    let extra = if cost_args.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", cost_args.trim())
    };
    format!(
        "make docker-vm-run TARGET=devcontainer-check-cost-gate COST_ARGS='--phase {}{extra}'",
        phase.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Verdict;

    /// 15.10.26f AC2: `next_phase` names the next phase for each result.
    #[test]
    fn next_phase_follows_the_plan() {
        let ab = Next::RunAb;
        let done = Next::Decided(Verdict::Pass);
        assert_eq!(
            next_phase(Phase::Prepare, Next::MeasureStored),
            Some(Phase::Measure)
        );
        assert_eq!(next_phase(Phase::Prepare, ab), Some(Phase::BaseTools));
        assert_eq!(next_phase(Phase::Measure, ab), Some(Phase::BaseTools));
        assert_eq!(next_phase(Phase::BaseTools, ab), Some(Phase::BaseCompile));
        assert_eq!(next_phase(Phase::BaseCompile, ab), Some(Phase::Ab));
        assert_eq!(next_phase(Phase::Ab, ab), None);
        for phase in Phase::ALL {
            assert_eq!(next_phase(phase, done), None, "{}", phase.name());
        }
    }

    /// 15.10.26f AC2: `deadline_fits` refuses an estimate the time left does
    /// not cover, and fits one exactly covered.
    #[test]
    fn deadline_fits_refuses_an_estimate_past_the_time_left() {
        assert!(deadline_fits(300, None, 1_000));
        assert!(deadline_fits(300, Some(1_300), 1_000));
        assert!(!deadline_fits(301, Some(1_300), 1_000));
        assert!(
            !deadline_fits(1, Some(900), 1_000),
            "a passed deadline fits nothing"
        );
    }

    #[test]
    fn phase_names_round_trip() {
        for phase in Phase::ALL {
            assert_eq!(Phase::parse(phase.name()), Some(phase));
        }
        // No default estimate is below the slowest run of that phase that
        // P0.3 measured, so a first run never starts on a ceiling it overruns.
        for (phase, measured_s) in [
            (Phase::Prepare, 263),
            (Phase::Measure, 200),
            (Phase::BaseTools, 89),
            (Phase::BaseCompile, 155),
            (Phase::Ab, 315),
        ] {
            assert!(phase.default_estimate_s() >= measured_s, "{phase:?}");
        }
        assert_eq!(Phase::parse("bogus"), None);
    }

    #[test]
    fn phase_state_round_trips_restarts_and_estimates() {
        let mut state = PhaseState::parse(None).restart("b1", "t1");
        assert!(!state.inputs_moved("b1", "t1"));
        assert!(state.inputs_moved("b2", "t1"));
        assert!(state.inputs_moved("b1", "t2"));
        assert_eq!(state.stored_samples(), None);
        assert_eq!(state.estimate_s(Phase::Ab), Phase::Ab.default_estimate_s());

        let samples = Samples {
            wall_s: vec![1.0, 2.0],
            max_rss_kb: vec![3, 4],
            exit_codes: vec![0, 1],
        };
        state.record_stored(&samples);
        state.ab_built_base = "b0".into();
        state.durations_s.insert("ab".into(), 77);
        let parsed = PhaseState::parse(Some(&state.render()));
        assert_eq!(parsed, state);
        assert_eq!(parsed.stored_samples(), Some(samples));
        assert_eq!(parsed.estimate_s(Phase::Ab), 77);

        let restarted = parsed.restart("b9", "t9");
        assert_eq!(restarted.base, "b9");
        assert_eq!(restarted.tree, "t9");
        assert!(!restarted.measured);
        assert_eq!(
            restarted.ab_built_base, "b0",
            "the worktree's build survives"
        );
        assert_eq!(restarted.estimate_s(Phase::Ab), 77, "durations survive");
        assert_eq!(
            PhaseState::parse(Some("not = [toml")),
            PhaseState::default()
        );
    }

    #[test]
    fn a_reused_phase_keeps_the_last_real_estimate() {
        let mut state = PhaseState::default();
        state.record_duration(Phase::BaseCompile, 143);
        assert_eq!(state.estimate_s(Phase::BaseCompile), 143);
        state.record_duration(Phase::BaseCompile, REUSED_PHASE_S - 1);
        assert_eq!(
            state.estimate_s(Phase::BaseCompile),
            143,
            "a reuse is not an estimate"
        );
        state.record_duration(Phase::BaseCompile, REUSED_PHASE_S);
        assert_eq!(state.estimate_s(Phase::BaseCompile), REUSED_PHASE_S);
    }

    #[test]
    fn next_command_carries_the_extra_args() {
        assert_eq!(
            next_command(Phase::Measure, ""),
            "make docker-vm-run TARGET=devcontainer-check-cost-gate COST_ARGS='--phase measure'"
        );
        assert_eq!(
            next_command(Phase::Ab, " --force-ab "),
            "make docker-vm-run TARGET=devcontainer-check-cost-gate COST_ARGS='--phase ab --force-ab'"
        );
    }
}
