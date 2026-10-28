//! Phase 2 tests: `subst` surface syntax (ADR 21.5.26d, 21.5.26g).
//!
//! Tests cover motive validation (ADR 21.5.26g):
//! - MotiveNotPredicate: non-lambda motive rejected
//! - MotiveDomainMismatch: motive binder type ≠ equality base type
//! - MotiveBodyNotType: motive body fails to elaborate as type

use super::*;
use crate::ast::{Ident, LambdaParam, Motive, Path, Pattern};
use crate::elaborate::tests::elab_err;

/// Build a simple motive `|x: Nat| Nat`.
fn nat_to_nat_motive() -> Motive {
    let param = LambdaParam {
        pattern: Pattern::Var(Ident {
            name: "x".to_string(),
            span: Span::new(6, 7),
        }),
        ty: Some(TypeExpr::Path(Path::simple(Ident {
            name: "Nat".to_string(),
            span: Span::new(9, 12),
        }))),
        span: Span::new(6, 12),
    };
    Motive::Lambda(
        param,
        Box::new(TypeExpr::Path(Path::simple(Ident {
            name: "Nat".to_string(),
            span: Span::new(14, 17),
        }))),
        Span::new(5, 17),
    )
}

/// Helper: check `subst(proof, motive, witness)` against `expected`.
fn check_subst(
    proof: &Expr,
    motive: &Motive,
    witness: &Expr,
    expected: &Type,
) -> Result<Term, ElabErrorKind> {
    let ctx = Box::leak(Box::new(Context::new()));
    let mut elab = Elaborator::new(ctx);
    let subst_expr = Expr::Subst(
        Box::new(proof.clone()),
        motive.clone(),
        Box::new(witness.clone()),
        Span::new(0, 10),
    );
    match elab.check(&subst_expr, expected) {
        Ok(term) => Ok(term),
        Err(e) => Err(e.kind),
    }
}

/// Helper: infer type of `subst(proof, motive, witness)`.
fn infer_subst(
    proof: &Expr,
    motive: &Motive,
    witness: &Expr,
) -> Result<(Term, Type), ElabErrorKind> {
    let ctx = Box::leak(Box::new(Context::new()));
    let mut elab = Elaborator::new(ctx);
    let subst_expr = Expr::Subst(
        Box::new(proof.clone()),
        motive.clone(),
        Box::new(witness.clone()),
        Span::new(0, 10),
    );
    match elab.infer(&subst_expr) {
        Ok(result) => Ok(result),
        Err(e) => Err(e.kind),
    }
}

#[test]
fn test_subst_check_mode_eq_nat_zero() {
    let expected = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let proof = annotated_refl_zero_eq_zero();
    let motive = nat_to_nat_motive();
    let witness = Expr::Refl(Span::new(8, 12));
    let result = check_subst(&proof, &motive, &witness, &expected);
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
}

#[test]
fn test_subst_proof_not_equality_type() {
    let expected = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let proof = Expr::BoolLiteral(true, Span::new(0, 4));
    let motive = nat_to_nat_motive();
    let witness = Expr::Refl(Span::new(8, 12));
    let result = check_subst(&proof, &motive, &witness, &expected);
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), ElabErrorKind::SubstExpectedEquality(_)),
        "expected SubstExpectedEquality"
    );
}

#[test]
fn test_subst_witness_type_mismatch() {
    let expected = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let proof = annotated_refl_zero_eq_zero();
    let motive = nat_to_nat_motive();
    let witness = Expr::BoolLiteral(true, Span::new(8, 12));
    let result = check_subst(&proof, &motive, &witness, &expected);
    assert!(result.is_err(), "expected error, got {:?}", result);
}

#[test]
fn test_subst_infer_mode() {
    // Fails because inferring bare `refl` is not supported
    let proof = Expr::Refl(Span::new(0, 4));
    let motive = nat_to_nat_motive();
    let witness = Expr::Refl(Span::new(8, 12));
    let result = infer_subst(&proof, &motive, &witness);
    assert!(result.is_err());
}

#[test]
fn test_subst_parser_round_trip() {
    let body = extract_body_expr("fn test() { subst(refl, |x: Nat| Nat, refl) }");
    assert!(
        matches!(body, Expr::Subst(_, _, _, _)),
        "expected Expr::Subst, got {:?}",
        body
    );
}

// ─── ADR 21.5.26g: Motive validation ────────────────────────────────

#[test]
fn test_subst_motive_not_predicate() {
    // subst(refl, 42, unit) → MotiveNotPredicate
    let errors = elab_err("fn test() -> Eq<Nat, 0, 0> { subst((refl : 0 == 0), 42, refl) }");
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.kind, ElabErrorKind::MotiveNotPredicate(_))),
        "expected MotiveNotPredicate, got {:?}",
        errors.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

#[test]
fn test_subst_motive_body_not_type() {
    // |x: Nat| x — body `x` is not a valid type
    let errors =
        elab_err("fn test() -> Eq<Nat, 0, 0> { subst((refl : 0 == 0), |x: Nat| x, refl) }");
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.kind, ElabErrorKind::MotiveBodyNotType)),
        "expected MotiveBodyNotType, got {:?}",
        errors.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

#[test]
fn test_subst_motive_domain_mismatch() {
    // Proof is Eq<Nat, 0, 0> but motive binds x: Bool → MotiveDomainMismatch
    let errors =
        elab_err("fn test() -> Eq<Nat, 0, 0> { subst((refl : 0 == 0), |x: Bool| Bool, refl) }");
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.kind, ElabErrorKind::MotiveDomainMismatch { .. })),
        "expected MotiveDomainMismatch, got {:?}",
        errors.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
}

// 1.10.26e AC2: a user `subst` stores `Π y:Nat. P`, keeping the binder its
// body mentions under the name it was written with. The result is the motive
// at the RIGHT endpoint, `Eq b b`: once declared `Eq a a`, which the kernel
// refuses now that it checks a proof-carrying `fn` (ADR 13.10.26t).
#[test]
fn subst_motive_keeps_its_binder() {
    let defs = crate::elaborate::tests::elab_ok(
        "fn u(a: Nat, b: Nat, h: Eq<Nat, a, b>, w: Eq<Nat, a, a>) -> Eq<Nat, b, b> \
         { subst(h, |y: Nat| Eq<Nat, y, y>, w) }",
    );
    let motive = super::combinators::first_subst_motive(&defs, "u");
    assert_eq!(
        motive,
        Type::pi(
            "y",
            Type::Nat,
            Type::eq(Type::Nat, Term::var("y"), Term::var("y"))
        )
    );
}

// 6.10.26h P0.1: a dependent motive whose right instance is the expected type
// checks its witness at the LEFT endpoint — Leibniz transport, the kernel's
// rule — where the old rule checked it against the result and refused.
#[test]
fn subst_transports_a_witness_from_the_left_endpoint() {
    crate::elaborate::tests::elab_ok(
        "fn t(a: Nat, b: Nat, h: Eq<Nat, a, b>, w: Eq<Nat, a, 0>) -> Eq<Nat, b, 0> \
         { subst(h, |y: Nat| Eq<Nat, y, 0>, w) }",
    );
    // The witness at the right endpoint is no longer what transport takes.
    let errors = elab_err(
        "fn t(a: Nat, b: Nat, h: Eq<Nat, a, b>, w: Eq<Nat, b, 0>) -> Eq<Nat, b, 0> \
         { subst(h, |y: Nat| Eq<Nat, y, 0>, w) }",
    );
    assert_eq!(errors[0].kind.code(), "E0010");
}
