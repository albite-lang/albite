//! Weak-head reduction for open terms (ADR 29.9.26c, B9 Phase 2a).
//!
//! Phase 1's discharge evaluates call-by-value, so a `Var` anywhere in an
//! argument sticks the whole application before the callee's body is entered.
//! This reducer is demand-driven instead: it reduces only at the head, passes
//! arguments unevaluated (call-by-name), and returns anything it has no rule
//! for as a **neutral** term, to be compared structurally by [`convertible`].
//! A missing rule is incomplete, never unsound: every rule here is one the CBV
//! evaluator performs on closed terms, or D4's `+` rule, which agrees with the
//! evaluator's arithmetic on every closed instance.
//!
//! Pure over injected data — terms, a global map, a budget — and naming no
//! elaborator type, so moving it below the elaborator is a file move (D9).
//! ADR 30.9.26g made that move: the bootstrap's δ discharge and both
//! compilers' δ-free fast path ([`convertible_without_delta`]) call this one
//! engine, so the two cannot drift.
//!
//! **Invariant: conversion never calls the type checker** (ADR 3.10.26o D3).
//! The kernel's dependent case split asserts surjective pairing and unit η,
//! so a reduct of a proof term can fail to re-type; it is sound only because
//! nothing re-types one. Nothing here imports `crate::typecheck`; keep it so.

mod compare;
pub mod delta;
// Tests: tests/ — the test files live there to keep this folder under its
// entry cap (ADR 7.10.26u D3); `#[path]` keeps each a child of this module,
// so their `super::` paths are unchanged.
#[cfg(test)]
#[path = "tests/head_tests.rs"]
mod head_tests;
#[cfg(test)]
#[path = "tests/mul_tests.rs"]
mod mul_tests;
mod rename;
mod rules;
#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "tests/without_delta_tests.rs"]
mod without_delta_tests;

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::eval::EvalStopped;
use crate::Term;

pub use compare::{convertible, convertible_in, Convertible};
pub use rename::{rename_apart, Renaming};

/// Steps each side of a discharged obligation may take (D6 of ADR 28.9.26d),
/// and the budget each reached nullary global is pre-evaluated under. Steps,
/// not wall-clock, so a verdict is the same on every machine. Shared by the
/// bootstrap's discharge and by [`convertible_without_delta`] (ADR 30.9.26g).
pub const CONVERSION_STEP_BUDGET: usize = 1_000_000;

/// The elaboration-time δ-free conversion check (ADR 30.9.26g D2): both
/// compilers' fast path. No global is unfolded, so no admission is needed.
///
/// Both sides are renamed apart first (29.9.26c D2); `eval` `==` accepts;
/// otherwise the WHNF comparison runs over an **empty** global map, gaining
/// ι and the `+`/`*` rules but no δ, and only when neither side performs an
/// effect call-by-name could duplicate or drop (D5).
#[must_use]
pub fn convertible_without_delta(left: &Term, right: &Term) -> bool {
    let (left, right) = rename_apart(left, right);
    if crate::eval::eval(&left) == crate::eval::eval(&right) {
        return true;
    }
    unfoldable_by_name(&left)
        && unfoldable_by_name(&right)
        && convertible(&left, &right, &HashMap::new(), CONVERSION_STEP_BUDGET) == Convertible::Yes
}

/// What WHNF may unfold (ADR 2.10.26l D3, D4): δ's bodies, and the certified
/// decreasing position of each self-recursive one, which guards its
/// unrolling. A plain name → body map records no position, so it unfolds
/// exactly as before; only the kernel's admitted bodies carry one.
pub trait Unfoldings {
    /// The body `name` unfolds to, if δ may unfold it.
    fn body(&self, name: &str) -> Option<&Term>;

    /// The decreasing term-parameter position of `name`'s fixpoint body, if
    /// it is one the kernel admitted as self-recursive.
    fn decreasing(&self, _name: &str) -> Option<usize> {
        None
    }
}

impl<S: BuildHasher> Unfoldings for HashMap<String, Term, S> {
    fn body(&self, name: &str) -> Option<&Term> {
        self.get(name)
    }
}

/// D6's one counter per obligation, shared by both sides and every
/// sub-comparison.
#[derive(Debug)]
pub(crate) struct Fuel {
    remaining: usize,
    limit: usize,
}

impl Fuel {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            limit,
        }
    }

    /// Spend one step, or stop with Phase 1's `StepLimit` so its note text is
    /// reused verbatim.
    pub(crate) fn tick(&mut self) -> Result<(), EvalStopped> {
        if self.remaining == 0 {
            return Err(EvalStopped::StepLimit { limit: self.limit });
        }
        self.remaining -= 1;
        Ok(())
    }
}

/// Whether call-by-name may run `term` (D3, D5): no early `return`, no
/// reference cell and no extern call anywhere in it. By-name `let` would
/// otherwise duplicate or drop an effect CBV performs exactly once.
#[must_use]
pub fn unfoldable_by_name(term: &Term) -> bool {
    if matches!(
        term,
        Term::Return(_)
            | Term::RefNew(_)
            | Term::RefGet(_)
            | Term::RefSet(..)
            | Term::ExternCall(..)
    ) {
        return false;
    }
    let mut unfoldable = true;
    term.for_each_subterm(|child| unfoldable = unfoldable && unfoldable_by_name(child));
    unfoldable
}

/// The globals WHNF may unfold: those whose body [`unfoldable_by_name`]
/// accepts. Computed once per discharge, not per lookup (§2.2).
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: the bootstrap's discharge and census pass std `HashMap`s; generalising buys no caller anything
pub fn unfoldable_globals(globals: &HashMap<String, Term>) -> HashMap<String, Term> {
    globals
        .iter()
        .filter(|(_, body)| unfoldable_by_name(body))
        .map(|(name, body)| (name.clone(), body.clone()))
        .collect()
}

/// Reduce `term` to weak-head normal form under `globals` (D3, D4).
/// Neutral terms come back unchanged; `Err` only on budget exhaustion.
pub(crate) fn whnf<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> Result<Term, EvalStopped> {
    let mut current = term.clone();
    loop {
        match step(&current, globals, fuel)? {
            Step::Reduced(next) => {
                fuel.tick()?;
                current = next;
            }
            Step::Neutral(head) => return Ok(head),
        }
    }
}

/// One head step: a rule fired, or the term is in WHNF (a constructor, or a
/// neutral whose principal argument has been reduced as far as it goes).
enum Step {
    Reduced(Term),
    Neutral(Term),
}

/// Dispatch `term` to its rule (`rules`). Wrappers are stripped and `let` is
/// substituted by name; every form with no rule — a variable, `Fix`, an
/// extern call, a constructor — is already in WHNF. An application whose head
/// is an admitted fixpoint goes to the guarded `fix` rule first (ADR
/// 2.10.26l D4), so its head is never δ-unfolded unguarded.
fn step<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> Result<Step, EvalStopped> {
    match term {
        // `Region` is erased like a span (ADR 9.10.26o D3, G6): `region { b }`
        // and `b` are convertible
        Term::Spanned(inner, _)
        | Term::Annot(inner, _)
        | Term::Region(_, inner)
        | Term::CallFrame(inner) => Ok(Step::Reduced((**inner).clone())),
        Term::Let(x, _, def, body) => Ok(Step::Reduced(body.substitute(x, def))),
        Term::Global(_) => rules::delta(term, globals),
        Term::App(..) | Term::TyApp(..) => {
            if let Some(step) = rules::unroll_fixpoint(term, globals, fuel)? {
                return Ok(step);
            }
            if matches!(term, Term::App(..)) {
                rules::beta(term, globals, fuel)
            } else {
                rules::type_beta(term, globals, fuel)
            }
        }
        Term::Unfold(..) => rules::unfold(term, globals, fuel),
        Term::Fst(_) | Term::Snd(_) => rules::projection(term, globals, fuel),
        Term::Case(..) => rules::case(term, globals, fuel),
        Term::If(..) => rules::conditional(term, globals, fuel),
        Term::AdtMatch(..) => rules::adt_match(term, globals, fuel),
        Term::NatRec(..) | Term::NatInd(..) => rules::recursor(term, globals, fuel),
        Term::Subst(..) => rules::subst(term, globals, fuel),
        Term::NatAdd(..) => rules::add(term, globals, fuel),
        Term::NatMul(..) => rules::mul(term, globals, fuel),
        _ if rules::is_closed_primitive_form(term) => rules::closed_primitive(term, globals, fuel),
        _ => Ok(Step::Neutral(term.clone())),
    }
}
