//! `type_handles_equal` against the owned α-equality.

use super::*;
use crate::ffi::types::nodes::import_type;
use crate::terms::Term;
use crate::types::{types_equal_alpha, Type};

/// A corpus spanning every equality rule: base, TyVar/@-prefix, TyVar↔App,
/// TyVar↔Mu (α_ convention), binders (α-renaming both ways), Eq terms,
/// wrappers, App/Adt, poison, and definite inequalities.
fn corpus() -> Vec<(Type, Type)> {
    let mut pairs = corpus_base_rules();
    pairs.extend(corpus_guards_and_binders());
    pairs
}

/// Base types, TyVar conventions, binary/wrapper shapes, Eq terms.
fn corpus_base_rules() -> Vec<(Type, Type)> {
    let list_mu = |var: &str| {
        Type::Mu(
            var.into(),
            Box::new(Type::sum(
                Type::Unit,
                Type::product(Type::Nat, Type::TyVar(var.into())),
            )),
        )
    };
    vec![
        (Type::Nat, Type::Nat),
        (Type::Nat, Type::Bool),
        (Type::TyVar("@Cursor".into()), Type::TyVar("Cursor".into())),
        (Type::TyVar("T".into()), Type::app("T", vec![])),
        (Type::TyVar("T".into()), Type::app("T", vec![Type::Nat])),
        (Type::TyVar("List".into()), list_mu("α_List")),
        (Type::TyVar("Wrong".into()), list_mu("α_List")),
        (list_mu("α_List"), list_mu("β")),
        (
            Type::Forall("a".into(), Box::new(Type::TyVar("a".into()))),
            Type::Forall("b".into(), Box::new(Type::TyVar("b".into()))),
        ),
        (
            Type::Forall("a".into(), Box::new(Type::TyVar("a".into()))),
            Type::Forall("b".into(), Box::new(Type::TyVar("c".into()))),
        ),
        (
            Type::arrow(Type::Nat, Type::Bool),
            Type::arrow(Type::Nat, Type::Bool),
        ),
        (
            Type::arrow(Type::Nat, Type::Bool),
            Type::arrow(Type::Bool, Type::Nat),
        ),
        (
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
        ),
        (
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
            Type::eq(Type::Nat, Term::Zero, Term::Succ(Box::new(Term::Zero))),
        ),
        // Type-part differs, terms match: pins the Eq arm's first conjunct.
        (
            Type::eq(Type::Nat, Term::Zero, Term::Zero),
            Type::eq(Type::Bool, Term::Zero, Term::Zero),
        ),
        (Type::ptr(Type::Nat), Type::ptr(Type::Nat)),
        (Type::ptr(Type::Nat), Type::ref_ty(Type::Nat)),
        (
            Type::app("Forest", vec![Type::Nat]),
            Type::app("Forest", vec![Type::Nat]),
        ),
        (
            Type::app("Forest", vec![Type::Nat]),
            Type::app("Forest", vec![Type::Bool]),
        ),
        (
            Type::adt("Tree", vec![Type::Nat], vec![("Leaf".into(), Type::Unit)]),
            Type::adt("Tree", vec![Type::Nat], vec![("Leaf".into(), Type::Unit)]),
        ),
        (
            Type::adt("Tree", vec![], vec![("Leaf".into(), Type::Unit)]),
            Type::adt("Tree", vec![], vec![("Node".into(), Type::Unit)]),
        ),
        (Type::Error, Type::arrow(Type::Nat, Type::Bool)),
        (
            Type::product(Type::String, Type::Unit),
            Type::product(Type::String, Type::Void),
        ),
    ]
}

/// Guard clauses (App/Adt), one-side-differs conjuncts, env-mapped
/// binder equivalences.
fn corpus_guards_and_binders() -> Vec<(Type, Type)> {
    let list_mu = |var: &str| {
        Type::Mu(
            var.into(),
            Box::new(Type::sum(
                Type::Unit,
                Type::product(Type::Nat, Type::TyVar(var.into())),
            )),
        )
    };
    vec![
        // One-side-differs pairs: pin each conjunct of the binary arms.
        (
            Type::arrow(Type::Nat, Type::Bool),
            Type::arrow(Type::Nat, Type::Nat),
        ),
        (
            Type::arrow(Type::Bool, Type::Bool),
            Type::arrow(Type::Nat, Type::Bool),
        ),
        // App arg-count vs name mismatches (pin each guard clause).
        (
            Type::app("Forest", vec![Type::Nat]),
            Type::app("Forest", vec![Type::Nat, Type::Bool]),
        ),
        (
            Type::app("Forest", vec![Type::Nat]),
            Type::app("Grove", vec![Type::Nat]),
        ),
        // Adt guard clauses: name / type-arg count / variant count.
        (
            Type::adt("Tree", vec![], vec![("Leaf".into(), Type::Unit)]),
            Type::adt("Bush", vec![], vec![("Leaf".into(), Type::Unit)]),
        ),
        (
            Type::adt("Tree", vec![Type::Nat], vec![("Leaf".into(), Type::Unit)]),
            Type::adt("Tree", vec![], vec![("Leaf".into(), Type::Unit)]),
        ),
        (
            Type::adt("Tree", vec![], vec![("Leaf".into(), Type::Unit)]),
            Type::adt(
                "Tree",
                vec![],
                vec![("Leaf".into(), Type::Unit), ("Node".into(), Type::Nat)],
            ),
        ),
        // Env-mapped TyVar↔Mu and TyVar↔App under enclosing binders.
        (
            Type::Forall("a".into(), Box::new(Type::TyVar("a".into()))),
            Type::Forall("X".into(), Box::new(list_mu("α_X"))),
        ),
        (
            Type::Forall("a".into(), Box::new(Type::TyVar("a".into()))),
            Type::Forall("N".into(), Box::new(Type::app("N", vec![]))),
        ),
        (
            Type::Forall("a".into(), Box::new(Type::TyVar("a".into()))),
            Type::Forall("N".into(), Box::new(Type::app("M", vec![]))),
        ),
    ]
}

/// The port must agree with the owned oracle on every corpus pair,
/// in both argument orders.
#[test]
fn handle_equality_matches_owned_oracle() {
    let mut arena = Arena::new();
    for (a, b) in corpus() {
        let ha = import_type(&mut arena, &a);
        let hb = import_type(&mut arena, &b);
        let oracle = types_equal_alpha(&a, &b);
        assert_eq!(
            type_handles_equal(&arena, ha, hb),
            oracle,
            "handle equality diverged from oracle for {a:?} vs {b:?}"
        );
        assert_eq!(
            type_handles_equal(&arena, hb, ha),
            types_equal_alpha(&b, &a),
            "handle equality diverged from oracle (reversed) for {b:?} vs {a:?}"
        );
    }
}

#[test]
fn invalid_handles_are_never_equal() {
    let mut arena = Arena::new();
    let nat = import_type(&mut arena, &Type::Nat);
    let invalid = crate::ffi::INVALID_HANDLE;
    assert!(!type_handles_equal(&arena, nat, invalid));
    assert!(!type_handles_equal(&arena, invalid, nat));
    assert!(!type_handles_equal(&arena, invalid, invalid));
}

/// Sharing must not confuse the env logic: the same shared child handle
/// compared under different binder mappings.
#[test]
fn shared_children_compare_correctly_under_binders() {
    let mut arena = Arena::new();
    let a = Type::Forall("a".into(), Box::new(Type::TyVar("a".into())));
    let b = Type::Forall("b".into(), Box::new(Type::TyVar("b".into())));
    let ha = import_type(&mut arena, &a);
    let hb = import_type(&mut arena, &b);
    assert!(type_handles_equal(&arena, ha, hb));
    // Same handle on both sides is equal (identity ⇒ α-equal).
    assert!(type_handles_equal(&arena, ha, ha));
}

/// The oracle test above is vacuous over an empty corpus, so each half is
/// pinned non-empty (moved here by 6.10.26h, whose sweep scored the helpers).
#[test]
fn the_equality_corpora_are_populated() {
    let base = corpus_base_rules().len();
    let guards = corpus_guards_and_binders().len();
    assert!(base > 5 && guards > 5, "{base} / {guards}");
    assert_eq!(corpus().len(), base + guards);
}

/// 6.10.26h D10: the predicate forms take the owned walk, which renames
/// binders and compares `PredApp` arguments.
#[test]
fn predicate_forms_compare_through_the_owned_walk() {
    let mut arena = Arena::new();
    let app = |var: &str, arg: Term| Type::pred_app(var, vec![arg]);
    let binder = |var: &str| Type::forall_pred(var, vec![Type::Nat], app(var, Term::Zero));
    let pairs = [
        (binder("P"), binder("Q"), true),
        (app("P", Term::Zero), app("P", Term::Zero), true),
        (app("P", Term::Zero), app("P", Term::Unit), false),
        (binder("P"), Type::Nat, false),
    ];
    for (a, b, want) in pairs {
        let (ha, hb) = (import_type(&mut arena, &a), import_type(&mut arena, &b));
        assert_eq!(type_handles_equal(&arena, ha, hb), want, "{a} vs {b}");
    }
}
