//! Core typing rules: Lambda, App, Let, If, `NatRec`, `NatInd`,
//! `TyApp`, Refl, Annot, Absurd, Succ.
//!
//! Sum type rules (Inl, Inr, Case) are in `rules_sum.rs`.

use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

use super::error::TypeError;
use super::rules::{check_type_wf, type_of, types_equal_in};
use super::TypeResult;

// ============================================================================
// Lambda / Application / Let
// ============================================================================

/// Type check application: `Γ ⊢ t₁ t₂ : τ₂` where `Γ ⊢ t₁ : τ₁ → τ₂`
pub(super) fn type_of_app(ctx: &Context, t1: &Term, t2: &Term) -> TypeResult<Type> {
    let ty1 = type_of(ctx, t1)?;
    match ty1 {
        Type::Arrow(param_ty, return_ty) => {
            let arg_ty = type_of(ctx, t2)?;
            if types_equal_in(ctx, &param_ty, &arg_ty) {
                Ok(*return_ty)
            } else {
                Err(TypeError::ArgumentTypeMismatch {
                    expected: *param_ty,
                    got: arg_ty,
                })
            }
        }
        // Π x:τ. B applied to t₂ has type B[x := t₂] (ADR 30.9.26d D5).
        Type::Pi(binder, param_ty, return_ty) => {
            let arg_ty = type_of(ctx, t2)?;
            if types_equal_in(ctx, &param_ty, &arg_ty) {
                Ok(return_ty.substitute_term(&binder, t2))
            } else {
                Err(TypeError::ArgumentTypeMismatch {
                    expected: *param_ty,
                    got: arg_ty,
                })
            }
        }
        _ => Err(TypeError::NotAFunction { got: ty1 }),
    }
}

/// Type check let binding: `Γ ⊢ let x : τ₁ = t₁ in t₂ : τ₂`
pub(super) fn type_of_let(
    ctx: &Context,
    x: &str,
    ty: &Type,
    def: &Term,
    body: &Term,
) -> TypeResult<Type> {
    check_type_wf(ctx, ty)?;
    let def_ty = type_of(ctx, def)?;
    if !types_equal_in(ctx, ty, &def_ty) {
        return Err(TypeError::TypeMismatch {
            expected: ty.clone(),
            got: def_ty,
        });
    }
    let body_ctx = ctx.with_term(x, ty.clone());
    type_of(&body_ctx, body)
}

/// Type check if-then-else: branches must agree on type.
pub(super) fn type_of_if(
    ctx: &Context,
    cond: &Term,
    then_: &Term,
    else_: &Term,
) -> TypeResult<Type> {
    let cond_ty = type_of(ctx, cond)?;
    if cond_ty != Type::Bool {
        return Err(TypeError::ConditionNotBool { got: cond_ty });
    }
    let then_ty = type_of(ctx, then_)?;
    let else_ty = type_of(ctx, else_)?;
    if types_equal_in(ctx, &then_ty, &else_ty) {
        Ok(then_ty)
    } else {
        Err(TypeError::BranchTypeMismatch {
            then_type: then_ty,
            else_type: else_ty,
        })
    }
}

// ============================================================================
// Natural number recursion / induction
// ============================================================================

/// Type check natrec: `natrec [τ] zero_case succ_case n : τ`
pub(super) fn type_of_natrec(
    ctx: &Context,
    ty: &Type,
    zero_case: &Term,
    succ_case: &Term,
    n: &Term,
) -> TypeResult<Type> {
    check_type_wf(ctx, ty)?;

    let zero_ty = type_of(ctx, zero_case)?;
    if !types_equal_in(ctx, &zero_ty, ty) {
        return Err(TypeError::TypeMismatch {
            expected: ty.clone(),
            got: zero_ty,
        });
    }

    let expected_succ_ty = Type::arrow(Type::Nat, Type::arrow(ty.clone(), ty.clone()));
    let succ_ty = type_of(ctx, succ_case)?;
    if !types_equal_in(ctx, &succ_ty, &expected_succ_ty) {
        return Err(TypeError::NatRecSuccTypeMismatch {
            expected: expected_succ_ty,
            got: succ_ty,
        });
    }

    let n_ty = type_of(ctx, n)?;
    if n_ty != Type::Nat {
        return Err(TypeError::NotANat { got: n_ty });
    }

    Ok(ty.clone())
}

/// Type check natind: `natind [k. Nat → P] zero_case succ_case n : P[k := n]`
/// (ADR 29.9.26d D7). `motive` is the binder and the stored `Nat → P`.
///
/// The zero case must have type `P[k := zero]`. For a *dependent* motive (`k`
/// free in `P`) the step must be the literal `λj:Nat. λh:P[k := j]. b` with
/// `b : P[k := succ j]`; otherwise it has type `Nat → P → P`. Every comparison
/// is structural in executable mode, so a case whose proof needs a reduction
/// is refused there (the known-refused control, ADR 29.9.26d §2.6); proof mode
/// converts `Eq` witnesses, δ-free (ADR 1.10.26c).
/// No large elimination (D8): the result is one fixed type with a term
/// substituted into its `Eq` arguments, never a type computed from `n`.
pub(super) fn type_of_natind(
    ctx: &Context,
    motive: (&str, &Type),
    zero_case: &Term,
    succ_case: &Term,
    n: &Term,
) -> TypeResult<Type> {
    let (binder, motive_ty) = motive;
    // The motive's witnesses may name its binder (ADR 1.10.26c D7).
    super::eq_witnesses::check_motive(ctx, binder, motive_ty)?;
    let body = match motive_ty {
        Type::Arrow(from, to) if **from == Type::Nat => to.as_ref(),
        _ => {
            return Err(TypeError::NatIndMotiveMismatch {
                expected: Type::arrow(Type::Nat, Type::TyVar("T".into())),
                got: motive_ty.clone(),
            });
        }
    };

    let zero_instance = body.substitute_term(binder, &Term::Zero);
    expect_case_type(ctx, type_of(ctx, zero_case)?, zero_instance)?;

    if body.free_term_vars().contains(binder) {
        check_dependent_step(ctx, binder, body, succ_case)?;
    } else {
        let expected_succ = Type::arrow(Type::Nat, Type::arrow(body.clone(), body.clone()));
        expect_case_type(ctx, type_of(ctx, succ_case)?, expected_succ)?;
    }

    let n_ty = type_of(ctx, n)?;
    if n_ty != Type::Nat {
        return Err(TypeError::NotANat { got: n_ty });
    }

    Ok(body.substitute_term(binder, n))
}

/// `TypeMismatch` unless `got` is `expected` (structurally, spans aside).
fn expect_case_type(ctx: &Context, got: Type, expected: Type) -> TypeResult<()> {
    if types_equal_in(ctx, &got, &expected) {
        Ok(())
    } else {
        Err(TypeError::TypeMismatch { expected, got })
    }
}

/// A dependent motive's step: `λj:Nat. λh:P[k := j]. b` with
/// `b : P[k := succ j]`, where neither `j` nor `h` is free in `P` other than
/// as `k` — else `P[k := j]` would conflate the step's `j` with an outer one
/// (D4's reverse capture).
fn check_dependent_step(ctx: &Context, binder: &str, body: &Type, step: &Term) -> TypeResult<()> {
    let refuse = |got: Type| TypeError::TypeMismatch {
        expected: Type::arrow(Type::Nat, Type::arrow(body.clone(), body.clone())),
        got,
    };
    let Term::Lambda(j, j_ty, inner) = peel_step(step) else {
        return Err(refuse(type_of(ctx, step)?));
    };
    let Term::Lambda(h, h_ty, proof) = peel_step(inner) else {
        return Err(refuse(type_of(ctx, step)?));
    };
    let mut outer = body.free_term_vars();
    outer.remove(binder);
    if *j_ty != Type::Nat || outer.contains(j) || outer.contains(h) {
        return Err(refuse(type_of(ctx, step)?));
    }
    let hypothesis = body.substitute_term(binder, &Term::var(j.clone()));
    expect_case_type(ctx, h_ty.clone(), hypothesis)?;
    let step_ctx = ctx.with_term(j, Type::Nat).with_term(h, h_ty.clone());
    let successor = body.substitute_term(binder, &Term::succ(Term::var(j.clone())));
    expect_case_type(ctx, type_of(&step_ctx, proof)?, successor)
}

/// See through the wrappers the elaborator puts around a lambda.
fn peel_step(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) | Term::Annot(inner, _) => peel_step(inner),
        _ => term,
    }
}

// ============================================================================
// Polymorphism / Equality / Annotation
// ============================================================================

/// Type check type application: `t [τ'] : τ[α := τ']`
pub(super) fn type_of_tyapp(ctx: &Context, t: &Term, ty: &Type) -> TypeResult<Type> {
    // A predicate is admitted only here, against a `ForallPred` (ADR 6.10.26h D6)
    if matches!(ty, Type::PredAbs(..)) {
        return super::rules_ext::type_of_pred_tyapp(ctx, t, ty);
    }
    check_type_wf(ctx, ty)?;
    let t_ty = type_of(ctx, t)?;
    match t_ty {
        Type::Forall(alpha, body) => Ok(body.substitute(&alpha, ty)),
        Type::ForallPred(..) => Err(TypeError::NotAPredicate { got: ty.clone() }),
        _ => Err(TypeError::NotPolymorphic { got: t_ty }),
    }
}

/// Type check refl: `refl [τ] t : Eq τ t t`
pub(super) fn type_of_refl(ctx: &Context, ty: &Type, t: &Term) -> TypeResult<Type> {
    check_type_wf(ctx, ty)?;
    let t_ty = type_of(ctx, t)?;
    if !types_equal_in(ctx, &t_ty, ty) {
        return Err(TypeError::TypeMismatch {
            expected: ty.clone(),
            got: t_ty,
        });
    }
    Ok(Type::eq(ty.clone(), t.clone(), t.clone()))
}

/// Type check annotation: `(t : τ) : τ`. Special-cases `sorry`.
pub(super) fn type_of_annot(ctx: &Context, t: &Term, ty: &Type) -> TypeResult<Type> {
    // ADR 6.10.26h D5: a predicate prefix is a definition's introduction.
    if ty.has_predicate_prefix() {
        return super::rules_ext::type_of_pred_annot(ctx, t, ty);
    }
    check_type_wf(ctx, ty)?;
    // An authored hole is `Spanned(Sorry)` (ADR 18.9.26g); see through it.
    if matches!(t, Term::Sorry)
        || matches!(t, Term::Spanned(inner, _) if matches!(**inner, Term::Sorry))
    {
        return Ok(ty.clone());
    }
    // ADR 3.10.26o D2: in proof mode a case split may prove `ty` by cases;
    // when it may not, today's rule answers, error and all. Both rules answer
    // exactly `ty` on acceptance, so asking the split first moves no verdict —
    // it only keeps nested splits linear, where today's rule first re-types
    // every subtree once per level (D5: 1365 checks for 63 nodes at depth 6).
    if ctx.is_proof()
        && super::rules_ext::is_case_split(t)
        && super::rules_ext::type_of_dependent_case(ctx, t, ty).is_ok()
    {
        return Ok(ty.clone());
    }
    // ADR 4.10.26h D2: a pair annotated with a `Sigma` is a dependent pair.
    if let (Type::Sigma(x, a, b), Some(pair)) = (ty, super::rules_ext::as_pair(t)) {
        return super::rules_ext::type_of_sigma_pair(ctx, pair, (x, a, b), ty);
    }
    annotated_as(ctx, t, ty)
}

/// Today's annotation rule: `t`'s synthesised type must be `ty`.
fn annotated_as(ctx: &Context, t: &Term, ty: &Type) -> TypeResult<Type> {
    let t_ty = type_of(ctx, t)?;
    if types_equal_in(ctx, &t_ty, ty) {
        Ok(ty.clone())
    } else {
        Err(TypeError::TypeMismatch {
            expected: ty.clone(),
            got: t_ty,
        })
    }
}

// ============================================================================
// Absurd / Succ / Fst / Snd
// ============================================================================

/// Type check absurd elimination: `Γ ⊢ absurd [τ] t : τ` where `Γ ⊢ t : Void`,
/// or where `t`'s type is a refuted `Bool` equation (ADR 7.10.26m D1).
pub(super) fn type_of_absurd(ctx: &Context, ty: &Type, t: &Term) -> TypeResult<Type> {
    check_type_wf(ctx, ty)?;
    let t_ty = type_of(ctx, t)?;
    if t_ty == Type::Void || is_refuted_bool_equation(ctx, &t_ty) {
        Ok(ty.clone())
    } else {
        Err(TypeError::NotVoid { got: t_ty })
    }
}

/// Whether `ty` is `Eq(Bool, True, False)` or its mirror, under the
/// comparison the context uses: conversion of the witnesses in proof mode,
/// α-equality modulo spans outside it (ADR 7.10.26m D1). Both denote the
/// empty set in the standard model, as `Void` does (D3), so this is the one
/// distinct-constructor refutation the kernel trusts; every other one is
/// derived from it by `cong` over a `Bool`-valued discriminator.
fn is_refuted_bool_equation(ctx: &Context, ty: &Type) -> bool {
    let refuted =
        |left: Term, right: Term| types_equal_in(ctx, ty, &Type::eq(Type::Bool, left, right));
    refuted(Term::True, Term::False) || refuted(Term::False, Term::True)
}

/// Type check successor: `Γ ⊢ succ t : Nat` where `Γ ⊢ t : Nat`
pub(super) fn type_of_succ(ctx: &Context, t: &Term) -> TypeResult<Type> {
    let ty = type_of(ctx, t)?;
    if ty == Type::Nat {
        Ok(Type::Nat)
    } else {
        Err(TypeError::NotANat { got: ty })
    }
}

/// Type check first projection: `Γ ⊢ fst t : τ₁` where `Γ ⊢ t : τ₁ × τ₂`
pub(super) fn type_of_fst(ctx: &Context, t: &Term) -> TypeResult<Type> {
    let ty = type_of(ctx, t)?;
    match ty {
        Type::Product(ty1, _) | Type::Sigma(_, ty1, _) => Ok(*ty1),
        _ => Err(TypeError::NotAProduct { got: ty }),
    }
}

/// Type check second projection: `Γ ⊢ snd t : τ₂` where `Γ ⊢ t : τ₁ × τ₂`
pub(super) fn type_of_snd(ctx: &Context, t: &Term) -> TypeResult<Type> {
    let ty = type_of(ctx, t)?;
    match ty {
        Type::Product(_, ty2) => Ok(*ty2),
        // ADR 4.10.26h D2: the body at the witness this pair projects.
        Type::Sigma(x, _, body) => Ok(super::rules_ext::snd_instance(&x, &body, t)),
        _ => Err(TypeError::NotAProduct { got: ty }),
    }
}
