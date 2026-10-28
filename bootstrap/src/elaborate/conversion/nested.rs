//! A closed `Eq` endpoint pair converts wherever it is nested (ADR 7.10.26i).
//!
//! Check mode's second fall-through, after [`super::eq_fallback`] has said
//! the two types are not both a bare `Eq`. It is the strict walk the
//! kernel's `types_convert` uses (ADR 1.10.26c), with an `Eq` comparator that
//! accepts an endpoint pair equal modulo spans, **or** one whose endpoints
//! are both closed and global-free and convert δ-free. Closed means nothing
//! is deferred and no binder can be captured; an open pair (`n + 0` against
//! `n`) stays structural until full Phase 2b.
//!
//! Narrowed to a dependent pair or a predicate application at the top (D4):
//! `albite1` refuses both shapes at parse, so no acceptance here can be one
//! the self-host does not share. It refuses a product of closed `Eq`s (P0.3),
//! so that shape stays refused on both.

use albite_core::conversion::convertible_without_delta;
use albite_core::types::{
    eq_witnesses_equal_modulo_spans, types_equal_strict_with, EqWitnessPairs,
};
use albite_core::{Term, Type};

use super::mentions_global;
use crate::elaborate::Elaborator;

impl Elaborator<'_> {
    /// Whether `inferred` meets `expected` once every nested `Eq`'s closed
    /// endpoints are compared by computation (D1, D2). Only a yes differs
    /// from the refusal the caller already has.
    pub(crate) fn closed_endpoints_convert(&self, inferred: &Type, expected: &Type) -> bool {
        let inferred = self.normalize_for_comparison(inferred);
        let expected = self.normalize_for_comparison(expected);
        nested_types_convert(&inferred, &expected)
    }
}

/// D2 over two already-normalised types, narrowed per D4. One side decides
/// the narrowing: the strict walk matches outermost constructors, so it
/// refuses a `Sigma` against anything but a `Sigma`.
pub(crate) fn nested_types_convert(inferred: &Type, expected: &Type) -> bool {
    refused_by_tungsten1_at_parse(expected)
        && types_equal_strict_with(inferred, expected, &endpoint_pairs_meet)
}

/// D4's narrowing: the shapes `albite1` cannot parse, so the bootstrap may
/// accept more of them without opening a single-compiler verdict.
fn refused_by_tungsten1_at_parse(ty: &Type) -> bool {
    matches!(ty, Type::Sigma(..) | Type::PredApp(..))
}

/// One `Eq`'s pairs, endpoint by endpoint, so an open left endpoint equal on
/// both sides does not stop a closed right one from converting.
fn endpoint_pairs_meet(
    ((l1, r1), (l2, r2)): EqWitnessPairs<'_>,
    binders: &[(String, String)],
) -> bool {
    endpoints_meet(l1, l2, binders) && endpoints_meet(r1, r2, binders)
}

/// Equal modulo spans under the binders in scope, or — with no type binder
/// open — both closed and convertible without δ.
fn endpoints_meet(found: &Term, wanted: &Term, binders: &[(String, String)]) -> bool {
    eq_witnesses_equal_modulo_spans(((found, found), (wanted, wanted)), binders)
        || (binders.is_empty()
            && is_closed(found)
            && is_closed(wanted)
            && convertible_without_delta(found, wanted))
}

/// No free variable and no global: a term whose normal form the δ-free
/// engine reaches with nothing deferred. No global also means no recursion,
/// which is why `convertible_without_delta`'s unbounded first `eval` — the
/// reason the kernel avoids it (`albite_core/src/typecheck/convert.rs`) —
/// terminates here.
fn is_closed(term: &Term) -> bool {
    term.free_vars().is_empty() && !mentions_global(term)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(n: Term, m: Term) -> Type {
        let sum = Term::nat_add(n, Term::var("k"));
        Type::sigma("k", Type::Nat, Type::eq(Type::Nat, sum, m))
    }

    fn two_plus_three() -> Term {
        Term::nat_add(Term::nat(2), Term::nat(3))
    }

    // 7.10.26i AC1: a closed sum under a dependent pair meets its numeral,
    // although the open left endpoint `2 + k` is only equal modulo spans.
    #[test]
    fn a_closed_sum_under_a_dependent_pair_converts() {
        let found = le(Term::nat(2), two_plus_three());
        let wanted = le(Term::nat(2), Term::nat(5));
        assert!(!albite_core::types::types_equal_strict(&found, &wanted));
        assert!(nested_types_convert(&found, &wanted));
        assert!(nested_types_convert(&wanted, &found));
    }

    // 7.10.26i AC1: unequal numerals do not convert.
    #[test]
    fn unequal_numerals_under_a_dependent_pair_are_refused() {
        let found = le(Term::nat(2), two_plus_three());
        assert!(!nested_types_convert(
            &found,
            &le(Term::nat(2), Term::nat(6))
        ));
    }

    // 7.10.26i AC3: an open endpoint is not converted, even one that would.
    #[test]
    fn an_open_endpoint_stays_structural() {
        let open = |t: Term| le(Term::nat(2), t);
        let n_plus_zero = Term::nat_add(Term::var("n"), Term::Zero);
        assert!(!nested_types_convert(
            &open(n_plus_zero),
            &open(Term::var("n"))
        ));
    }

    // 7.10.26i AC3: a global is never unfolded here (no δ for nested endpoints).
    #[test]
    fn an_endpoint_mentioning_a_global_stays_structural() {
        let found = le(Term::nat(2), Term::global("five"));
        assert!(!nested_types_convert(
            &found,
            &le(Term::nat(2), Term::nat(5))
        ));
        assert!(!is_closed(&Term::global("five")));
        assert!(!is_closed(&Term::var("n")));
        assert!(is_closed(&two_plus_three()));
    }

    // 7.10.26i D4: narrowed — a product of closed `Eq`s stays refused, because
    // `albite1` refuses it too (P0.3); both sides must be a `Sigma` or a
    // `PredApp`.
    #[test]
    fn a_product_of_closed_eqs_is_narrowed_out() {
        let product = |t: Term| Type::product(Type::eq(Type::Nat, t, Term::nat(5)), Type::Nat);
        assert!(!nested_types_convert(
            &product(two_plus_three()),
            &product(Term::nat(5))
        ));
        let sigma = le(Term::nat(2), Term::nat(5));
        assert!(!nested_types_convert(&sigma, &Type::Nat));
        assert!(!nested_types_convert(&Type::Nat, &sigma));
    }

    // 7.10.26i D4: a predicate application's arguments are compared through
    // the same comparator, so a closed argument converts there too.
    #[test]
    fn a_closed_predicate_argument_converts() {
        let app = |t: Term| Type::PredApp("Le".into(), vec![Term::nat(2), t]);
        assert!(nested_types_convert(
            &app(two_plus_three()),
            &app(Term::nat(5))
        ));
        assert!(!nested_types_convert(
            &app(two_plus_three()),
            &app(Term::nat(6))
        ));
    }

    // 7.10.26i D2: BOTH endpoints must be closed — an open endpoint that does
    // compute to the closed one (`(λx. 0) n` against `0`) is still refused.
    #[test]
    fn one_open_endpoint_is_not_converted() {
        let open = Term::app(Term::lambda("x", Type::Nat, Term::Zero), Term::var("n"));
        assert!(convertible_without_delta(&open, &Term::Zero));
        assert!(!endpoints_meet(&open, &Term::Zero, &[]));
        assert!(!endpoints_meet(&Term::Zero, &open, &[]));
    }

    // 7.10.26i D2: under an open type binder nothing converts, as in the kernel.
    #[test]
    fn no_conversion_under_an_open_type_binder() {
        let binders = vec![("a".to_string(), "b".to_string())];
        assert!(!endpoints_meet(&two_plus_three(), &Term::nat(5), &binders));
        assert!(endpoints_meet(&two_plus_three(), &Term::nat(5), &[]));
        assert!(endpoints_meet(&Term::var("x"), &Term::var("x"), &binders));
    }
}
