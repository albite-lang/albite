//! Typing rules for equality elimination, fixpoints, and recursive types.
//!
//! `fold`/`unfold` run the strict-positivity gate in both modes; `fix` runs
//! the termination gate in proof mode only (ADR 29.9.26e D3, D6).

use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

use crate::typecheck::error::TypeError;
use crate::typecheck::gates::{fix_admission, mu_positivity};
use crate::typecheck::rules::{check_type_wf, type_of, types_equal_in};
use crate::typecheck::TypeResult;

/// Type check substitution (equality elimination), ADR 1.10.26e D2:
/// `subst [τ] [Π x:τ. P] (eq : Eq τ a b) (proof : P[x := a]) : P[x := b]`.
///
/// The motive is a type family, never anyone's type: a `Pi` carries its
/// binder (D1), and an `Arrow` `τ → P` is the constant family `P` — identity
/// transport. The legacy placeholder `τ → Prop` therefore keeps its old rule
/// exactly: the proof must be `Prop`, and so is the result. `P` may be any
/// type, because it is one fixed type with a term substituted into its `Eq`
/// arguments, never a type computed from `b` — no large elimination (ADR
/// 29.9.26d D8).
pub(in crate::typecheck) fn type_of_subst(
    ctx: &Context,
    ty: &Type,
    motive: &Type,
    eq_proof: &Term,
    proof: &Term,
) -> TypeResult<Type> {
    check_type_wf(ctx, ty)?;
    // In proof mode this types the motive's `Eq` witnesses with its `Pi`
    // binder in scope (ADR 1.10.26c D7), the discipline a `natind` motive gets.
    check_type_wf(ctx, motive)?;

    let (binder, family) = match motive {
        Type::Pi(binder, from, to) if types_equal_in(ctx, from, ty) => (Some(binder), &**to),
        Type::Arrow(from, to) if types_equal_in(ctx, from, ty) => (None, &**to),
        _ => {
            return Err(TypeError::MotiveNotFunction {
                got: motive.clone(),
            });
        }
    };
    let instance = |endpoint: &Term| match binder {
        Some(binder) => family.substitute_term(binder, endpoint),
        None => family.clone(),
    };

    let eq_ty = type_of(ctx, eq_proof)?;
    let (left, right) = match &eq_ty {
        Type::Eq(eq_base_ty, a, b) if types_equal_in(ctx, eq_base_ty, ty) => (a, b),
        _ => return Err(TypeError::NotEquality { got: eq_ty }),
    };

    let expected = instance(left);
    let proof_ty = type_of(ctx, proof)?;
    if !types_equal_in(ctx, &proof_ty, &expected) {
        return Err(TypeError::TypeMismatch {
            expected,
            got: proof_ty,
        });
    }

    Ok(instance(right))
}

/// Type check fix point: `fix f:τ. t : τ`
pub(in crate::typecheck) fn type_of_fix(
    ctx: &Context,
    f: &str,
    ty: &Type,
    body: &Term,
) -> TypeResult<Type> {
    check_type_wf(ctx, ty)?;
    let body_ctx = ctx.with_term(f, ty.clone());
    let body_ty = type_of(&body_ctx, body)?;
    if !types_equal_in(ctx, &body_ty, ty) {
        return Err(TypeError::TypeMismatch {
            expected: ty.clone(),
            got: body_ty,
        });
    }
    if ctx.is_proof() {
        let mut type_params: Vec<String> = ctx.type_vars().into_iter().collect();
        type_params.sort();
        // The certified position is δ's (ADR 2.10.26l D4); typing needs none.
        fix_admission(f, ty, body, &type_params, None).map(|_| ())?;
    }
    Ok(ty.clone())
}

/// Type check fold: `fold [μα.τ] t : μα.τ`
pub(in crate::typecheck) fn type_of_fold(
    ctx: &Context,
    mu_ty: &Type,
    t: &Term,
) -> TypeResult<Type> {
    check_type_wf(ctx, mu_ty)?;
    mu_positivity(mu_ty)?;
    match mu_ty {
        Type::Mu(alpha, body) => {
            // Expected type for t is body with α replaced by μα.body
            let expected = body.substitute(alpha, mu_ty);
            let t_ty = type_of(ctx, t)?;
            if !types_equal_in(ctx, &t_ty, &expected) {
                return Err(TypeError::TypeMismatch {
                    expected,
                    got: t_ty,
                });
            }
            Ok(mu_ty.clone())
        }
        _ => Err(TypeError::MalformedType(mu_ty.clone())),
    }
}

/// Type check unfold: `unfold [μα.τ] t : τ[α := μα.τ]`
pub(in crate::typecheck) fn type_of_unfold(
    ctx: &Context,
    mu_ty: &Type,
    t: &Term,
) -> TypeResult<Type> {
    check_type_wf(ctx, mu_ty)?;
    mu_positivity(mu_ty)?;
    match mu_ty {
        Type::Mu(alpha, body) => {
            let t_ty = type_of(ctx, t)?;
            if !types_equal_in(ctx, &t_ty, mu_ty) {
                return Err(TypeError::TypeMismatch {
                    expected: mu_ty.clone(),
                    got: t_ty,
                });
            }
            Ok(body.substitute(alpha, mu_ty))
        }
        _ => Err(TypeError::MalformedType(mu_ty.clone())),
    }
}
