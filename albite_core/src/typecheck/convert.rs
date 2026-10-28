//! The proof judgment's type conversion (ADR 1.10.26c D2, D3).
//!
//! Type conversion is the strict walk with one change: an `Eq`'s witness
//! pairs also count as equal when the shared WHNF engine finds them
//! convertible. Conversion is a relation on terms, so it enters a type only
//! through an `Eq`; every other arm of the walk stays strict.
//!
//! The engine unfolds only the bodies the context carries (ADR 2.10.26l D3):
//! bodies the kernel admitted itself — `Total`, effect-free and typed by
//! `check_proof` — and a self-recursive one only on a constructor (D4). Every
//! other global is neutral and compared by name; a context with no body is
//! the δ-free conversion of 1.10.26c exactly. It is budgeted, and a budget
//! that runs out is a refusal, never "equal".

use crate::context::{Context, ContextUnfoldings};
use crate::conversion::{
    convertible_in, rename_apart, unfoldable_by_name, Convertible, CONVERSION_STEP_BUDGET,
};
use crate::terms::Term;
use crate::types::{
    eq_witnesses_equal_modulo_spans, types_equal_strict_with, EqWitnessPairs, Type,
};

/// Whether `a` and `b` are equal up to conversion of the witnesses in their
/// `Eq`s, unfolding the bodies `ctx` admitted.
#[must_use]
pub(super) fn types_convert(ctx: &Context, a: &Type, b: &Type) -> bool {
    let unfoldings = ctx.unfoldings();
    types_equal_strict_with(a, b, &|pairs, binders| {
        eq_witnesses_equal_modulo_spans(pairs, binders)
            || (binders.is_empty() && eq_witnesses_convert(pairs, &unfoldings))
    })
}

/// D3: one `Eq` pair, `Eq τ l₁ r₁` against `Eq τ l₂ r₂`.
///
/// Only where no type binder is open (D2): `convertible` compares the types
/// embedded in terms with `==`, blind to a `∀`/`μ` binder pairing, so under
/// one it could answer a false `Yes`.
///
/// 1. The effect guard: call-by-name would duplicate or drop an effect that
///    call-by-value performs once (29.9.26c D5).
/// 2. One joint renaming, so both pairs share it.
/// 3. One comparison of the two pairs, so both spend one budget: a `Pair` is a
///    constructor head, so the engine compares its components pairwise.
///
/// Not `convertible_without_delta`: its first step is an unbounded `eval`, and
/// a kernel must answer on every input.
fn eq_witnesses_convert(
    ((l1, r1), (l2, r2)): EqWitnessPairs<'_>,
    unfoldings: &ContextUnfoldings<'_>,
) -> bool {
    if ![l1, r1, l2, r2].into_iter().all(unfoldable_by_name) {
        return false;
    }
    let left = Term::pair(l1.clone(), r1.clone());
    let right = Term::pair(l2.clone(), r2.clone());
    let (left, right) = rename_apart(&left, &right);
    convertible_in(&left, &right, unfoldings, CONVERSION_STEP_BUDGET) == Convertible::Yes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq_nat(left: Term, right: Term) -> Type {
        Type::eq(Type::Nat, left, right)
    }

    // 1.10.26c AC1: the converting arm fires only on an `Eq`, only where the
    // strict walk refused, and under a `Pi`.
    #[test]
    fn conversion_equates_witnesses_by_computation() {
        let n_plus_zero = || Term::nat_add(Term::var("n"), Term::Zero);
        let refl_side = eq_nat(n_plus_zero(), n_plus_zero());
        let stated = eq_nat(n_plus_zero(), Term::var("n"));
        assert!(!crate::types::types_equal_strict(&refl_side, &stated));
        assert!(types_convert(&Context::new(), &refl_side, &stated));
        assert!(types_convert(
            &Context::new(),
            &Type::pi("n", Type::Nat, refl_side.clone()),
            &Type::pi(
                "m",
                Type::Nat,
                eq_nat(Term::nat_add(Term::var("m"), Term::Zero), Term::var("m"))
            ),
        ));
        assert!(!types_convert(
            &Context::new(),
            &eq_nat(Term::var("n"), Term::var("n")),
            &eq_nat(Term::var("n"), Term::succ(Term::var("n"))),
        ));
        // A base type is compared strictly, not converted.
        assert!(!types_convert(
            &Context::new(),
            &Type::eq(Type::Bool, Term::var("n"), Term::var("n")),
            &eq_nat(Term::var("n"), Term::var("n")),
        ));
    }

    // 1.10.26c AC1: both witness pairs must convert — a left pair that agrees
    // does not carry a right pair that does not.
    #[test]
    fn both_witness_pairs_must_convert() {
        let two = || Term::nat_add(Term::NatLit(1), Term::NatLit(1));
        assert!(types_convert(
            &Context::new(),
            &eq_nat(two(), Term::NatLit(3)),
            &eq_nat(Term::NatLit(2), Term::NatLit(3)),
        ));
        assert!(!types_convert(
            &Context::new(),
            &eq_nat(two(), Term::NatLit(3)),
            &eq_nat(Term::NatLit(2), Term::NatLit(4)),
        ));
        assert!(!types_convert(
            &Context::new(),
            &eq_nat(Term::NatLit(3), two()),
            &eq_nat(Term::NatLit(4), Term::NatLit(2)),
        ));
    }
}
