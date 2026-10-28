//! The bootstrap's side of the δ discharge (ADR 7.10.26u D3).
//!
//! The engine is `albite_core::conversion::delta`, shared with the
//! self-host; it sees an obligation as its two sides and reports a failure as
//! an index and a note. This file maps that back: each failing index to its
//! [`ConversionObligation`], whose `ElabError` was built and file-stamped at
//! deferral time (28.9.26d D9), with the note attached.

use std::collections::{BTreeMap, HashMap};

use albite_core::conversion::delta::{self, DeltaEnv, Sides};
use albite_core::terms::termination::AdmissionState;
use albite_core::Term;

use super::ConversionObligation;
use crate::elaborate::ElabError;

/// The obligations as the engine sees them: their two sides.
fn sides(obligations: &[ConversionObligation]) -> Vec<Sides<'_>> {
    obligations.iter().map(|o| (&o.left, &o.right)).collect()
}

/// Build the discharge environment over these obligations' mention closure.
pub(crate) fn delta_globals(
    candidates: &HashMap<String, Term>,
    admission: &BTreeMap<String, AdmissionState>,
    obligations: &[ConversionObligation],
    budget: usize,
) -> DeltaEnv {
    delta::delta_globals(candidates, admission, &sides(obligations), budget)
}

/// Discharge each obligation, returning the errors of those that failed, each
/// carrying the engine's note when it has one.
pub(crate) fn discharge(
    obligations: &[ConversionObligation],
    env: &DeltaEnv,
    budget: usize,
) -> Vec<ElabError> {
    delta::discharge(&sides(obligations), env, budget)
        .into_iter()
        .map(|failure| {
            let error = obligations[failure.index].error.clone();
            match failure.note {
                Some(note) => error.with_note(note),
                None => error,
            }
        })
        .collect()
}
