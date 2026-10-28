//! `rewrite`'s tests (ADR 2.10.26e AC 2–4).
//!
//! This file holds AC 2: the occurrence search and the motive are pure, so
//! they are tested over hand-built Core. `elaboration.rs` runs the tactic
//! through `elab_ok`/`elab_err` (AC 1's elaboration half, AC 3, AC 4, D6).

mod back_end;
mod elaboration;

use std::collections::BTreeMap;

use albite_core::{Term, TermSpan, Type};

use super::abstraction::{abstract_occurrences, first_match, motive_and_new_goal, occurs};
use crate::elaborate::exprs::combinators::SYNTHESISED_MOTIVE_BINDER;

fn var(name: &str) -> Term {
    Term::var(name)
}

fn add(left: Term, right: Term) -> Term {
    Term::NatAdd(Box::new(left), Box::new(right))
}

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

fn bound() -> Term {
    var(SYNTHESISED_MOTIVE_BINDER)
}

/// `(Nat, l, r)` borrowed from `goal`, as `goal_endpoints` returns it.
fn endpoints(goal: &Type) -> (&Type, &Term, &Term) {
    match goal {
        Type::Eq(base, left, right) => (base, left, right),
        other => panic!("not an equation: {other:?}"),
    }
}

// ADR 2.10.26e AC2
#[test]
fn every_occurrence_is_abstracted() {
    let goal = eq_nat(add(var("a"), var("a")), var("b"));
    let (motive, new_goal) =
        motive_and_new_goal(endpoints(&goal), &Type::Nat, &var("a"), &var("b"));
    assert_eq!(
        motive,
        Type::pi(
            SYNTHESISED_MOTIVE_BINDER,
            Type::Nat,
            eq_nat(add(bound(), bound()), var("b"))
        )
    );
    assert_eq!(new_goal, eq_nat(add(var("b"), var("b")), var("b")));
}

// ADR 2.10.26e AC2: a binder's body may mention what it binds (D3).
#[test]
fn an_occurrence_under_a_binder_is_left_alone() {
    let under_lambda = Term::lambda("a", Type::Nat, add(var("a"), var("c")));
    let term = add(var("a"), Term::app(under_lambda.clone(), var("a")));
    assert_eq!(
        abstract_occurrences(&term, &var("a"), &bound()),
        add(bound(), Term::app(under_lambda.clone(), bound()))
    );
    // `c` occurs only under the binder: not an occurrence at all.
    assert!(!occurs(&var("c"), &term));
    assert!(occurs(&var("a"), &term));
    // Every binding form stops the walk, not only `λ`.
    let under_let = Term::let_in("x", Type::Nat, var("c"), var("c"));
    assert!(!occurs(&var("c"), &under_let));
    let under_fix = Term::Fix("f".into(), Type::Nat, Box::new(var("c")));
    assert!(!occurs(&var("c"), &under_fix));
}

// ADR 2.10.26e AC2: the outermost-leftmost sum wins (D3).
#[test]
fn the_first_match_in_preorder_fixes_the_metavariables() {
    let vars = vec!["n".to_string(), "m".to_string()];
    let pattern = add(var("n"), var("m"));
    let inner = add(var("a"), var("b"));
    let left = add(inner.clone(), var("c"));
    let right = add(var("c"), inner.clone());
    let bindings = first_match(&pattern, &vars, &left, &right).expect("a sum matches");
    assert_eq!(
        bindings,
        BTreeMap::from([
            ("n".to_string(), inner.clone()),
            ("m".to_string(), var("c"))
        ])
    );
    // With no match in `l`, the search goes on into `r`.
    let bindings = first_match(&pattern, &vars, &var("z"), &right).expect("r has a sum");
    assert_eq!(bindings["n"], var("c"));
    assert_eq!(first_match(&pattern, &vars, &var("z"), &var("y")), None);
}

// ADR 2.10.26e AC2: after the first match, every copy of the instantiated
// pattern moves — including one in the other endpoint.
#[test]
fn the_instantiated_pattern_is_abstracted_everywhere() {
    let sum = add(var("a"), var("b"));
    let spanned_sum = Term::Spanned(Box::new(sum.clone()), TermSpan::new(4, 9));
    let goal = eq_nat(add(sum.clone(), var("c")), add(var("c"), spanned_sum));
    let (motive, _) = motive_and_new_goal(endpoints(&goal), &Type::Nat, &sum, &var("z"));
    assert_eq!(
        motive,
        Type::pi(
            SYNTHESISED_MOTIVE_BINDER,
            Type::Nat,
            eq_nat(add(bound(), var("c")), add(var("c"), bound()))
        )
    );
}

// ADR 2.10.26e AC2: a span is transparent to the search — the walk looks
// through it, and a spanned occurrence is still found and still a match.
#[test]
fn the_search_looks_through_spans() {
    let spanned = Term::Spanned(Box::new(add(var("a"), var("b"))), TermSpan::new(0, 5));
    assert!(occurs(&var("b"), &spanned));
    assert!(occurs(&add(var("a"), var("b")), &spanned));
    assert!(!occurs(&var("c"), &spanned));
    let vars = vec!["n".to_string()];
    let bindings = first_match(&add(var("a"), var("n")), &vars, &spanned, &var("z"))
        .expect("the sum under the span matches");
    assert_eq!(bindings["n"], var("b"));
}

/// The motive's body, `P`.
fn family(motive: &Type) -> &Type {
    match motive {
        Type::Pi(_, _, body) => body,
        other => panic!("expected a Π motive, got {other:?}"),
    }
}

// ADR 2.10.26e AC2: D4's two rows. `P[#x := from]` gives back the goal, so
// the kernel's result meets it with no conversion; `substitute_term` renames
// only on capture (P0.2), so the comparison is exact, not up to α.
#[test]
fn forward_and_reverse_build_the_tables_terms() {
    let (a, b) = (add(var("n"), var("zero_side")), var("n"));
    let goal = eq_nat(add(a.clone(), var("k")), var("k"));
    // Forward: `a` is abstracted, the new goal has `b`.
    let (motive, new_goal) = motive_and_new_goal(endpoints(&goal), &Type::Nat, &a, &b);
    assert_eq!(new_goal, eq_nat(add(b.clone(), var("k")), var("k")));
    let restored = family(&motive).substitute_term(SYNTHESISED_MOTIVE_BINDER, &a);
    assert_eq!(restored, goal);
    // Reverse: `b` is abstracted, the new goal has `a`.
    let goal = eq_nat(add(b.clone(), var("k")), var("k"));
    let (motive, new_goal) = motive_and_new_goal(endpoints(&goal), &Type::Nat, &b, &a);
    assert_eq!(new_goal, eq_nat(add(a.clone(), var("k")), var("k")));
    let restored = family(&motive).substitute_term(SYNTHESISED_MOTIVE_BINDER, &b);
    assert_eq!(restored, goal);
}

// ADR 2.10.26e AC2
#[test]
fn the_motive_binder_is_the_combinators_binder() {
    let goal = eq_nat(var("a"), var("c"));
    let (motive, _) = motive_and_new_goal(endpoints(&goal), &Type::Nat, &var("a"), &var("b"));
    match motive {
        Type::Pi(binder, domain, _) => {
            assert_eq!(binder, "#x");
            assert_eq!(*domain, Type::Nat);
        }
        other => panic!("expected a Π motive, got {other:?}"),
    }
}
