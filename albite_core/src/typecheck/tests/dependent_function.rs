//! The kernel's dependent function rules (ADR 30.9.26d AC 4, D5).
//!
//! A λ whose body type mentions its binder has a `Pi` type, and applying it
//! substitutes the argument into that body type. A λ whose body type does
//! not mention its binder keeps today's `Arrow`.

use super::*;
use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

fn three() -> Term {
    Term::succ(Term::succ(Term::succ(Term::Zero)))
}

/// `λn:Nat. refl [Nat] n : Π n:Nat. Eq<Nat, n, n>`
fn reflexive_at() -> Term {
    Term::lambda("n", Type::Nat, Term::refl(Type::Nat, Term::var("n")))
}

fn reflexive_at_type() -> Type {
    Type::pi("n", Type::Nat, eq_nat(Term::var("n"), Term::var("n")))
}

// 30.9.26d AC4: a λ whose body type mentions its binder has a `Pi` type.
#[test]
fn a_dependent_lambda_has_a_pi_type() {
    assert_eq!(
        type_of(&Context::new(), &reflexive_at()),
        Ok(reflexive_at_type())
    );
}

// 30.9.26d AC4: its application has the substituted `Eq` type.
#[test]
fn applying_a_pi_substitutes_the_argument() {
    let term = Term::app(reflexive_at(), three());
    assert_eq!(
        type_of(&Context::new(), &term),
        Ok(eq_nat(three(), three()))
    );
}

// 30.9.26d AC4: an argument of the wrong type is refused.
#[test]
fn a_pi_refuses_an_argument_of_the_wrong_type() {
    let term = Term::app(reflexive_at(), Term::True);
    assert_eq!(
        type_of(&Context::new(), &term),
        Err(TypeError::ArgumentTypeMismatch {
            expected: Type::Nat,
            got: Type::Bool,
        })
    );
}

// 30.9.26d AC4: a non-dependent λ still has today's `Arrow` type.
#[test]
fn a_non_dependent_lambda_keeps_its_arrow() {
    let term = Term::lambda("n", Type::Nat, Term::refl(Type::Nat, Term::Zero));
    assert_eq!(
        type_of(&Context::new(), &term),
        Ok(Type::arrow(Type::Nat, eq_nat(Term::Zero, Term::Zero)))
    );
}

// 30.9.26d AC4: the same judgements hold in proof mode.
#[test]
fn the_dependent_rules_hold_in_proof_mode() {
    let ctx = Context::new();
    assert_eq!(
        type_of_proof(&ctx, &reflexive_at()),
        Ok(reflexive_at_type())
    );
    assert_eq!(
        type_of_proof(&ctx, &Term::app(reflexive_at(), three())),
        Ok(eq_nat(three(), three()))
    );
}
