//! The kernel's predicate binders (ADR 6.10.26h AC 1): the known-good
//! control and instantiation, `strong_induction`'s statement, the hereditary
//! substitution under a capturing binder, and D9's twelve twins (`twins.rs`).
//! Everything is built directly in Core, with no surface syntax.

use super::*;
use crate::context::{Context, GlobalAdmission};
use crate::terms::Term;
use crate::types::Type;

use crate::typecheck::rules_ext::check_signature_wf;

mod twins;

fn pred(var: &str, args: Vec<Term>) -> Type {
    Type::pred_app(var, args)
}

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `Π a b : Nat. a == b → P<a> → P<b>` under `P`.
fn transport_matrix() -> Type {
    let (a, b) = (|| Term::var("a"), || Term::var("b"));
    Type::pi_or_arrow(
        "a",
        Type::Nat,
        Type::pi_or_arrow(
            "b",
            Type::Nat,
            Type::arrow(
                eq_nat(a(), b()),
                Type::arrow(pred("P", vec![a()]), pred("P", vec![b()])),
            ),
        ),
    )
}

/// `pred_transport : ∀P : Nat → Prop. Π a b. a == b → P<a> → P<b>`.
fn transport_statement() -> Type {
    Type::forall_pred("P", vec![Type::Nat], transport_matrix())
}

/// `Annot(ΛP. λa b e h. subst(e, λx. P<x>, h), transport_statement())`.
fn transport_proof() -> Term {
    let motive = Type::pi_or_arrow("x", Type::Nat, pred("P", vec![Term::var("x")]));
    let body = Term::lambda(
        "a",
        Type::Nat,
        Term::lambda(
            "b",
            Type::Nat,
            Term::lambda(
                "e",
                eq_nat(Term::var("a"), Term::var("b")),
                Term::lambda(
                    "h",
                    pred("P", vec![Term::var("a")]),
                    Term::subst(Type::Nat, motive, Term::var("e"), Term::var("h")),
                ),
            ),
        ),
    );
    Term::annot(Term::ty_abs("P", body), transport_statement())
}

/// `λ(n: Nat). n == 0`, the family `IsZero` as a predicate.
fn is_zero() -> Type {
    Type::pred_abs(
        vec![("n".into(), Type::Nat)],
        eq_nat(Term::var("n"), Term::Zero),
    )
}

fn with_transport() -> Context {
    Context::new().with_global(
        "pred_transport",
        transport_statement(),
        GlobalAdmission::Admitted,
    )
}

// 6.10.26h AC1: the control — a well-formed introduction is admitted, in the
// proof judgment, against its own statement.
#[test]
fn a_predicate_generic_definition_is_admitted() {
    assert_eq!(
        check_proof(&Context::new(), &transport_proof(), &transport_statement()),
        Ok(())
    );
    assert_eq!(
        type_of(&Context::new(), &transport_proof()),
        Ok(transport_statement())
    );
}

// 6.10.26h AC1: instantiation at a predicate is hereditary — the result
// holds no `PredApp` and no `PredAbs`, only the predicate's body.
#[test]
fn instantiation_at_a_predicate_substitutes_its_body() {
    let instance = Term::ty_app(Term::global("pred_transport"), is_zero());
    let expected = Type::pi_or_arrow(
        "a",
        Type::Nat,
        Type::pi_or_arrow(
            "b",
            Type::Nat,
            Type::arrow(
                eq_nat(Term::var("a"), Term::var("b")),
                Type::arrow(
                    eq_nat(Term::var("a"), Term::Zero),
                    eq_nat(Term::var("b"), Term::Zero),
                ),
            ),
        ),
    );
    let got = type_of(&with_transport(), &instance).expect("instantiates");
    assert_eq!(got, expected);
    assert!(!got.mentions_predicate_form());
}

// 6.10.26h AC1/AC3: `zero_transport` — the instance, applied, proves `0 == 0`
// in the proof judgment, so the kernel ran the instantiation.
#[test]
fn a_citation_at_a_predicate_proves_its_instance() {
    let refl_zero = || Term::refl(Type::Nat, Term::Zero);
    let instance = Term::ty_app(Term::global("pred_transport"), is_zero());
    let proof = [Term::Zero, Term::Zero, refl_zero(), refl_zero()]
        .into_iter()
        .fold(instance, Term::app);
    assert_eq!(
        check_proof(&with_transport(), &proof, &eq_nat(Term::Zero, Term::Zero)),
        Ok(())
    );
}

/// `Lt<m, n>` expanded: `Σ k:Nat. (m + 1) + k == n`.
fn lt(m: Term, n: Term) -> Type {
    Type::sigma(
        "k",
        Type::Nat,
        eq_nat(
            Term::nat_add(Term::nat_add(m, Term::NatLit(1)), Term::var("k")),
            n,
        ),
    )
}

// 6.10.26h AC3: `strong_induction`'s Core statement is a well-formed signature.
#[test]
fn the_strong_induction_statement_is_well_formed() {
    let (m, n) = (|| Term::var("m"), || Term::var("n"));
    let below = Type::pi_or_arrow(
        "m",
        Type::Nat,
        Type::arrow(lt(m(), n()), pred("P", vec![m()])),
    );
    let step = Type::pi_or_arrow("n", Type::Nat, Type::arrow(below, pred("P", vec![n()])));
    let statement = Type::forall_pred(
        "P",
        vec![Type::Nat],
        Type::arrow(
            step,
            Type::pi_or_arrow("n", Type::Nat, pred("P", vec![n()])),
        ),
    );
    assert_eq!(check_signature_wf(&Context::new(), &statement), Ok(()));
    assert_eq!(
        check_signature_wf(&Context::new().for_proof(), &statement),
        Ok(())
    );
}

// 6.10.26h AC1 (D4): substituting under a `Sigma` that would capture a free
// variable of the argument renames the binder; the argument stays free.
#[test]
fn hereditary_substitution_renames_a_capturing_binder() {
    // P := λn. Σ k:Nat. n + k == n, applied to the free variable `k`.
    let predicate = Type::pred_abs(
        vec![("n".into(), Type::Nat)],
        Type::sigma(
            "k",
            Type::Nat,
            eq_nat(
                Term::nat_add(Term::var("n"), Term::var("k")),
                Term::var("n"),
            ),
        ),
    );
    let got = pred("P", vec![Term::var("k")]).substitute("P", &predicate);
    let Type::Sigma(binder, _, body) = &got else {
        panic!("expected a Sigma, got {got}");
    };
    assert_ne!(binder, "k", "the binder captured the argument: {got}");
    assert_eq!(
        **body,
        eq_nat(
            Term::nat_add(Term::var("k"), Term::var(binder.clone())),
            Term::var("k")
        )
    );
}

// 6.10.26h D4: simultaneous — the second argument is never re-substituted by
// the first parameter's replacement.
#[test]
fn hereditary_substitution_is_simultaneous() {
    let swap = Type::pred_abs(
        vec![("x".into(), Type::Nat), ("y".into(), Type::Nat)],
        eq_nat(Term::var("y"), Term::var("x")),
    );
    let got = pred("R", vec![Term::var("y"), Term::var("x")]).substitute("R", &swap);
    assert_eq!(got, eq_nat(Term::var("x"), Term::var("y")));
}

// 6.10.26h D5: the abstraction may name its binder differently from the
// annotation; the body is read under the abstraction's own name.
#[test]
fn an_introduction_may_rename_its_binder() {
    let renamed = Term::annot(
        Term::ty_abs(
            "Q",
            Term::lambda("h", pred("Q", vec![Term::Zero]), Term::var("h")),
        ),
        Type::forall_pred(
            "P",
            vec![Type::Nat],
            Type::arrow(pred("P", vec![Term::Zero]), pred("P", vec![Term::Zero])),
        ),
    );
    assert!(type_of(&Context::new(), &renamed).is_ok());
}

// 6.10.26h D5: an authored hole under a predicate prefix answers the
// annotation after the signature is checked, as `sorry` does elsewhere.
#[test]
fn a_hole_under_a_predicate_prefix_answers_its_annotation() {
    let hole = Term::annot(Term::Sorry, transport_statement());
    assert_eq!(type_of(&Context::new(), &hole), Ok(transport_statement()));
    let bad = Term::annot(Term::Sorry, Type::forall_pred("P", vec![], Type::Unit));
    assert!(type_of(&Context::new(), &bad).is_err());
}

// 6.10.26h D4: a predicate form is well formed exactly when its domains and
// body are; a `PredApp`'s variable is resolved by the kernel's walk instead.
#[test]
fn a_predicate_form_is_well_formed_by_its_children() {
    use std::collections::HashSet;
    let none = HashSet::new();
    let unbound = || Type::TyVar("a".into());
    assert!(Type::forall_pred("P", vec![Type::Nat], pred("P", vec![])).is_well_formed(&none));
    assert!(!Type::forall_pred("P", vec![unbound()], Type::Unit).is_well_formed(&none));
    assert!(!Type::pred_abs(vec![("n".into(), Type::Nat)], unbound()).is_well_formed(&none));
    assert!(pred("P", vec![Term::Zero]).is_well_formed(&none));
}
