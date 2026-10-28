//! `substitute_type_handle` against the owned `Type::substitute` oracle.

use super::*;
use crate::ffi::types::nodes::import_type;
use crate::terms::Term;

/// (type, var, replacement) triples covering every arm: leafs, hits,
/// misses, binder shadowing (Forall/Mu), Eq term descent, App/Adt args.
fn corpus() -> Vec<(Type, &'static str, Type)> {
    let tv = |s: &str| Type::TyVar(s.into());
    vec![
        (Type::Nat, "a", Type::Bool),
        (tv("a"), "a", Type::Nat),
        (tv("b"), "a", Type::Nat),
        (Type::arrow(tv("a"), tv("b")), "a", Type::Nat),
        (
            Type::product(tv("a"), tv("a")),
            "a",
            Type::arrow(Type::Nat, Type::Bool),
        ),
        (Type::sum(Type::Unit, tv("a")), "a", Type::Void),
        // Shadowed: binder of the same name blocks substitution.
        (Type::Forall("a".into(), Box::new(tv("a"))), "a", Type::Nat),
        (
            Type::Forall("b".into(), Box::new(Type::arrow(tv("a"), tv("b")))),
            "a",
            Type::Nat,
        ),
        (
            Type::Mu("a".into(), Box::new(Type::sum(Type::Unit, tv("a")))),
            "a",
            Type::Nat,
        ),
        // The μ-unfold shape: substitute the binder var in the BODY.
        (
            Type::sum(Type::Unit, Type::product(Type::Nat, tv("α_List"))),
            "α_List",
            Type::Mu(
                "α_List".into(),
                Box::new(Type::sum(
                    Type::Unit,
                    Type::product(Type::Nat, tv("α_List")),
                )),
            ),
        ),
        (
            Type::eq(
                tv("a"),
                Term::Zero,
                Term::Annot(Box::new(Term::Zero), tv("a")),
            ),
            "a",
            Type::Nat,
        ),
        // One-side-changed Eq cases: pin each conjunct of the eq-arm's
        // share detection (type-only, lhs-only, rhs-only changes).
        (Type::eq(tv("a"), Term::Zero, Term::Zero), "a", Type::Nat),
        (
            Type::eq(
                Type::Nat,
                Term::Annot(Box::new(Term::Zero), tv("a")),
                Term::Zero,
            ),
            "a",
            Type::Nat,
        ),
        (
            Type::eq(
                Type::Nat,
                Term::Zero,
                Term::Annot(Box::new(Term::Zero), tv("a")),
            ),
            "a",
            Type::Nat,
        ),
        (Type::ptr(tv("a")), "a", Type::Nat),
        (Type::ref_ty(tv("a")), "a", Type::Nat),
        (
            Type::app("Forest", vec![tv("a"), Type::Nat]),
            "a",
            Type::Bool,
        ),
        (
            Type::adt(
                "Tree",
                vec![tv("a")],
                vec![("Leaf".into(), Type::Unit), ("Node".into(), tv("a"))],
            ),
            "a",
            Type::Nat,
        ),
        // One-side-changed Adt cases: pin each conjunct of the adt-arm's
        // share detection (type-args-only, variants-only changes).
        (
            Type::adt("Tree", vec![tv("a")], vec![("Leaf".into(), Type::Unit)]),
            "a",
            Type::Nat,
        ),
        (
            Type::adt("Tree", vec![Type::Nat], vec![("Node".into(), tv("a"))]),
            "a",
            Type::Nat,
        ),
    ]
}

/// The port must produce exactly what the owned `Type::substitute`
/// produces, for every corpus triple.
#[test]
fn substitute_matches_owned_oracle() {
    let mut arena = Arena::new();
    for (ty, var, repl) in corpus() {
        let h_ty = import_type(&mut arena, &ty);
        let h_repl = import_type(&mut arena, &repl);
        let result = substitute_type_handle(&mut arena, h_ty, var, h_repl).expect("valid handles");
        let materialized = materialize_type(&arena, result).expect("valid result");
        let oracle = ty.substitute(var, &repl);
        assert_eq!(
            materialized, oracle,
            "substitute diverged from oracle: {ty:?}[{var} := {repl:?}]"
        );
    }
}

/// Untouched subtrees must share: substituting a var that does not occur
/// returns the original handle and allocates nothing.
#[test]
fn no_occurrence_shares_the_original_handle() {
    let mut arena = Arena::new();
    let ty = Type::arrow(
        Type::product(Type::Nat, Type::Bool),
        Type::sum(Type::Unit, Type::String),
    );
    let h_ty = import_type(&mut arena, &ty);
    let h_repl = import_type(&mut arena, &Type::Nat);
    let nodes_before = arena.types.len();
    let result = substitute_type_handle(&mut arena, h_ty, "absent", h_repl).unwrap();
    assert_eq!(result, h_ty, "unchanged tree must return the same handle");
    assert_eq!(arena.types.len(), nodes_before, "no new nodes allocated");
}

/// A hit deep on one path must rebuild ONLY that spine: siblings share.
#[test]
fn partial_hit_rebuilds_only_the_touched_spine() {
    let mut arena = Arena::new();
    let big_untouched = Type::product(
        Type::arrow(Type::Nat, Type::Bool),
        Type::arrow(Type::String, Type::Unit),
    );
    let ty = Type::arrow(big_untouched, Type::TyVar("a".into()));
    let h_ty = import_type(&mut arena, &ty);
    let h_repl = import_type(&mut arena, &Type::Nat);
    let nodes_before = arena.types.len();
    let result = substitute_type_handle(&mut arena, h_ty, "a", h_repl).unwrap();
    // Only the root Arrow is rebuilt (TyVar→replacement shares h_repl,
    // the big left subtree shares wholesale): exactly ONE new node.
    assert_eq!(arena.types.len(), nodes_before + 1);
    let materialized = materialize_type(&arena, result).unwrap();
    assert_eq!(materialized, ty.substitute("a", &Type::Nat));
}

/// The eq-arm's share detection must return the ORIGINAL handle when
/// nothing (type or terms) contains the variable (kills the `==`→`!=`
/// mutants in its three-way comparison).
#[test]
fn absent_var_in_eq_type_shares_the_original_handle() {
    let mut arena = Arena::new();
    let ty = Type::eq(
        Type::arrow(Type::Nat, Type::Bool),
        Term::Zero,
        Term::Annot(Box::new(Term::Zero), Type::Nat),
    );
    let h_ty = import_type(&mut arena, &ty);
    let h_repl = import_type(&mut arena, &Type::Nat);
    let (nodes_before, terms_before) = (arena.types.len(), arena.terms.len());
    let result = substitute_type_handle(&mut arena, h_ty, "absent", h_repl).unwrap();
    assert_eq!(
        result, h_ty,
        "unchanged Eq type must return the same handle"
    );
    assert_eq!(arena.types.len(), nodes_before, "no new type nodes");
    assert_eq!(arena.terms.len(), terms_before, "no new term nodes");
}

/// Same for the adt-arm: unchanged type-args AND variants must share
/// (kills the `==`→`!=` mutants in its two-way comparison).
#[test]
fn absent_var_in_adt_type_shares_the_original_handle() {
    let mut arena = Arena::new();
    let ty = Type::adt(
        "Tree",
        vec![Type::Nat],
        vec![("Leaf".into(), Type::Unit), ("Node".into(), Type::Bool)],
    );
    let h_ty = import_type(&mut arena, &ty);
    let h_repl = import_type(&mut arena, &Type::Nat);
    let nodes_before = arena.types.len();
    let result = substitute_type_handle(&mut arena, h_ty, "absent", h_repl).unwrap();
    assert_eq!(
        result, h_ty,
        "unchanged Adt type must return the same handle"
    );
    assert_eq!(arena.types.len(), nodes_before, "no new type nodes");
}

#[test]
fn dangling_handles_return_none() {
    let mut arena = Arena::new();
    let h = import_type(&mut arena, &Type::Nat);
    let invalid = crate::ffi::INVALID_HANDLE;
    assert_eq!(substitute_type_handle(&mut arena, invalid, "a", h), None);
    assert_eq!(substitute_type_handle(&mut arena, h, "a", invalid), None);
}

/// The oracle test above is vacuous over an empty corpus (6.10.26h's sweep
/// scored the helper once it moved here).
#[test]
fn the_substitution_corpus_is_populated() {
    assert!(corpus().len() > 10, "{}", corpus().len());
}
