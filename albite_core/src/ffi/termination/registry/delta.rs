//! The δ discharge's share of the termination streams (ADR 7.10.26u D2).
//!
//! The discharge needs the bodies of admitted definitions such as a nullary
//! `two`, which the Retain stream never keeps: it holds only recursive
//! components, because holding every body cost +587 MiB (19.8.26d). So this is
//! **route (a)**: the obligations' global mentions are known before `plan`
//! (they were recorded during body elaboration), the Reduce stream already
//! holds every definition's mention set, and `plan` closes the one over the
//! other. The Retain stream then keeps exactly that closure's bodies besides
//! the planned ones. With no obligation — `src/compiler/main.tg` has none —
//! the closure is empty and nothing more is kept.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::conversion::delta::{delta_globals, discharge, global_names, DischargeFailure, Sides};
use crate::conversion::CONVERSION_STEP_BUDGET;
use crate::terms::termination::AdmissionState;
use crate::terms::Term;

use super::TerminationRegistry;

/// What the δ discharge asks the streams to keep.
#[derive(Debug, Default)]
pub(crate) struct DeltaRetention {
    /// The globals the recorded obligations mention, seeded before `plan`.
    seeds: BTreeSet<String>,
    /// The seeds' transitive mention closure, fixed by `plan`.
    closure: BTreeSet<String>,
    /// The closure's bodies, as the Retain stream offered them — before the
    /// descent-only binder rewrite, which evaluation does not want.
    bodies: HashMap<String, Term>,
    /// Every definition's admission state, from the last `check`.
    admission: BTreeMap<String, AdmissionState>,
    /// Names the Reduce stream offered more than once (ADR 13.10.26m D5).
    repeated: BTreeSet<String>,
}

/// Every name reachable from `seeds` through `mentions`, the seeds included
/// when they are definitions. A seed with no mention set — a constructor, a
/// name nothing declared — is not a definition and is left out.
pub(crate) fn mention_closure(
    mentions: &BTreeMap<String, BTreeSet<String>>,
    seeds: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut closure = BTreeSet::new();
    let mut queue: Vec<&String> = seeds.iter().collect();
    while let Some(name) = queue.pop() {
        let Some(reached) = mentions.get(name) else {
            continue;
        };
        if closure.insert(name.clone()) {
            queue.extend(reached);
        }
    }
    closure
}

impl TerminationRegistry {
    /// Name the globals the recorded obligations mention. Called before
    /// `plan`, which closes them over the Reduce stream's mention sets.
    pub(crate) fn seed_delta(&mut self, seeds: BTreeSet<String>) {
        self.delta.seeds = seeds;
    }

    /// Fix the closure the Retain stream keeps for the discharge, and return
    /// how many of it the descent plan does not already hold.
    pub(super) fn plan_delta(&mut self) -> usize {
        self.delta.closure = mention_closure(&self.mentions, &self.delta.seeds);
        self.delta.closure.difference(&self.planned).count()
    }

    /// Record that the Reduce stream offered `name` a second time.
    pub(super) fn note_repeated_definition(&mut self, name: &str) {
        self.delta.repeated.insert(name.to_string());
    }

    /// Whether the Reduce stream offered `name` more than once.
    pub(super) fn is_repeated(&self, name: &str) -> bool {
        self.delta.repeated.contains(name)
    }

    /// Keep `term` if the discharge's closure names it and the name is not
    /// repeated (ADR 13.10.26m D5).
    ///
    /// The terms carry a bare `Global(name)`, so two modules' same-named
    /// definitions are one key here, and the last writer's body would answer
    /// for both: `x::two() == 3` would hold because `y::two` is 3. Keeping no
    /// body makes the name opaque, and an obligation that needs it fails.
    pub(super) fn retain_for_delta(&mut self, name: &str, term: &Term) {
        if self.delta.closure.contains(name) && !self.is_repeated(name) {
            self.delta.bodies.insert(name.to_string(), term.clone());
        }
    }

    /// The names the Reduce stream offered more than once.
    #[cfg(test)]
    pub(crate) fn delta_repeated_for_test(&self) -> BTreeSet<String> {
        self.delta.repeated.clone()
    }

    /// Record the last `check`'s admission states.
    pub(super) fn record_admission(&mut self, admission: BTreeMap<String, AdmissionState>) {
        self.delta.admission = admission;
    }

    /// The bodies the Retain stream kept for the discharge.
    #[cfg(test)]
    pub(crate) fn delta_bodies_for_test(&self) -> BTreeSet<String> {
        self.delta.bodies.keys().cloned().collect()
    }

    /// Discharge `obligations` against the retained bodies and the last
    /// `check`'s admission (28.9.26d D1). A registry that never checked has
    /// admitted nothing, so every obligation that needs δ fails — a lost
    /// stream refuses a proof, never accepts one.
    pub(crate) fn discharge_deltas(&self, obligations: &[Sides<'_>]) -> Vec<DischargeFailure> {
        let env = delta_globals(
            &self.delta.bodies,
            &self.delta.admission,
            obligations,
            CONVERSION_STEP_BUDGET,
        );
        let mut failures = discharge(obligations, &env, CONVERSION_STEP_BUDGET);
        for failure in &mut failures {
            if let Some(name) = self.repeated_name_reached(obligations[failure.index]) {
                failure.note = Some(format!(
                    "`{name}` is defined in more than one module; tungsten1 cannot yet unfold it"
                ));
            }
        }
        failures
    }

    /// The first repeated name an obligation's sides reach through the
    /// mention sets — the reason δ could not unfold it, when there is one.
    fn repeated_name_reached(&self, (left, right): Sides<'_>) -> Option<String> {
        let seeds: BTreeSet<String> = global_names(left)
            .into_iter()
            .chain(global_names(right))
            .collect();
        let reached = mention_closure(&self.mentions, &seeds);
        self.delta.repeated.intersection(&reached).next().cloned()
    }
}
