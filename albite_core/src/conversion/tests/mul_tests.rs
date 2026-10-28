//! ADR 30.9.26e AC 5: primitive `*` computes on its right operand, arm by arm.
//! Pure: every test builds its terms directly and names only core types, so
//! it moves with `whnf/` when the δ-free half moves below the elaborator.

use std::collections::HashMap;

use crate::eval::{term_to_nat, EvalStopped};
use crate::Term;

use super::tests::{head, same};
use super::*;

fn var(name: &str) -> Term {
    Term::var(name)
}

fn numeral(term: &Term) -> Option<u64> {
    term_to_nat(term).map(|value| value as u64)
}

fn successors(count: u64) -> Term {
    (0..count).fold(Term::Zero, |pred, _| Term::succ(pred))
}

// 30.9.26e AC5: each arm of D2's rule, in `add`'s order.
#[test]
fn multiplication_reduces_on_its_right_operand() {
    let n = || var("n");
    assert_eq!(head(Term::nat_mul(n(), Term::Zero)), Term::Zero);
    assert_eq!(head(Term::nat_mul(n(), Term::NatLit(0))), Term::Zero);
    assert_eq!(
        head(Term::nat_mul(n(), Term::succ(var("k")))),
        Term::nat_add(Term::nat_mul(n(), var("k")), n())
    );
    assert_eq!(
        head(Term::nat_mul(n(), Term::NatLit(3))),
        Term::nat_add(Term::nat_mul(n(), Term::NatLit(2)), n()),
        "a literal multiplier unrolls exactly one step"
    );
    let product = head(Term::nat_mul(Term::NatLit(4), Term::NatLit(5)));
    assert_eq!(numeral(&product), Some(20));
    // ADR 1.10.26c D4: an overflowing product is not reduced, where the
    // evaluator saturates.
    let overflowing = Term::nat_mul(Term::NatLit(u64::MAX), Term::NatLit(2));
    assert_eq!(
        head(overflowing.clone()),
        overflowing,
        "an overflow stays neutral"
    );
}

// 30.9.26e AC5: a left-operand recursion does not exist, and an open right
// operand is not an error.
#[test]
fn multiplication_by_an_open_right_operand_is_neutral() {
    let left_zero = Term::nat_mul(Term::NatLit(0), var("n"));
    assert_eq!(head(left_zero.clone()), left_zero);
    let open = Term::nat_mul(var("n"), var("m"));
    assert_eq!(head(open.clone()), open);
    // The right operand is rebuilt reduced, as `add` does.
    let wrapped = Term::nat_mul(
        var("n"),
        Term::app(Term::lambda("x", crate::Type::Nat, var("x")), var("m")),
    );
    assert_eq!(head(wrapped), Term::nat_mul(var("n"), var("m")));
    // The rule is called only on its own form; any other comes back as it was.
    let other = Term::nat_add(var("n"), var("m"));
    let step = super::rules::mul(&other, &HashMap::new(), &mut Fuel::new(10)).unwrap();
    assert!(matches!(step, Step::Neutral(ref t) if *t == other));
}

// 30.9.26e AC5: D2's soundness argument, checked — the rule agrees with the
// evaluator's `*` on every closed instance of the grid wherever the evaluator
// does not saturate, and leaves an overflowing instance unreduced (ADR
// 1.10.26c D4).
#[test]
fn multiplication_agrees_with_the_evaluator_on_closed_operands() {
    for a in [0, 1, 2, 5, u64::MAX] {
        for b in 0..=5u64 {
            let reduced = head(Term::nat_mul(Term::NatLit(a), successors(b)));
            match a.checked_mul(b) {
                Some(product) => assert_eq!(numeral(&reduced), Some(product), "{a} * {b}"),
                None => assert_eq!(numeral(&reduced), None, "{a} * {b} overflows"),
            }
        }
    }
}

// 1.10.26c AC2: no rule reduces past `u64::MAX` — the `+` and `*` literal
// rules, a `Succ` chain read as a literal, and a closed primitive — so
// conversion never equates `MAX + 1` with `MAX`.
#[test]
fn overflow_is_never_reduced() {
    let max = || Term::NatLit(u64::MAX);
    let sum = Term::nat_add(max(), Term::NatLit(1));
    assert_eq!(head(sum.clone()), sum, "MAX + 1 stays neutral");
    let product = Term::nat_mul(max(), Term::NatLit(2));
    assert_eq!(head(product.clone()), product, "MAX * 2 stays neutral");
    let successor = Term::succ(max());
    assert_eq!(head(successor.clone()), successor);
    // A `Succ` over `MAX` is not read as a literal, so `succ(MAX) == MAX`
    // never reaches the evaluator to be compared saturated.
    let compared = Term::NatEq(Box::new(successor), Box::new(max()));
    assert_eq!(head(compared.clone()), compared);
    // A signed overflow traps in the evaluator, so it stays neutral too.
    let signed = Term::int_bin(
        crate::terms::IntBinOp::Add,
        Term::IntLit(i64::MAX),
        Term::IntLit(1),
    );
    assert_eq!(head(signed.clone()), signed);
    let widened = Term::NatToInt(Box::new(max()));
    assert_eq!(head(widened.clone()), widened);
    assert!(matches!(
        same(Term::nat_add(max(), Term::NatLit(1)), max()),
        Convertible::No { .. }
    ));
    // Below the boundary, both rules still compute.
    let below = head(Term::nat_add(Term::NatLit(u64::MAX - 1), Term::NatLit(1)));
    assert_eq!(numeral(&below), Some(u64::MAX));
    let half = head(Term::nat_mul(Term::NatLit(u64::MAX / 2), Term::NatLit(2)));
    assert_eq!(numeral(&half), Some(u64::MAX - 1));
}

// 30.9.26e AC5: conversion meets `+`'s convention, and `0 * n` needs induction.
#[test]
fn multiplication_converts_by_its_successor_rule() {
    let n_times_succ_m = Term::nat_mul(var("n"), Term::nat_add(var("m"), Term::NatLit(1)));
    let n_times_m_plus_n = Term::nat_add(Term::nat_mul(var("n"), var("m")), var("n"));
    assert_eq!(same(n_times_succ_m, n_times_m_plus_n), Convertible::Yes);
    let Convertible::No { first_mismatch } = same(Term::nat_mul(Term::Zero, var("n")), Term::Zero)
    else {
        panic!("`0 * n` is not definitionally `0`");
    };
    assert_eq!(first_mismatch.1, Term::Zero);
    assert!(matches!(first_mismatch.0, Term::NatMul(..)));
    let thrice = Term::nat_mul(var("n"), Term::NatLit(3));
    let summed = Term::nat_add(Term::nat_add(var("n"), var("n")), var("n"));
    assert!(matches!(same(thrice, summed), Convertible::No { .. }));
}

// 30.9.26e AC5: a large literal multiplier costs budget, never stack.
#[test]
fn multiplication_by_a_long_literal_is_bounded_by_the_budget() {
    let big = |name: &str| Term::nat_mul(var(name), Term::NatLit(100_000));
    let (left, right) = rename_apart(&big("n"), &big("m"));
    assert!(matches!(
        convertible(&left, &right, &HashMap::new(), 1_000_000),
        Convertible::No { .. }
    ));
    let verdict = convertible(&left, &right, &HashMap::new(), 5);
    assert!(matches!(
        verdict,
        Convertible::Stopped(EvalStopped::StepLimit { limit: 5 }) | Convertible::No { .. }
    ));
    // `n * 10` against `m * 10` first differs only at the bottom of ten
    // unrolled steps, so five firings cannot reach it.
    let ten = |name: &str| Term::nat_mul(var(name), Term::NatLit(10));
    let (left, right) = rename_apart(&ten("n"), &ten("m"));
    assert!(matches!(
        convertible(&left, &right, &HashMap::new(), 10_000),
        Convertible::No { .. }
    ));
    assert_eq!(
        convertible(&left, &right, &HashMap::new(), 5),
        Convertible::Stopped(EvalStopped::StepLimit { limit: 5 })
    );
}
