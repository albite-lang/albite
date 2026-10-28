//! ADR 7.10.26m AC 1: the kernel refutes exactly `true ≠ false`. `Absurd`
//! accepts a hypothesis of type `Void` or of a `Bool` equation between `true`
//! and `false` in either orientation (D1); every other distinct-constructor
//! refutation is derived from that one, so each twin below is a fact the
//! kernel must NOT trust directly (D6).

use std::collections::BTreeSet;

use super::*;
use crate::context::{Context, GlobalAdmission};
use crate::conversion::Unfoldings;
use crate::terms::Term;
use crate::typecheck::{DeltaAdmission, DeltaCandidate};
use crate::types::Type;

fn eq_bool(left: Term, right: Term) -> Type {
    Type::eq(Type::Bool, left, right)
}

/// `Absurd(Nat, h)` typed under `ctx` with `h : hypothesis`.
fn absurd_from(ctx: &Context, hypothesis: Type) -> TypeResult<Type> {
    let ctx = ctx.with_term("h", hypothesis);
    type_of(&ctx, &Term::absurd(Type::Nat, Term::var("h")))
}

fn admitted(verdict: &TypeResult<Type>) -> bool {
    *verdict == Ok(Type::Nat)
}

fn refused_not_void(verdict: &TypeResult<Type>) -> bool {
    matches!(verdict, Err(TypeError::NotVoid { .. }))
}

fn proof() -> Context {
    Context::new().for_proof()
}

/// `λx:Nat. natrec [Bool] true (λk. λacc. false) x`: `true` at zero and
/// `false` at every successor, the discriminator D5's lemmas carry.
fn nat_discriminator() -> Term {
    let succ_case = Term::lambda("k", Type::Nat, Term::lambda("acc", Type::Bool, Term::False));
    Term::lambda(
        "x",
        Type::Nat,
        Term::natrec(Type::Bool, Term::True, succ_case, Term::var("x")),
    )
}

/// `cong(disc, h)` for `h : Eq Nat left right`, lowered as the elaborator
/// lowers it: a `subst` whose motive is the discriminator's equation.
fn cong_discriminator(left: &Term, h: Term) -> Term {
    let disc = nat_discriminator();
    let at = |t: Term| Term::app(disc.clone(), t);
    let motive = Type::pi(
        "#x",
        Type::Nat,
        eq_bool(at(left.clone()), at(Term::var("#x"))),
    );
    Term::subst(
        Type::Nat,
        motive,
        h,
        Term::refl(Type::Bool, at(left.clone())),
    )
}

/// `n + 1`, spelled as the elaborator spells it.
fn n_plus_one() -> Term {
    Term::nat_add(Term::var("n"), Term::succ(Term::Zero))
}

/// A context holding `f : Bool` with body `true`, bound as the bootstrap
/// binds it: `total` decides whether the kernel admits the body for δ.
fn with_bool_global(total: bool) -> Context {
    let ty = Type::Bool;
    let body = Term::True;
    let mut admission = DeltaAdmission::new([(
        "f".to_string(),
        DeltaCandidate {
            ty: &ty,
            body: &body,
            total,
            decreasing: None,
        },
    )]);
    let marker = if total {
        GlobalAdmission::Admitted
    } else {
        GlobalAdmission::Opaque
    };
    let base = proof().with_global("f", Type::Bool, marker);
    admission.bind(&base, &BTreeSet::from(["f".to_string()]))
}

/// `natrec [Bool] true (λ_. λa. a) count`: `true` for every `count`, reached
/// only after `count` steps.
fn true_after(count: u64) -> Term {
    let keep = Term::lambda(
        "_",
        Type::Nat,
        Term::lambda("a", Type::Bool, Term::var("a")),
    );
    Term::natrec(Type::Bool, Term::True, keep, Term::NatLit(count))
}

// The verdict helpers' own contracts: each answers no for the other's
// verdict, so a control cannot pass on a refusal nor a twin on an admission.
#[test]
fn the_verdict_helpers_tell_admission_from_refusal() {
    let refusal: TypeResult<Type> = Err(TypeError::NotVoid { got: Type::Nat });
    let other_refusal: TypeResult<Type> = Err(TypeError::MalformedType(Type::Nat));
    assert!(admitted(&Ok(Type::Nat)));
    assert!(!admitted(&Ok(Type::Bool)));
    assert!(!admitted(&refusal));
    assert!(refused_not_void(&refusal));
    assert!(!refused_not_void(&other_refusal));
    assert!(!refused_not_void(&Ok(Type::Nat)));
}

// ==========================================================================
// Admitted — the controls
// ==========================================================================

// 7.10.26m AC1: the premise, in the orientation D1 names first.
#[test]
fn true_equals_false_is_refuted() {
    let verdict = absurd_from(&Context::new(), eq_bool(Term::True, Term::False));
    assert!(admitted(&verdict), "{verdict:?}");
}

// 7.10.26m AC1: and its mirror.
#[test]
fn false_equals_true_is_refuted() {
    let verdict = absurd_from(&Context::new(), eq_bool(Term::False, Term::True));
    assert!(admitted(&verdict), "{verdict:?}");
}

// `succ_ne_zero`'s Core term: `cong(disc, h)` has type
// `Eq Bool (disc (n + 1)) (disc 0)`, which is `false == true` only by
// conversion, so the premise holds in proof mode.
#[test]
fn succ_ne_zero_is_refuted_by_conversion_in_proof_mode() {
    let hypothesis = Type::eq(Type::Nat, n_plus_one(), Term::Zero);
    let ctx = proof().with_term("n", Type::Nat).with_term("h", hypothesis);
    let term = Term::absurd(
        Type::Void,
        cong_discriminator(&n_plus_one(), Term::var("h")),
    );
    assert_eq!(type_of(&ctx, &term), Ok(Type::Void));
}

#[test]
fn a_closed_primitive_reaches_the_premise_in_proof_mode() {
    let one_is_one = Term::nat_eq(Term::NatLit(1), Term::NatLit(1));
    let verdict = absurd_from(&proof(), eq_bool(one_is_one, Term::False));
    assert!(admitted(&verdict), "{verdict:?}");
}

#[test]
fn void_is_still_refuted() {
    assert!(admitted(&absurd_from(&Context::new(), Type::Void)));
    assert!(admitted(&absurd_from(&proof(), Type::Void)));
}

// The control for twin 6: the same body, admitted, is unfolded.
#[test]
fn an_admitted_global_body_reaches_the_premise() {
    let verdict = absurd_from(
        &with_bool_global(true),
        eq_bool(Term::global("f"), Term::False),
    );
    assert!(admitted(&verdict), "{verdict:?}");
}

// The control for twin 7: the same witness, short enough, converts.
#[test]
fn a_short_reduction_reaches_the_premise() {
    let verdict = absurd_from(&proof(), eq_bool(true_after(2), Term::False));
    assert!(admitted(&verdict), "{verdict:?}");
}

// The control for twin 8: in proof mode the `not` converts.
#[test]
fn a_negation_converts_in_proof_mode() {
    let not_false = Term::bool_not(Term::False);
    let verdict = absurd_from(&proof(), eq_bool(not_false, Term::False));
    assert!(admitted(&verdict), "{verdict:?}");
}

// ==========================================================================
// Refused, each `NotVoid` — the twins
// ==========================================================================

#[test]
fn twin_1_true_equals_true_is_not_refuted() {
    let verdict = absurd_from(&proof(), eq_bool(Term::True, Term::True));
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

// `Nat` is derived through a discriminator, never trusted.
#[test]
fn twin_2_zero_equals_one_is_not_trusted() {
    let hypothesis = Type::eq(Type::Nat, Term::Zero, Term::succ(Term::Zero));
    let verdict = absurd_from(&proof(), hypothesis);
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

#[test]
fn twin_3_an_open_bool_is_not_refuted() {
    let ctx = proof().with_term("b", Type::Bool);
    let verdict = absurd_from(&ctx, eq_bool(Term::var("b"), Term::False));
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

#[test]
fn twin_4_a_neutral_application_is_not_refuted() {
    let ctx = proof().with_term("g", Type::arrow(Type::Bool, Type::Bool));
    let neutral = Term::app(Term::var("g"), Term::False);
    let verdict = absurd_from(&ctx, eq_bool(neutral, Term::True));
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

// Only `Bool` is trusted, even between distinct literals.
#[test]
fn twin_5_distinct_int_literals_are_not_trusted() {
    let hypothesis = Type::eq(Type::Int, Term::int_lit(1), Term::int_lit(2));
    let verdict = absurd_from(&proof(), hypothesis);
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

#[test]
fn twin_6_a_partial_body_is_never_unfolded() {
    let ctx = with_bool_global(false);
    assert!(ctx.unfoldings().body("f").is_none());
    let verdict = absurd_from(&ctx, eq_bool(Term::global("f"), Term::False));
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

#[test]
fn twin_7_a_reduction_past_the_budget_is_refused() {
    let verdict = absurd_from(&proof(), eq_bool(true_after(2_000_000), Term::False));
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

// Outside proof mode the comparison is α-equality, so a `not` never converts.
#[test]
fn twin_8_a_negation_is_not_converted_outside_proof_mode() {
    let not_false = Term::bool_not(Term::False);
    let verdict = absurd_from(&Context::new(), eq_bool(not_false, Term::False));
    assert!(refused_not_void(&verdict), "{verdict:?}");
}

// ==========================================================================
// Unchanged — and the message
// ==========================================================================

#[test]
fn an_ill_formed_result_is_refused_before_either_premise() {
    let ctx = Context::new().with_term("h", eq_bool(Term::True, Term::False));
    let term = Term::absurd(Type::TyVar("alpha".into()), Term::var("h"));
    assert_eq!(
        type_of(&ctx, &term),
        Err(TypeError::MalformedType(Type::TyVar("alpha".into())))
    );
}

#[test]
fn the_refusal_names_both_premises() {
    let message = TypeError::NotVoid { got: Type::Nat }.to_string();
    assert_eq!(
        message,
        "Expected Void or an equation between `true` and `false`, got: Nat"
    );
}
