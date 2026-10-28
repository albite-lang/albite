//! ADR 1.10.26e AC 1: `subst [τ] [Π x:τ. P] (eq : Eq τ a b) proof` checks
//! `proof : P[x := a]` and types as `P[x := b]`; an `Arrow` motive is a
//! constant family, so the placeholder `τ → Prop` keeps its old rule.

use super::*;

/// `Π x:Nat. body`
fn family(body: Type) -> Type {
    Type::pi("x", Type::Nat, body)
}

fn transport(motive: Type, eq: Term, proof: Term) -> Term {
    Term::subst(Type::Nat, motive, eq, proof)
}

/// `a, b : Nat` and `h : Eq Nat a b`.
fn over_a_b() -> Context {
    Context::new()
        .with_term("a", Type::Nat)
        .with_term("b", Type::Nat)
        .with_term("h", eq_nat(var("a"), var("b")))
}

// 1.10.26e AC1: `sym`'s shape — `Π x. Eq x a` from `refl a` to `Eq b a`.
#[test]
fn a_dependent_motive_types_as_its_instance_at_the_right_endpoint() {
    let term = transport(
        family(eq_nat(var("x"), var("a"))),
        var("h"),
        refl_nat(var("a")),
    );
    let flipped = eq_nat(var("b"), var("a"));
    assert_eq!(type_of(&over_a_b(), &term), Ok(flipped.clone()));
    assert_eq!(type_of_proof(&over_a_b(), &term), Ok(flipped));
}

// 1.10.26e AC1: the proof meets `P[n]` only by conversion, so proof mode
// admits and executable mode refuses (1.10.26c's D1 asymmetry).
#[test]
fn the_proof_is_checked_against_the_left_instance_up_to_conversion() {
    let ctx = Context::new()
        .with_term("n", Type::Nat)
        .with_term("m", Type::Nat)
        .with_term("h", eq_nat(var("n"), var("m")));
    let term = transport(
        family(eq_nat(plus(var("x"), Term::Zero), var("x"))),
        var("h"),
        refl_nat(plus(var("n"), Term::Zero)),
    );
    assert_eq!(
        type_of_proof(&ctx, &term),
        Ok(eq_nat(plus(var("m"), Term::Zero), var("m")))
    );
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

// 1.10.26e AC1 (D2): an `Arrow` motive is identity transport.
#[test]
fn a_constant_motive_returns_its_body() {
    let body = eq_nat(var("a"), var("a"));
    let term = transport(
        Type::arrow(Type::Nat, body.clone()),
        var("h"),
        refl_nat(var("a")),
    );
    assert_eq!(type_of(&over_a_b(), &term), Ok(body.clone()));
    assert_eq!(type_of_proof(&over_a_b(), &term), Ok(body));
}

// 1.10.26e AC1 (D2): a constant motive types exactly as before. Re-pointed
// from `Nat → Prop` to `Nat → Unit` when the `Prop` type went (ADR 4.10.26i D9).
#[test]
fn a_constant_motive_returns_its_codomain() {
    let ctx = over_a_b().with_term("p", Type::Unit);
    let placeholder = Type::arrow(Type::Nat, Type::Unit);
    let term = transport(placeholder.clone(), var("h"), var("p"));
    assert_eq!(type_of(&ctx, &term), Ok(Type::Unit));
    // ...and it still demands its codomain, as the old rule did.
    let refused = transport(placeholder, var("h"), refl_nat(var("a")));
    assert_eq!(
        type_of(&ctx, &refused),
        Err(TypeError::TypeMismatch {
            expected: Type::Unit,
            got: eq_nat(var("a"), var("a")),
        })
    );
}

// 1.10.26e AC1 must-fail: a proof of `P[b]` where `P[a]` is due.
#[test]
fn a_proof_of_the_wrong_instance_is_refused() {
    let term = transport(
        family(eq_nat(var("x"), var("x"))),
        var("h"),
        refl_nat(var("b")),
    );
    for verdict in [
        type_of(&over_a_b(), &term),
        type_of_proof(&over_a_b(), &term),
    ] {
        assert_eq!(
            verdict,
            Err(TypeError::TypeMismatch {
                expected: eq_nat(var("a"), var("a")),
                got: eq_nat(var("b"), var("b")),
            })
        );
    }
}

// 1.10.26e AC1 must-fail: `refl 0` transports `P[0]` to `P[0]`, never `P[1]`.
#[test]
fn a_false_equality_transports_nothing() {
    let one = Term::succ(Term::Zero);
    let term = transport(
        family(eq_nat(var("x"), Term::Zero)),
        refl_nat(Term::Zero),
        refl_nat(Term::Zero),
    );
    let ctx = Context::new();
    assert_eq!(
        type_of_proof(&ctx, &term),
        Ok(eq_nat(Term::Zero, Term::Zero))
    );
    assert!(matches!(
        check_proof(&ctx, &term, &eq_nat(one, Term::Zero)),
        Err(TypeError::TypeMismatch { .. })
    ));
}

// 1.10.26e AC1 must-fail: the motive's domain must be the equality's base.
#[test]
fn a_motive_over_the_wrong_domain_is_refused() {
    let wrong_domain = Type::pi("x", Type::Bool, eq_nat(var("a"), var("a")));
    let term = transport(wrong_domain, var("h"), refl_nat(var("a")));
    assert!(matches!(
        type_of(&over_a_b(), &term),
        Err(TypeError::MotiveNotFunction { .. })
    ));
}

// 1.10.26e AC1 must-fail: a motive that is no family at all.
#[test]
fn a_non_function_motive_is_refused() {
    let term = transport(eq_nat(var("a"), var("a")), var("h"), refl_nat(var("a")));
    assert!(matches!(
        type_of(&over_a_b(), &term),
        Err(TypeError::MotiveNotFunction { .. })
    ));
}

// 1.10.26e AC1 must-fail: instantiating `Π x. Π y. Eq x y` at `b = y` renames
// the inner `y`, so the endpoint and the inner binder stay apart.
#[test]
fn the_motive_binder_does_not_capture() {
    let inner = Type::pi("y", Type::Nat, eq_nat(var("x"), var("y")));
    let ctx = Context::new()
        .with_term("a", Type::Nat)
        .with_term("y", Type::Nat)
        .with_term("h", eq_nat(var("a"), var("y")))
        .with_term("p", Type::pi("y", Type::Nat, eq_nat(var("a"), var("y"))));
    let term = transport(family(inner), var("h"), var("p"));
    let Ok(Type::Pi(binder, domain, body)) = type_of(&ctx, &term) else {
        panic!("the instance is a Pi");
    };
    assert_ne!(binder, "y");
    assert_eq!(*domain, Type::Nat);
    assert_eq!(*body, eq_nat(var("y"), var(&binder)));
}

// 1.10.26e AC1 (D2): a pre-ADR cached `Arrow(τ, body)` whose body names its
// lost binder — refused in proof mode, identity in executable mode.
#[test]
fn a_cached_arrow_motive_is_identity_transport() {
    let body = eq_nat(var("x"), var("x"));
    let ctx = over_a_b().with_term("p", body.clone());
    let term = transport(Type::arrow(Type::Nat, body.clone()), var("h"), var("p"));
    assert_eq!(type_of(&ctx, &term), Ok(body));
    assert!(matches!(
        type_of_proof(&ctx, &term),
        Err(TypeError::IllTypedEqWitness { .. })
    ));
}
