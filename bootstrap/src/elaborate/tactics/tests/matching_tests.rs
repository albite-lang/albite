//! AC 3 — `apply`'s first-order matcher: syntactic, span-blind, no conversion.
//! And `rewrite`'s `match_term`, factored out of it (ADR 2.10.26e AC 3).

use std::collections::BTreeMap;

use albite_core::{Term, TermSpan, Type};

use super::{eq_nat, zero_plus};
use crate::elaborate::tactics::matching::{
    instantiate, instantiate_term, match_conclusion, match_term,
};

fn n_var() -> Vec<String> {
    vec!["n".to_string()]
}

/// `Eq Nat (0 + n) n`, the conclusion of `add_zero_left`.
fn add_zero_left_conclusion() -> Type {
    eq_nat(zero_plus(Term::var("n")), Term::var("n"))
}

// ADR 2.10.26c AC3
#[test]
fn a_variable_binds_at_its_first_occurrence() {
    let target = eq_nat(zero_plus(Term::var("k")), Term::var("k"));
    assert_eq!(
        match_conclusion(&add_zero_left_conclusion(), &n_var(), &target),
        Some(BTreeMap::from([("n".to_string(), Term::var("k"))]))
    );
}

// ADR 2.10.26c AC3
#[test]
fn a_repeated_variable_must_agree() {
    let target = eq_nat(zero_plus(Term::var("k")), Term::var("j"));
    assert_eq!(
        match_conclusion(&add_zero_left_conclusion(), &n_var(), &target),
        None
    );
}

// ADR 2.10.26c AC3
#[test]
fn matching_ignores_spans() {
    let spanned_k = Term::Spanned(Box::new(Term::var("k")), TermSpan::new(3, 4));
    let target = eq_nat(zero_plus(spanned_k), Term::var("k"));
    let bindings = match_conclusion(&add_zero_left_conclusion(), &n_var(), &target)
        .expect("a span is not a mismatch");
    assert_eq!(
        bindings["n"],
        Term::var("k"),
        "the binding is stored span-free"
    );
}

// ADR 2.10.26c AC3: `k * 1` computes to `k`, and the matcher must not care.
#[test]
fn matching_does_not_convert() {
    let k_times_one = Term::NatMul(Box::new(Term::var("k")), Box::new(Term::NatLit(1)));
    let target = eq_nat(k_times_one, Term::var("k"));
    assert_eq!(
        match_conclusion(&add_zero_left_conclusion(), &n_var(), &target),
        None
    );
}

// ADR 2.10.26c AC3
#[test]
fn a_rigid_mismatch_fails() {
    // The base type differs.
    let bool_eq = Type::Eq(
        Box::new(Type::Bool),
        Box::new(zero_plus(Term::var("k"))),
        Box::new(Term::var("k")),
    );
    assert_eq!(
        match_conclusion(&add_zero_left_conclusion(), &n_var(), &bool_eq),
        None
    );
    // A rigid operator differs.
    let succ = eq_nat(Term::Succ(Box::new(Term::var("k"))), Term::var("k"));
    assert_eq!(
        match_conclusion(&add_zero_left_conclusion(), &n_var(), &succ),
        None
    );
    // Not an equality at all.
    assert_eq!(
        match_conclusion(&add_zero_left_conclusion(), &n_var(), &Type::Nat),
        None
    );
}

/// A variable that occurs only under a rigid node is still found: the walk
/// descends into nodes of the same shape.
#[test]
fn a_variable_nested_under_a_rigid_node_binds() {
    let pattern = eq_nat(zero_plus(Term::var("n")), Term::Zero);
    let target = eq_nat(zero_plus(Term::var("k")), Term::Zero);
    assert_eq!(
        match_conclusion(&pattern, &n_var(), &target),
        Some(BTreeMap::from([("n".to_string(), Term::var("k"))]))
    );
}

/// A variable absent from the conclusion is left unbound, for the caller to
/// report as "cannot infer".
#[test]
fn a_variable_absent_from_the_conclusion_stays_unbound() {
    let pattern = eq_nat(Term::Zero, Term::Zero);
    let bindings = match_conclusion(&pattern, &n_var(), &pattern).expect("rigid match");
    assert!(bindings.is_empty());
}

/// Matching walks under arrows too, binding inside a premise's `Eq`.
#[test]
fn matching_walks_under_an_arrow() {
    let pattern = Type::arrow(add_zero_left_conclusion(), Type::Nat);
    let target = Type::arrow(eq_nat(zero_plus(Term::Zero), Term::Zero), Type::Nat);
    assert_eq!(
        match_conclusion(&pattern, &n_var(), &target),
        Some(BTreeMap::from([("n".to_string(), Term::Zero)]))
    );
}

/// Instantiation is simultaneous: a binding that spells another variable's
/// name is not rewritten by that variable's own binding.
#[test]
fn instantiate_is_simultaneous() {
    let pattern = eq_nat(Term::var("n"), Term::var("m"));
    let bindings = BTreeMap::from([
        ("m".to_string(), Term::var("n")),
        ("n".to_string(), Term::var("m")),
    ]);
    assert_eq!(
        instantiate(&pattern, &bindings),
        eq_nat(Term::var("m"), Term::var("n"))
    );
}

fn add(left: Term, right: Term) -> Term {
    Term::NatAdd(Box::new(left), Box::new(right))
}

// ADR 2.10.26e AC3: `rewrite`'s entry — a term pattern against one subterm.
#[test]
fn match_term_matches_a_subterm_pattern() {
    let vars = vec!["n".to_string(), "m".to_string()];
    let pattern = add(Term::var("n"), Term::var("m"));
    let target = add(
        add(Term::var("a"), Term::var("b")),
        Term::Spanned(Box::new(Term::var("c")), TermSpan::new(1, 2)),
    );
    let mut bindings = BTreeMap::new();
    assert!(match_term(&pattern, &vars, &target, &mut bindings));
    assert_eq!(bindings["n"], add(Term::var("a"), Term::var("b")));
    assert_eq!(bindings["m"], Term::var("c"), "stored span-free");
    // A rigid mismatch: `succ c` is not a sum.
    let mut bindings = BTreeMap::new();
    assert!(!match_term(
        &pattern,
        &vars,
        &Term::Succ(Box::new(Term::var("c"))),
        &mut bindings
    ));
}

// ADR 2.10.26e AC3: a binding threaded in must be met, not overwritten.
#[test]
fn match_term_respects_a_binding_already_made() {
    let pattern = zero_plus(Term::var("n"));
    let mut bindings = BTreeMap::from([("n".to_string(), Term::var("k"))]);
    assert!(!match_term(
        &pattern,
        &n_var(),
        &zero_plus(Term::var("j")),
        &mut bindings
    ));
    assert!(match_term(
        &pattern,
        &n_var(),
        &zero_plus(Term::var("k")),
        &mut bindings
    ));
}

// ADR 2.10.26e AC3: one map across both endpoints, so `?n` bound in `l`
// refuses an `r` that needs a different `?n`.
#[test]
fn a_repeated_variable_across_both_endpoints_refuses_an_inconsistent_target() {
    let pattern = eq_nat(zero_plus(Term::var("n")), Term::var("n"));
    let consistent = eq_nat(zero_plus(Term::var("k")), Term::var("k"));
    let inconsistent = eq_nat(zero_plus(Term::var("k")), Term::var("j"));
    assert!(match_conclusion(&pattern, &n_var(), &consistent).is_some());
    assert_eq!(match_conclusion(&pattern, &n_var(), &inconsistent), None);
}

// ADR 2.10.26e AC3: term instantiation is simultaneous, like `instantiate`.
#[test]
fn instantiate_term_is_simultaneous() {
    let bindings = BTreeMap::from([
        ("m".to_string(), Term::var("n")),
        ("n".to_string(), Term::var("m")),
    ]);
    assert_eq!(
        instantiate_term(&add(Term::var("n"), Term::var("m")), &bindings),
        add(Term::var("m"), Term::var("n"))
    );
}

/// `exists k: Nat. Eq Nat (left + k) right`, `Le<left, right>`'s expansion.
fn le(binder: &str, left: Term, right: Term) -> Type {
    Type::sigma(
        binder,
        Type::Nat,
        eq_nat(add(left, Term::var(binder)), right),
    )
}

// ADR 6.10.26e AC1: `le_refl`'s conclusion binds `n := a` under the `exists`.
#[test]
fn a_variable_binds_under_an_exists() {
    let pattern = le("k", Term::var("n"), Term::var("n"));
    let target = le("k", Term::var("a"), Term::var("a"));
    assert_eq!(
        match_conclusion(&pattern, &n_var(), &target),
        Some(BTreeMap::from([("n".to_string(), Term::var("a"))]))
    );
    // A differently named binder still binds, and the α-check passes.
    let renamed = le("j", Term::var("a"), Term::var("a"));
    assert!(match_conclusion(&pattern, &n_var(), &renamed).is_some());
    // The binder's type is walked too: `n` occurs only there.
    let zero_eq = eq_nat(Term::Zero, Term::Zero);
    let pattern = Type::sigma("k", eq_nat(Term::var("n"), Term::var("n")), zero_eq.clone());
    let target = Type::sigma("k", eq_nat(Term::var("a"), Term::var("a")), zero_eq);
    assert_eq!(
        match_conclusion(&pattern, &n_var(), &target),
        Some(BTreeMap::from([("n".to_string(), Term::var("a"))]))
    );
}

// ADR 6.10.26e AC1: a lemma variable spelled like the binder is shadowed in
// the body, so it binds nothing there.
#[test]
fn a_lemma_variable_shadowed_by_the_binder_binds_nothing() {
    let vars = vec!["k".to_string()];
    let pattern = le("k", Term::var("a"), Term::var("a"));
    let target = le("k", Term::var("a"), Term::var("a"));
    let bindings = match_conclusion(&pattern, &vars, &target).expect("it matches");
    assert!(bindings.is_empty(), "{bindings:?}");
}

// ADR 6.10.26e AC1: a binding that captures the target's own binder is
// refused by the final α-check.
#[test]
fn a_binding_that_captures_the_targets_binder_is_refused() {
    let pattern = Type::sigma("k", Type::Nat, eq_nat(Term::var("n"), Term::var("k")));
    let target = Type::sigma("j", Type::Nat, eq_nat(Term::var("j"), Term::var("j")));
    assert_eq!(match_conclusion(&pattern, &n_var(), &target), None);
}
