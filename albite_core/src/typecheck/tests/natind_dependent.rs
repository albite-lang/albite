//! The kernel's dependent `natind` rule (ADR 29.9.26d AC 5, D7).
//!
//! `natind [k. Nat → P] z s n : P[k := n]`, with `z : P[k := zero]` and, for
//! a dependent motive, `s` the literal `λj:Nat. λh:P[k := j]. b` with
//! `b : P[k := succ j]`. Every comparison is structural: the kernel has no
//! conversion, so a case whose proof needs a reduction is refused.

use super::*;
use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `k. Eq<Nat, k, k>` stored as `Nat → Eq<Nat, k, k>`.
fn reflexive_motive() -> Type {
    Type::arrow(Type::Nat, eq_nat(Term::var("k"), Term::var("k")))
}

/// `λj:Nat. λh:hypothesis. proof`
fn step(index: &str, hypothesis: Type, proof: Term) -> Term {
    Term::lambda(index, Type::Nat, Term::lambda("h", hypothesis, proof))
}

fn over_n() -> Context {
    Context::new().with_term("n", Type::Nat)
}

/// 29.9.26d AC5: AC 1's first term, hand-built, has the motive at its target.
#[test]
fn a_dependent_natind_has_the_motive_at_its_target() {
    let term = Term::natind(
        "k",
        reflexive_motive(),
        Term::refl(Type::Nat, Term::Zero),
        step(
            "j",
            eq_nat(Term::var("j"), Term::var("j")),
            Term::refl(Type::Nat, Term::succ(Term::var("j"))),
        ),
        Term::var("n"),
    );
    assert_eq!(
        type_of(&over_n(), &term),
        Ok(eq_nat(Term::var("n"), Term::var("n")))
    );
}

/// 29.9.26d AC5: a step whose proof is its IH proves `P(j)`, not `P(succ j)`.
#[test]
fn a_step_that_proves_its_own_case_is_refused() {
    let hypothesis = eq_nat(Term::var("j"), Term::var("j"));
    let term = Term::natind(
        "k",
        reflexive_motive(),
        Term::refl(Type::Nat, Term::Zero),
        step("j", hypothesis, Term::var("h")),
        Term::var("n"),
    );
    let successor = eq_nat(Term::succ(Term::var("j")), Term::succ(Term::var("j")));
    assert!(matches!(
        type_of(&over_n(), &term),
        Err(TypeError::TypeMismatch { expected, .. }) if expected == successor
    ));
}

/// 29.9.26d AC5: the base must prove the motive at zero.
#[test]
fn a_base_that_is_not_the_zero_instance_is_refused() {
    let term = Term::natind(
        "k",
        reflexive_motive(),
        Term::refl(Type::Nat, Term::succ(Term::Zero)),
        step(
            "j",
            eq_nat(Term::var("j"), Term::var("j")),
            Term::refl(Type::Nat, Term::succ(Term::var("j"))),
        ),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&over_n(), &term),
        Err(TypeError::TypeMismatch { expected, .. }) if expected == eq_nat(Term::Zero, Term::Zero)
    ));
}

/// 29.9.26d AC5: a written IH type other than `P(j)` is refused.
#[test]
fn a_hypothesis_that_is_not_the_index_instance_is_refused() {
    let term = Term::natind(
        "k",
        reflexive_motive(),
        Term::refl(Type::Nat, Term::Zero),
        step(
            "j",
            eq_nat(Term::Zero, Term::Zero),
            Term::refl(Type::Nat, Term::succ(Term::var("j"))),
        ),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&over_n(), &term),
        Err(TypeError::TypeMismatch { expected, .. })
            if expected == eq_nat(Term::var("j"), Term::var("j"))
    ));
}

/// 29.9.26d AC5: a motive over anything but `Nat` is `NatIndMotiveMismatch`.
#[test]
fn a_motive_not_over_nat_is_refused() {
    let term = Term::natind(
        "k",
        Type::arrow(Type::Bool, eq_nat(Term::var("k"), Term::var("k"))),
        Term::refl(Type::Nat, Term::Zero),
        Term::var("s"),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&over_n(), &term),
        Err(TypeError::NatIndMotiveMismatch { .. })
    ));
}

/// 29.9.26d AC5: a dependent motive's step must be a literal lambda pair.
#[test]
fn a_dependent_step_that_is_not_a_lambda_is_refused() {
    let dependent_step_ty = Type::arrow(
        Type::Nat,
        Type::arrow(
            eq_nat(Term::var("k"), Term::var("k")),
            eq_nat(Term::var("k"), Term::var("k")),
        ),
    );
    let ctx = over_n().with_term("s", dependent_step_ty);
    let term = Term::natind(
        "k",
        reflexive_motive(),
        Term::refl(Type::Nat, Term::Zero),
        Term::var("s"),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

/// 29.9.26d AC5: a lambda whose index is not `Nat`, or with one parameter, is refused.
#[test]
fn a_step_with_a_wrong_index_or_arity_is_refused() {
    // The proof does not use `j`, so a `Bool` index still types — the refusal
    // is the shape rule's, not an ill-typed body's.
    let hypothesis = eq_nat(Term::var("j"), Term::var("j"));
    let proof = Term::refl(Type::Nat, Term::Zero);
    let wrong_index = Term::lambda(
        "j",
        Type::Bool,
        Term::lambda("h", hypothesis.clone(), proof.clone()),
    );
    let one_parameter = Term::lambda("j", Type::Nat, proof);
    for succ_case in [wrong_index, one_parameter] {
        let term = Term::natind(
            "k",
            reflexive_motive(),
            Term::refl(Type::Nat, Term::Zero),
            succ_case,
            Term::var("n"),
        );
        assert!(matches!(
            type_of(&over_n(), &term),
            Err(TypeError::TypeMismatch { .. })
        ));
    }
}

/// 29.9.26d AC5: `succ_add`'s step binder `m` is the motive's free `m` (D7's
/// reverse capture), so the step is refused although it would otherwise check.
#[test]
fn a_step_binder_the_motive_mentions_is_refused() {
    let m = || Term::var("m");
    let one = || Term::succ(Term::Zero);
    let body = |at: Term| {
        eq_nat(
            Term::nat_add(Term::nat_add(m(), one()), at.clone()),
            Term::nat_add(Term::nat_add(m(), at), one()),
        )
    };
    let ctx = over_n().with_term("m", Type::Nat);
    let base_instance = body(Term::Zero);
    let term = Term::natind(
        "k",
        Type::arrow(Type::Nat, body(Term::var("k"))),
        Term::annot(Term::Sorry, base_instance),
        step(
            "m",
            body(m()),
            Term::annot(Term::Sorry, body(Term::succ(m()))),
        ),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

/// 29.9.26d AC5: a non-dependent motive keeps `Nat → P → P`, which a step
/// that is not a lambda can have.
#[test]
fn a_non_dependent_motive_keeps_the_simple_step_type() {
    let ctx = over_n().with_term(
        "s",
        Type::arrow(Type::Nat, Type::arrow(Type::Nat, Type::Nat)),
    );
    let term = Term::natind(
        "k",
        Type::arrow(Type::Nat, Type::Nat),
        Term::Zero,
        Term::var("s"),
        Term::var("n"),
    );
    assert_eq!(type_of(&ctx, &term), Ok(Type::Nat));

    let wrong_step = Term::natind(
        "k",
        Type::arrow(Type::Nat, Type::Nat),
        Term::Zero,
        Term::var("n"),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&ctx, &wrong_step),
        Err(TypeError::TypeMismatch { .. })
    ));
}

/// 29.9.26d AC5: a non-`Nat` target is refused.
#[test]
fn a_target_that_is_not_nat_is_refused() {
    let term = Term::natind(
        "k",
        Type::arrow(Type::Nat, Type::Nat),
        Term::Zero,
        Term::lambda("j", Type::Nat, Term::lambda("h", Type::Nat, Term::var("h"))),
        Term::True,
    );
    assert!(matches!(
        type_of(&over_n(), &term),
        Err(TypeError::NotANat { .. })
    ));
}

/// Known-refused, and it stays so: `zero_add`'s step built with the
/// *placeholder* motive `Nat → Prop` and a `sorry` base, run in executable
/// mode. Since ADR 1.10.26e D2 that placeholder is the constant family
/// `Prop` — identity transport — so its `refl` proof is still not `Prop` and
/// the term is refused for the same reason as before (ADR 29.9.26d §2.6).
/// The proof the elaborator now lowers types: see
/// `zero_add_types_in_the_kernel_with_a_dependent_motive` (D5).
#[test]
fn zero_add_step_is_known_refused_by_the_kernel() {
    let zero_plus = |at: Term| Term::nat_add(Term::Zero, at);
    let body = |at: Term| eq_nat(zero_plus(at.clone()), at);
    let plus_one = Term::lambda(
        "x",
        Type::Nat,
        Term::nat_add(Term::var("x"), Term::succ(Term::Zero)),
    );
    let cong_step = Term::subst(
        Type::Nat,
        Type::arrow(Type::Nat, Type::Unit),
        Term::var("h"),
        Term::refl(Type::Nat, Term::app(plus_one, zero_plus(Term::var("j")))),
    );
    let term = Term::natind(
        "k",
        Type::arrow(Type::Nat, body(Term::var("k"))),
        Term::annot(Term::Sorry, body(Term::Zero)),
        step("j", body(Term::var("j")), cong_step),
        Term::var("n"),
    );
    assert!(matches!(
        type_of(&over_n(), &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

/// 1.10.26e D5: `zero_add` as the elaborator now lowers it — a `refl` base
/// and the `Subst` `cong(|x: Nat| x + 1, ih)` lowers to under D3's motive
/// `Π x. Eq Nat (f (0 + j)) (f x)` — admits under `check_proof`, which
/// converts `0 + succ(j) ≡ succ(0 + j)` (ADR 1.10.26c).
#[test]
fn zero_add_types_in_the_kernel_with_a_dependent_motive() {
    let zero_plus = |at: Term| Term::nat_add(Term::Zero, at);
    let body = |at: Term| eq_nat(zero_plus(at.clone()), at);
    let plus_one = Term::lambda(
        "x",
        Type::Nat,
        Term::nat_add(Term::var("x"), Term::succ(Term::Zero)),
    );
    let applied = |at: Term| Term::app(plus_one.clone(), at);
    let cong_motive = Type::pi(
        "x",
        Type::Nat,
        eq_nat(applied(zero_plus(Term::var("j"))), applied(Term::var("x"))),
    );
    let cong_step = Term::subst(
        Type::Nat,
        cong_motive,
        Term::var("h"),
        Term::refl(Type::Nat, applied(zero_plus(Term::var("j")))),
    );
    let term = Term::natind(
        "k",
        Type::arrow(Type::Nat, body(Term::var("k"))),
        Term::refl(Type::Nat, Term::Zero),
        step("j", body(Term::var("j")), cong_step),
        Term::var("n"),
    );
    assert_eq!(check_proof(&over_n(), &term, &body(Term::var("n"))), Ok(()));
}
