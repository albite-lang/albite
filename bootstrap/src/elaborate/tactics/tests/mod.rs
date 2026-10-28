//! Tactic-engine tests (ADR 2.10.26c AC 2, AC 3, AC 5).
//!
//! The proof state and the matcher are pure, so their tests build Core by
//! hand; the steps run through `elab_ok`, the single-file entry point.

mod disjunction_tests;
mod exists_tests;
mod induction_tests;
mod matching_tests;
mod state_tests;
mod steps_tests;

use albite_core::{Term, Type};

use crate::elaborate::error::{ElabErrorKind, TacticErrorKind};
use crate::elaborate::tests::elab_err;

/// The first tactic error `source` raises. Shared with `rewrite/tests/`
/// (ADR 2.10.26e).
pub(super) fn tactic_error(source: &str) -> TacticErrorKind {
    let errors = elab_err(source);
    errors
        .iter()
        .find_map(|e| match &e.kind {
            ElabErrorKind::Tactic(kind) => Some(kind.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no tactic error in {errors:?}"))
}

/// `Eq Nat left right`.
pub(super) fn eq_nat(left: Term, right: Term) -> Type {
    Type::Eq(Box::new(Type::Nat), Box::new(left), Box::new(right))
}

/// `0 + term`.
pub(super) fn zero_plus(term: Term) -> Term {
    Term::NatAdd(Box::new(Term::Zero), Box::new(term))
}
