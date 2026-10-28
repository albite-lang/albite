//! ADR 7.10.26u AC 1: the bootstrap discharges through the core's one engine,
//! and `discharge.rs` maps each failing index back to its own obligation's
//! error, attaching the engine's note only when there is one.

use std::collections::{BTreeMap, HashMap};

use albite_core::terms::termination::AdmissionState;
use albite_core::{Term, Type};

use super::*;
use crate::span::Span;

/// `left == right`, deferred from a `refl` at `start..start + 4`.
fn obligation_at(start: u32, left: Term, right: Term) -> ConversionObligation {
    ConversionObligation {
        error: ElabError::invalid_refl(Span::new(start, start + 4), left.clone(), right.clone()),
        left,
        right,
        ty: Type::Nat,
    }
}

/// Discharge `obligations` with `two ↦ 2` admitted as `state`.
fn run(obligations: &[ConversionObligation], state: AdmissionState) -> Vec<ElabError> {
    let candidates: HashMap<String, Term> = [("two".to_string(), Term::nat(2))].into();
    let admission: BTreeMap<String, AdmissionState> = [("two".to_string(), state)].into();
    let env = delta_globals(&candidates, &admission, obligations, CONVERSION_STEP_BUDGET);
    discharge(obligations, &env, CONVERSION_STEP_BUDGET)
}

/// 7.10.26u AC 1: of two obligations, only the false one fails, and its error
/// is the one built at ITS span, carrying the note the core engine wrote.
#[test]
fn a_failure_is_reported_at_its_own_obligation_with_the_engines_note() {
    let two = || Term::Global("two".into());
    let obligations = [
        obligation_at(0, two(), Term::nat(2)),
        obligation_at(10, two(), Term::nat(3)),
    ];
    let errors = run(&obligations, AdmissionState::Total);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].span,
        Span::new(10, 14),
        "the second obligation's error"
    );
    let notes: Vec<&str> = errors[0].notes.iter().map(|n| n.message.as_str()).collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].starts_with("after unfolding total definitions"),
        "{notes:?}"
    );
}

/// 7.10.26u AC 1 twin: when the engine gives no note — `two` is not admitted,
/// so nothing unfolded — the error is the deferred one, unadorned.
#[test]
fn a_failure_without_a_note_keeps_its_error_unadorned() {
    let obligations = [obligation_at(0, Term::Global("two".into()), Term::nat(2))];
    let errors = run(&obligations, AdmissionState::Partial);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].span, Span::new(0, 4));
    assert!(errors[0].notes.is_empty(), "{:?}", errors[0].notes);
}
