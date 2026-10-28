//! ADR 30.9.26g AC 3: `convertible_without_delta`, the one fast path both
//! compilers call. Pure: every test builds its terms directly.

use crate::{Term, Type};

use super::{convertible, convertible_without_delta, rename_apart, Convertible};

fn var(name: &str) -> Term {
    Term::var(name)
}

// 30.9.26g AC3: computation needs no definition — `eval ==` and the WHNF rules.
#[test]
fn the_fast_path_accepts_what_needs_no_definition() {
    assert!(convertible_without_delta(
        &Term::nat_add(var("k"), Term::NatLit(0)),
        &var("k")
    ));
    assert!(convertible_without_delta(
        &Term::nat_add(Term::NatLit(2), Term::NatLit(2)),
        &Term::NatLit(4)
    ));
    assert!(convertible_without_delta(
        &Term::nat_mul(var("k"), Term::NatLit(0)),
        &Term::NatLit(0)
    ));
    assert!(convertible_without_delta(
        &Term::nat_add(Term::nat_add(var("k"), Term::NatLit(1)), Term::NatLit(0)),
        &Term::nat_add(Term::nat_add(var("k"), Term::NatLit(0)), Term::NatLit(1))
    ));
}

// 30.9.26g AC3: `+` and `*` compute on their right operand only.
#[test]
fn the_fast_path_refuses_what_needs_induction() {
    assert!(!convertible_without_delta(
        &Term::nat_add(Term::NatLit(0), var("k")),
        &var("k")
    ));
    assert!(!convertible_without_delta(
        &Term::nat_mul(var("k"), Term::NatLit(1)),
        &var("k")
    ));
    assert!(!convertible_without_delta(
        &Term::nat_add(Term::NatLit(2), Term::NatLit(2)),
        &Term::NatLit(5)
    ));
}

// 30.9.26g AC3: a global is neutral even when its body is the identity — the
// property that makes the FFI safe before admission, and part (b)'s gap.
#[test]
fn the_fast_path_never_unfolds_a_global() {
    let call = Term::app(Term::global("same"), var("k"));
    assert!(!convertible_without_delta(&call, &var("k")));
    // Under a map holding `same`'s body the engine WOULD say yes: the refusal
    // above is the empty map's, not a missing rule's.
    let identity = Term::lambda("x", Type::Nat, var("x"));
    let globals = [("same".to_string(), identity)].into();
    let (left, right) = rename_apart(&call, &var("k"));
    assert_eq!(
        convertible(&left, &right, &globals, 1_000),
        Convertible::Yes
    );
}

// 30.9.26g AC3: 29.9.26c D5's reference-cell pair agrees only by name, so
// the fast path keeps CBV's 1 ≠ 0.
#[test]
fn the_fast_path_refuses_an_effectful_pair() {
    let cell = || Term::ref_new(Term::Zero);
    let written = Term::let_in(
        "r",
        Type::Nat,
        cell(),
        Term::let_in(
            "_",
            Type::Unit,
            Term::ref_set(var("r"), Term::NatLit(1)),
            Term::ref_get(var("r")),
        ),
    );
    let read = Term::let_in("r", Type::Nat, cell(), Term::ref_get(var("r")));
    let (left, right) = rename_apart(&written, &read);
    assert_eq!(
        convertible(&left, &right, &Default::default(), 10_000),
        Convertible::Yes,
        "by name both sides reach `!(ref 0)` — the hazard the guard exists for"
    );
    assert!(!convertible_without_delta(&written, &read));
    assert!(!convertible_without_delta(&read, &written));
}

// 30.9.26g AC3 / 29.9.26c D5: ONE effectful side keeps WHNF out, in either
// position — by name the unregistered extern is dropped and the sides agree.
#[test]
fn one_effectful_side_refuses_the_fast_path() {
    let dropped = Term::let_in(
        "_",
        Type::Unit,
        Term::extern_call("tg_not_in_the_registry", vec![]),
        Term::Zero,
    );
    let (left, right) = rename_apart(&dropped, &Term::Zero);
    assert_eq!(
        convertible(&left, &right, &Default::default(), 1_000),
        Convertible::Yes,
        "by name, the extern is dropped"
    );
    assert!(!convertible_without_delta(&dropped, &Term::Zero));
    assert!(!convertible_without_delta(&Term::Zero, &dropped));
}

// 30.9.26g D8: a budget stop is a refusal, never "equal" and never a panic.
#[test]
fn the_fast_path_refuses_when_the_budget_stops() {
    // `k + N` against `k + (N + 1)`: WHNF walks N successors per side before
    // the heads differ; far past the budget it stops, and stopping is `false`.
    let long = super::CONVERSION_STEP_BUDGET as u64;
    let left = Term::nat_add(var("k"), Term::NatLit(long));
    let right = Term::nat_add(var("k"), Term::NatLit(long + 1));
    assert!(!convertible_without_delta(&left, &right));
}

fn nat_sum(a: u64, b: u64) -> Term {
    Term::nat_add(Term::NatLit(a), Term::NatLit(b))
}

// 13.10.26e AC4: §1.1's false rows. Before the fix, `eval` stepped a nested
// operand under the wrong operator, or saturated at the largest Nat, and the
// `eval ==` arm accepted each of these.
#[test]
fn the_fast_path_refuses_false_nested_or_overflowing_arithmetic() {
    let max = || Term::NatLit(u64::MAX);
    let lit = Term::NatLit;
    let false_rows: [(&str, Term, Term); 11] = [
        ("MAX + 1 == MAX", Term::nat_add(max(), lit(1)), max()),
        (
            "MAX + 1 == MAX + 2",
            Term::nat_add(max(), lit(1)),
            Term::nat_add(max(), lit(2)),
        ),
        ("MAX * MAX == MAX", Term::nat_mul(max(), max()), max()),
        (
            "(2 + 1) - 1 == 4",
            Term::nat_sub(nat_sum(2, 1), lit(1)),
            lit(4),
        ),
        (
            "(2 + 1) * 2 == 5",
            Term::nat_mul(nat_sum(2, 1), lit(2)),
            lit(5),
        ),
        (
            "(3 + 3) / 2 == 8",
            Term::nat_div(nat_sum(3, 3), lit(2)),
            lit(8),
        ),
        (
            "(3 + 3) % 4 == 10",
            Term::nat_mod(nat_sum(3, 3), lit(4)),
            lit(10),
        ),
        (
            "((1 + 2) == 3) == false",
            Term::nat_eq(nat_sum(1, 2), lit(3)),
            Term::False,
        ),
        (
            "((1 + 1) >= 3) == true",
            Term::nat_ge(nat_sum(1, 1), lit(3)),
            Term::True,
        ),
        (
            "10 - (2 + 1) == 13",
            Term::nat_sub(lit(10), nat_sum(2, 1)),
            lit(13),
        ),
        (
            "((1 == 1) || false) == false",
            Term::bool_or(Term::nat_eq(lit(1), lit(1)), Term::False),
            Term::False,
        ),
    ];
    for (label, left, right) in false_rows {
        assert!(
            !convertible_without_delta(&left, &right),
            "accepted {label}"
        );
    }
    assert!(
        !convertible_without_delta(&nat_sum(5, 1), &lit(5)),
        "control"
    );
}

// 13.10.26e AC4: §1.1's true rows, and a stuck overflow is equal to itself.
#[test]
fn the_fast_path_still_accepts_true_nested_arithmetic() {
    let lit = Term::NatLit;
    let overflow = Term::nat_add(lit(u64::MAX), lit(1));
    let true_rows: [(&str, Term, Term); 3] = [
        (
            "(2 + 1) - 1 == 2",
            Term::nat_sub(nat_sum(2, 1), lit(1)),
            lit(2),
        ),
        (
            "10 - (2 + 1) == 7",
            Term::nat_sub(lit(10), nat_sum(2, 1)),
            lit(7),
        ),
        ("MAX + 1 == MAX + 1", overflow.clone(), overflow),
    ];
    for (label, left, right) in true_rows {
        assert!(convertible_without_delta(&left, &right), "refused {label}");
    }
}
