//! Adversarial tests for the type/term stratification (ADR 28.9.26c § 2.2).
//!
//! The core has no sort, universe or level: types and terms are separate
//! layers, `∀` ranges over types with one implicit kind, and `Prop` is a sort
//! no term has as its type (ADR 4.10.26i D9). These tests pin that separation, pin impredicative
//! instantiation as a deliberate positive control. The soundness argument
//! they support is ADR 28.9.26c § 6.1.
//!
//! The three known-admitted controls that stood here — an unrestricted `fix`,
//! a negative `μ` and `return` at `Void` — moved to `proof_gates.rs` and
//! flipped to refusals when the kernel gained its own gates (ADR 29.9.26e
//! § 2.4).

use super::*;
use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

// ==========================================================================
// Stratification premise: each of these must be rejected
// ==========================================================================

#[test]
fn term_variable_cannot_classify_a_term() {
    // λp:Nat. λh:p. h — `p` is a term variable, so it never enters the
    // type-variable set and `p` in type position is malformed.
    let ctx = Context::new();
    let term = Term::lambda(
        "p",
        Type::Nat,
        Term::lambda("h", Type::TyVar("p".into()), Term::var("h")),
    );
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::MalformedType(_))
    ));
}

#[test]
fn type_variable_is_not_a_term() {
    // Λa. a — a type variable in term position is an unbound term variable.
    let ctx = Context::new();
    let term = Term::ty_abs("a", Term::var("a"));
    assert_eq!(
        type_of(&ctx, &term),
        Err(TypeError::UnboundVariable("a".into()))
    );
}

#[test]
fn unbound_type_name_is_malformed() {
    // λx:Type. x — there is no type of types; `Type` is just an unbound name.
    let ctx = Context::new();
    let term = Term::lambda("x", Type::TyVar("Type".into()), Term::var("x"));
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::MalformedType(_))
    ));
}

/// No large elimination (ADR 29.9.26d D8): a `natind` motive is one fixed
/// type body over a `Nat` binder, instantiated by substituting a TERM into its
/// `Eq` arguments — never a type computed from the target by recursion. So the
/// domain must be `Nat`, and a constant family (`k. Nat`) is judged like any
/// other: its cases must have the body's type. Renamed from
/// `natind_motive_must_end_in_prop`, whose `Prop`-only rule this replaced.
#[test]
fn natind_motive_is_a_fixed_family_over_nat() {
    let ctx = Context::new().with_term("p", Type::Unit);
    let succ_case = Term::lambda(
        "n",
        Type::Nat,
        Term::lambda("h", Type::Unit, Term::var("h")),
    );
    let not_over_nat = Term::natind(
        "k",
        Type::arrow(Type::Bool, Type::Unit),
        Term::var("p"),
        succ_case.clone(),
        Term::Zero,
    );
    assert!(matches!(
        type_of(&ctx, &not_over_nat),
        Err(TypeError::NatIndMotiveMismatch { .. })
    ));

    // The constant family `k. Nat` is refused because the zero case has type
    // `Unit`, not `Nat` — the case is judged against the family.
    let constant_nat = Term::natind(
        "k",
        Type::arrow(Type::Nat, Type::Nat),
        Term::var("p"),
        succ_case.clone(),
        Term::Zero,
    );
    assert!(matches!(
        type_of(&ctx, &constant_nat),
        Err(TypeError::TypeMismatch { .. })
    ));

    // The accepted shape returns the body instantiated at the target —
    // here `Unit`, which mentions no `k`.
    let good = Term::natind(
        "k",
        Type::arrow(Type::Nat, Type::Unit),
        Term::var("p"),
        succ_case,
        Term::Zero,
    );
    assert_eq!(type_of(&ctx, &good), Ok(Type::Unit));
}

/// No large elimination (ADR 1.10.26e D2): a `subst` motive is one fixed type
/// family over its base, so a constant family (`Nat → Nat`) is identity
/// transport — its proof has the body's type and so does the result — while a
/// motive over the wrong domain is still refused. Renamed from
/// `subst_motive_must_end_in_prop`, whose `Prop`-only rule this replaced.
#[test]
fn subst_motive_is_a_fixed_family_over_its_base() {
    let ctx = Context::new().with_term("n", Type::Nat);
    let transport = |motive: Type| {
        Term::subst(
            Type::Nat,
            motive,
            Term::refl(Type::Nat, Term::Zero),
            Term::var("n"),
        )
    };
    assert_eq!(
        type_of(&ctx, &transport(Type::arrow(Type::Nat, Type::Nat))),
        Ok(Type::Nat)
    );
    assert!(matches!(
        type_of(&ctx, &transport(Type::arrow(Type::Bool, Type::Nat))),
        Err(TypeError::MotiveNotFunction { .. })
    ));
}

// ==========================================================================
// Positive control: impredicativity is admitted on purpose
// ==========================================================================

#[test]
fn impredicative_instantiation_is_admitted() {
    // (Λa. λx:a. x) [∀b. b → b] : (∀b. b → b) → (∀b. b → b)
    // System F, as Girard proved it normalizing. A predicativity change
    // must turn this red as a visible decision, not a side effect.
    let ctx = Context::new();
    let poly_id_ty = Type::forall(
        "b",
        Type::arrow(Type::TyVar("b".into()), Type::TyVar("b".into())),
    );
    let id = Term::ty_abs(
        "a",
        Term::lambda("x", Type::TyVar("a".into()), Term::var("x")),
    );
    let term = Term::ty_app(id, poly_id_ty.clone());
    assert_eq!(
        type_of(&ctx, &term),
        Ok(Type::arrow(poly_id_ty.clone(), poly_id_ty))
    );
}

// ==========================================================================
// Tripwire: every `Type` variant is classified
// ==========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stratum {
    /// A primitive with no type arguments.
    Ground,
    /// A variant that introduces a *type* variable.
    Binder,
    /// Everything else (including a variable reference, `App` and `Error`).
    Former,
}

/// Adding a variant here — above all a sort, universe or level — invalidates
/// ADR 28.9.26c § 6.1's soundness note; re-read it before classifying.
///
/// Deliberately wildcard-free: a new `Type` variant fails to compile until it
/// is placed in a bucket by hand.
fn stratum(ty: &Type) -> Stratum {
    match ty {
        Type::Bool | Type::Nat | Type::Int | Type::Unit | Type::Void | Type::String => {
            Stratum::Ground
        }
        Type::Forall(..) | Type::Mu(..) => Stratum::Binder,
        Type::Arrow(..)
        | Type::Product(..)
        | Type::Sum(..)
        | Type::TyVar(_)
        | Type::Eq(..)
        | Type::Ptr(_)
        | Type::Ref(_)
        | Type::App(..)
        | Type::Adt(..)
        | Type::Error => Stratum::Former,
        // Binds a TERM variable that occurs only inside `Eq` arguments, so it
        // is a former, not a binder (ADR 30.9.26d D5/D7, 28.9.26c § 6.1 note).
        Type::Pi(..) => Stratum::Former,
        // The same for a dependent pair (ADR 4.10.26h D9, § 6.1's Σ clause).
        Type::Sigma(..) => Stratum::Former,
        // A predicate applied binds nothing, as `TyVar` and `App` bind
        // nothing (ADR 6.10.26h D8).
        Type::PredApp(..) => Stratum::Former,
        // Binds TERM variables only, which occur only in `PredApp` and `Eq`
        // arguments — `Pi`'s and `Sigma`'s reason (ADR 6.10.26h D8).
        Type::PredAbs(..) => Stratum::Former,
        // Binds a type-level variable, as `Forall` and `Mu` do; prenex and
        // predicative by the kernel's `check_signature_wf` (ADR 6.10.26h D8).
        Type::ForallPred(..) => Stratum::Binder,
    }
}

/// One sample value per `Type` variant, in declaration order.
fn one_sample_per_type_variant() -> Vec<Type> {
    let alpha = || Type::TyVar("a".into());
    vec![
        Type::Bool,
        Type::Nat,
        Type::Int,
        Type::Unit,
        Type::Void,
        Type::String,
        Type::arrow(Type::Nat, Type::Nat),
        Type::product(Type::Nat, Type::Nat),
        Type::sum(Type::Nat, Type::Nat),
        alpha(),
        Type::forall("a", alpha()),
        Type::eq(Type::Nat, Term::Zero, Term::Zero),
        Type::mu("a", alpha()),
        Type::ptr(Type::Nat),
        Type::Ref(Box::new(Type::Nat)),
        Type::App("List".into(), vec![Type::Nat]),
        Type::Adt("Pick".into(), vec![], vec![("PA".into(), Type::Unit)]),
        Type::Error,
        Type::pi(
            "n",
            Type::Nat,
            Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
        ),
        Type::sigma(
            "k",
            Type::Nat,
            Type::eq(Type::Nat, Term::var("k"), Term::var("k")),
        ),
        Type::pred_app("P", vec![Term::Zero]),
        Type::forall_pred("P", vec![Type::Nat], Type::pred_app("P", vec![Term::Zero])),
        Type::pred_abs(
            vec![("n".into(), Type::Nat)],
            Type::eq(Type::Nat, Term::var("n"), Term::Zero),
        ),
    ]
}

// 6.10.26h AC1: three predicate forms flip the tripwire, 20/6/12/2 → 23/6/14/3.
#[test]
fn type_variants_are_classified() {
    let samples = one_sample_per_type_variant();
    // `Prop` left the ground types when it became a sort (ADR 4.10.26i D9).
    assert_eq!(samples.len(), 23);
    let count = |want: Stratum| samples.iter().filter(|t| stratum(t) == want).count();
    assert_eq!(count(Stratum::Ground), 6);
    assert_eq!(count(Stratum::Former), 14);
    assert_eq!(count(Stratum::Binder), 3);
    let stratum_of = |i: usize| stratum(&samples[i]);
    assert_eq!(stratum_of(20), Stratum::Former, "PredApp");
    assert_eq!(stratum_of(21), Stratum::Binder, "ForallPred");
    assert_eq!(stratum_of(22), Stratum::Former, "PredAbs");
}
