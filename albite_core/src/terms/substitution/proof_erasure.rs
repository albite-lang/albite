//! Proof erasure before codegen (ADR 3.10.26e D4).
//!
//! A `natind` whose motive is a proposition computes nothing a program can
//! observe: its value is a proof, and codegen already lowers every `Eq` value
//! to the unit `Refl` lowers to. Compiled as written it becomes a `natrec`
//! loop computing that unit in O(n) — and its accumulator is mistyped, which
//! is the LLVM verifier failure 30.9.26d parked. So the compile entry erases
//! it. Only the compile path runs this, after the kernel has checked every
//! proof; the evaluator never sees it.

use crate::conversion::unfoldable_by_name;
use crate::types::Type;

use super::Term;

/// The binder spelled on each `λ` an erased generalised proof keeps. The body
/// is `Unit`, so nothing reads it; `%` keeps it out of any source name.
const ERASED_BINDER: &str = "%erased";

impl Term {
    /// Every effect-free `natind` over a proposition replaced by the unit
    /// value it lowers to — under a `λ` per argument when the motive is
    /// generalised, so an application around it still type-checks.
    #[must_use]
    pub fn erase_proofs(&self) -> Term {
        let mut erased = self.clone();
        erase_proofs_in(&mut erased);
        erased
    }
}

fn erase_proofs_in(term: &mut Term) {
    if let Some(replacement) = erased_proof(term) {
        *term = replacement;
        return;
    }
    for child in term.subterms_mut() {
        erase_proofs_in(child);
    }
}

/// The erased form of `term` when it is a proof-valued `natind` whose cases
/// and scrutinee drop no effect; `None` for anything else, which is kept.
fn erased_proof(term: &Term) -> Option<Term> {
    let Term::NatInd(_, motive, base, step, scrutinee) = term else {
        return None;
    };
    let (Type::Arrow(_, body) | Type::Pi(_, _, body)) = motive else {
        return None;
    };
    let effect_free = [base, step, scrutinee]
        .iter()
        .all(|part| unfoldable_by_name(part));
    (body.is_proposition() && effect_free).then(|| unit_with_arity(body))
}

/// `Unit` under one `λ` per argument of `proposition`'s arrow chain.
fn unit_with_arity(proposition: &Type) -> Term {
    match proposition {
        Type::Arrow(domain, rest) | Type::Pi(_, domain, rest) => {
            Term::lambda(ERASED_BINDER, (**domain).clone(), unit_with_arity(rest))
        }
        _ => Term::Unit,
    }
}
