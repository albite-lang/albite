//! `Eq` types compare their terms modulo source spans (ADR 29.9.26d AC 7, D6).

use crate::terms::{Term, TermSpan};
use crate::types::{types_equal_alpha, Type};

/// `term` under a point span at `start`: distinct starts, distinct spans.
fn spanned(term: Term, start: u32) -> Term {
    Term::spanned(term, TermSpan::new(start, start))
}

fn call(argument: Term, start: u32) -> Term {
    spanned(Term::app(Term::Global("f".into()), argument), start)
}

/// 29.9.26d AC7: two `Eq` types differing only in `Spanned` wrappers are equal.
#[test]
fn eq_types_differing_only_in_spans_are_equal() {
    let written = Type::eq(
        Type::Nat,
        call(Term::var("n"), 10),
        call(Term::var("n"), 20),
    );
    let restated = Type::eq(
        Type::Nat,
        call(Term::var("n"), 40),
        call(Term::var("n"), 50),
    );
    assert_ne!(
        written, restated,
        "the spans differ, so the derived == does"
    );
    assert!(types_equal_alpha(&written, &restated));
    // A span on one side only is still a span.
    let unspanned = Type::eq(
        Type::Nat,
        Term::app(Term::Global("f".into()), Term::var("n")),
        call(Term::var("n"), 50),
    );
    assert!(types_equal_alpha(&written, &unspanned));
}

/// 29.9.26d AC7: two `Eq` types differing in a subterm are not equal.
#[test]
fn eq_types_differing_in_a_subterm_are_not_equal() {
    let left = Type::eq(Type::Nat, call(Term::var("n"), 10), Term::Zero);
    let other_argument = Type::eq(Type::Nat, call(Term::var("m"), 40), Term::Zero);
    let other_right = Type::eq(Type::Nat, call(Term::var("n"), 40), Term::succ(Term::Zero));
    let other_base = Type::eq(Type::Bool, call(Term::var("n"), 40), Term::Zero);
    assert!(!types_equal_alpha(&left, &other_argument));
    assert!(!types_equal_alpha(&left, &other_right));
    assert!(!types_equal_alpha(&left, &other_base));
}

/// 29.9.26d AC7: the walk compares labels — names, literals, operators and
/// arm tags — not just the shape.
#[test]
fn the_walk_compares_every_label() {
    let pairs = [
        (Term::var("a"), Term::var("b")),
        (Term::Global("f".into()), Term::Global("g".into())),
        (Term::NatLit(1), Term::NatLit(2)),
        (Term::IntLit(1), Term::IntLit(2)),
        (Term::StringLit("a".into()), Term::StringLit("b".into())),
        (
            Term::lambda("x", Type::Nat, Term::Zero),
            Term::lambda("y", Type::Nat, Term::Zero),
        ),
        (
            Term::lambda("x", Type::Nat, Term::Zero),
            Term::lambda("x", Type::Bool, Term::Zero),
        ),
        (
            Term::let_in("x", Type::Nat, Term::Zero, Term::Zero),
            Term::let_in("y", Type::Nat, Term::Zero, Term::Zero),
        ),
        (
            Term::fix("x", Type::Nat, Term::Zero),
            Term::fix("y", Type::Nat, Term::Zero),
        ),
        (Term::ty_abs("a", Term::Zero), Term::ty_abs("b", Term::Zero)),
        (
            Term::extern_call("f", vec![Term::Zero]),
            Term::extern_call("g", vec![Term::Zero]),
        ),
        (
            Term::extern_call("f", vec![Term::Zero]),
            Term::extern_call("f", vec![Term::Zero, Term::Zero]),
        ),
        (
            Term::int_bin(crate::terms::IntBinOp::Add, Term::Zero, Term::Zero),
            Term::int_bin(crate::terms::IntBinOp::Sub, Term::Zero, Term::Zero),
        ),
        (
            Term::AdtConstruct(Type::Nat, 0, Box::new(Term::Zero)),
            Term::AdtConstruct(Type::Nat, 1, Box::new(Term::Zero)),
        ),
        (
            Term::case(Term::Zero, "x", Term::Zero, "y", Term::Zero),
            Term::case(Term::Zero, "x", Term::Zero, "z", Term::Zero),
        ),
        (
            Term::case(Term::Zero, "x", Term::Zero, "y", Term::Zero),
            Term::case(Term::Zero, "w", Term::Zero, "y", Term::Zero),
        ),
        (
            Term::adt_match(Term::Zero, vec![(0, "x".into(), Box::new(Term::Zero))]),
            Term::adt_match(Term::Zero, vec![(1, "x".into(), Box::new(Term::Zero))]),
        ),
        (
            Term::adt_match(Term::Zero, vec![(0, "x".into(), Box::new(Term::Zero))]),
            Term::adt_match(Term::Zero, vec![(0, "y".into(), Box::new(Term::Zero))]),
        ),
        (
            Term::adt_match(Term::Zero, vec![(0, "x".into(), Box::new(Term::Zero))]),
            Term::adt_match(Term::Zero, vec![]),
        ),
        (
            Term::natind("k", Type::Nat, Term::Zero, Term::Zero, Term::Zero),
            Term::natind("q", Type::Nat, Term::Zero, Term::Zero, Term::Zero),
        ),
        // Two embedded types: the walk must compare the first as well as the last.
        (
            Term::subst(Type::Nat, Type::Unit, Term::Zero, Term::Zero),
            Term::subst(Type::Bool, Type::Unit, Term::Zero, Term::Zero),
        ),
        (Term::Zero, Term::succ(Term::Zero)),
        (
            Term::nat_add(Term::Zero, Term::Zero),
            Term::nat_sub(Term::Zero, Term::Zero),
        ),
    ];
    for (left, right) in pairs {
        let spanned_left = Type::eq(Type::Nat, spanned(left.clone(), 1), Term::Zero);
        let spanned_right = Type::eq(Type::Nat, spanned(right.clone(), 9), Term::Zero);
        assert!(
            !types_equal_alpha(&spanned_left, &spanned_right),
            "{left:?} and {right:?} compared equal"
        );
        let same = Type::eq(Type::Nat, spanned(left.clone(), 9), Term::Zero);
        assert!(
            types_equal_alpha(&spanned_left, &same),
            "{left:?} is not equal to itself"
        );
    }
}

/// 29.9.26d AC7: `labels_equal`'s fall-through arm is label-free variants
/// only; a labelled variant reaching it is a mismatched pair.
#[test]
fn a_labelled_pair_of_different_variants_has_unequal_labels() {
    use crate::types::equality::modulo_spans::labels_equal;
    assert!(!labels_equal(&Term::var("x"), &Term::Global("x".into())));
    assert!(labels_equal(&Term::Zero, &Term::Zero));
}
