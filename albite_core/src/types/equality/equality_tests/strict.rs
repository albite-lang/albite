//! `types_equal_strict` (ADR 29.9.26e D11): each shortcut `types_equal_alpha`
//! takes is refused, and plain α-renaming still agrees.

use super::*;
use crate::terms::{Term, TermSpan};

fn tv(name: &str) -> Type {
    Type::TyVar(name.into())
}

/// Every pair here is accepted by the executable equality and refused by the
/// strict one — the shortcut closed by D11, one per row.
#[test]
fn strict_refuses_each_shortcut_alpha_takes() {
    let positive_mu = Type::mu("a", Type::product(Type::Void, tv("a")));
    let cases = [
        // type-variable/μ: `a` "equals" any μ binding `a`
        (tv("a"), positive_mu.clone()),
        (positive_mu, tv("a")),
        // the `α_` spelling of the same shortcut
        (tv("B"), Type::mu("α_B", Type::arrow(tv("B"), Type::Void))),
        // `@` stripping on free variables
        (tv("@List"), tv("List")),
        // a 0-arity App against a type variable
        (tv("List"), Type::App("List".into(), vec![])),
        (Type::App("List".into(), vec![]), tv("List")),
        // poison against anything
        (Type::Error, Type::Nat),
        (Type::Void, Type::Error),
    ];
    for (left, right) in cases {
        assert!(types_equal_alpha(&left, &right), "{left} vs {right}");
        assert!(!types_equal_strict(&left, &right), "{left} vs {right}");
    }
}

#[test]
fn strict_agrees_with_alpha_on_renamed_binders() {
    let pairs = [
        (
            Type::mu("a", Type::sum(Type::Unit, tv("a"))),
            Type::mu("b", Type::sum(Type::Unit, tv("b"))),
        ),
        (
            Type::forall("a", Type::arrow(tv("a"), tv("a"))),
            Type::forall("b", Type::arrow(tv("b"), tv("b"))),
        ),
        (
            Type::mu("x", Type::mu("y", Type::product(tv("x"), tv("y")))),
            Type::mu("p", Type::mu("q", Type::product(tv("p"), tv("q")))),
        ),
    ];
    for (left, right) in pairs {
        assert!(types_equal_alpha(&left, &right));
        assert!(types_equal_strict(&left, &right), "{left} vs {right}");
    }
}

#[test]
fn strict_keeps_binder_structure() {
    // μx. μy. x × y  ≢  μx. μy. y × x — the binders cross.
    let crossed = (
        Type::mu("x", Type::mu("y", Type::product(tv("x"), tv("y")))),
        Type::mu("x", Type::mu("y", Type::product(tv("y"), tv("x")))),
    );
    assert!(!types_equal_strict(&crossed.0, &crossed.1));
    // A bound variable never equals a free one of the same spelling.
    let bound_vs_free = (
        Type::forall("a", Type::arrow(tv("a"), tv("b"))),
        Type::forall("b", Type::arrow(tv("b"), tv("b"))),
    );
    assert!(!types_equal_strict(&bound_vs_free.0, &bound_vs_free.1));
    assert!(!types_equal_strict(&bound_vs_free.1, &bound_vs_free.0));
    // μ and ∀ are different binders.
    assert!(!types_equal_strict(
        &Type::mu("a", tv("a")),
        &Type::forall("a", tv("a"))
    ));
    // Free variables compare by spelling.
    assert!(types_equal_strict(&tv("a"), &tv("a")));
    assert!(!types_equal_strict(&tv("a"), &tv("b")));
}

#[test]
fn strict_compares_every_constructor_structurally() {
    let same = [
        Type::Nat,
        Type::arrow(Type::Nat, Type::Bool),
        Type::product(Type::Nat, Type::String),
        Type::sum(Type::Bool, Type::Unit),
        Type::ptr(Type::Nat),
        Type::Ref(Box::new(Type::Nat)),
        Type::App("List".into(), vec![Type::Nat]),
        Type::Adt(
            "Pick".into(),
            vec![Type::Nat],
            vec![("PA".into(), Type::Unit)],
        ),
        Type::eq(Type::Nat, Term::Zero, Term::Zero),
        Type::Error,
    ];
    for ty in &same {
        assert!(types_equal_strict(ty, ty), "{ty}");
    }
    let differ = [
        (Type::Nat, Type::Bool),
        (
            Type::arrow(Type::Nat, Type::Bool),
            Type::arrow(Type::Nat, Type::Nat),
        ),
        (
            Type::arrow(Type::Nat, Type::Bool),
            Type::arrow(Type::Bool, Type::Bool),
        ),
        (
            Type::arrow(Type::Nat, Type::Bool),
            Type::product(Type::Nat, Type::Bool),
        ),
        (Type::ptr(Type::Nat), Type::ptr(Type::Bool)),
        (Type::ptr(Type::Nat), Type::Ref(Box::new(Type::Nat))),
        (
            Type::App("List".into(), vec![Type::Nat]),
            Type::App("Vec".into(), vec![Type::Nat]),
        ),
        (
            Type::App("List".into(), vec![Type::Nat]),
            Type::App("List".into(), vec![Type::Bool]),
        ),
        (
            Type::App("List".into(), vec![Type::Nat]),
            Type::App("List".into(), vec![]),
        ),
        (
            Type::Adt("P".into(), vec![], vec![("A".into(), Type::Unit)]),
            Type::Adt("Q".into(), vec![], vec![("A".into(), Type::Unit)]),
        ),
        (
            Type::Adt("P".into(), vec![Type::Nat], vec![("A".into(), Type::Unit)]),
            Type::Adt("P".into(), vec![Type::Bool], vec![("A".into(), Type::Unit)]),
        ),
        (
            Type::Adt("P".into(), vec![], vec![("A".into(), Type::Unit)]),
            Type::Adt("P".into(), vec![], vec![("B".into(), Type::Unit)]),
        ),
        (
            Type::Adt("P".into(), vec![], vec![("A".into(), Type::Unit)]),
            Type::Adt("P".into(), vec![], vec![("A".into(), Type::Nat)]),
        ),
        (
            Type::Adt("P".into(), vec![], vec![("A".into(), Type::Unit)]),
            Type::Adt(
                "P".into(),
                vec![],
                vec![("A".into(), Type::Unit), ("B".into(), Type::Unit)],
            ),
        ),
        (
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
            Type::eq(Type::Bool, Term::Zero, Term::Zero),
        ),
        (
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
            Type::eq(Type::Nat, Term::NatLit(1), Term::Zero),
        ),
        (
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
            Type::eq(Type::Nat, Term::Zero, Term::NatLit(1)),
        ),
    ];
    for (left, right) in &differ {
        assert!(!types_equal_strict(left, right), "{left} vs {right}");
    }
}

#[test]
fn strict_eq_witnesses_compare_modulo_spans() {
    let span = TermSpan::default();
    let spanned = Term::Spanned(Box::new(Term::Zero), span);
    let left = Type::eq(Type::Nat, spanned, Term::Zero);
    let right = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    assert!(types_equal_strict(&left, &right));
}
