//! The terms inside an `Eq` type are typed in proof mode (ADR 1.10.26c D7).
//!
//! Proof mode converts an `Eq`'s witnesses, and a reduction means something
//! only on a well-typed term: an ill-typed witness could make two different
//! propositions convert. So at each `Eq(τ, a, b)` the walk checks `a : τ` and
//! `b : τ` with the proof judgment, under every binder in scope — a `Pi`'s
//! term binder, a `∀`/`μ`'s type variable, and a `natind` motive's binder
//! (through [`check_motive`]).
//!
//! The proof judgment, not the executable one: a proposition that mentions an
//! `Opaque` global is refused (26a §9's `#[partial]` boundary).
//!
//! The same walk types a `PredApp`'s arguments against its kind's domains,
//! and refuses a `ForallPred` or `PredAbs` it meets (ADR 6.10.26h D6). That
//! half runs in **every** mode: no corpus carried a predicate form before it,
//! so nothing can regress, while `Eq` witnesses outside proof mode stay
//! unchecked, as before.

use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

use super::error::TypeError;
use super::rules::{check as check_term, check_type_wf};
use super::TypeResult;

/// Type every `Eq` witness (in proof mode) and every `PredApp` argument in
/// `ty`, under `ctx` and the binders `ty` itself opens.
///
/// # Errors
///
/// `TypeError::IllTypedEqWitness` naming the first `Eq` whose witness does
/// not check, with the inner refusal as its cause; a predicate-binder
/// refusal for a misused predicate form.
pub(super) fn check(ctx: &Context, ty: &Type) -> TypeResult<()> {
    match ty {
        // A `Sigma`'s binder scopes its body exactly as a `Pi`'s (ADR 4.10.26h)
        Type::Pi(binder, domain, codomain) | Type::Sigma(binder, domain, codomain) => {
            check(ctx, domain)?;
            check(&ctx.with_term(binder.clone(), (**domain).clone()), codomain)
        }
        Type::Forall(alpha, body) | Type::Mu(alpha, body) => {
            check(&ctx.with_type_var(alpha.clone()), body)
        }
        Type::Eq(base, left, right) => {
            check(ctx, base)?;
            if ctx.is_proof() {
                witness_checks(ctx, ty, left, base)?;
                witness_checks(ctx, ty, right, base)?;
            }
            Ok(())
        }
        Type::PredApp(pred, args) => pred_app_checks(ctx, pred, args),
        // Admitted only by `check_signature_wf`'s prefix and `TyApp`'s
        // argument rule, each of which runs before this walk (D6).
        Type::ForallPred(..) => Err(TypeError::PredicateBinderNotPrenex { ty: ty.clone() }),
        Type::PredAbs(..) => Err(TypeError::StrayPredicate { ty: ty.clone() }),
        _ => ty
            .children()
            .into_iter()
            .try_for_each(|child| check(ctx, child)),
    }
}

/// `P<args>` is well formed when `P` is a predicate variable of `ctx` with
/// one domain per argument, and each argument checks against its domain.
fn pred_app_checks(ctx: &Context, pred: &str, args: &[Term]) -> TypeResult<()> {
    let Some(domains) = ctx.lookup_pred_var(pred) else {
        return Err(if ctx.has_type_var(pred) {
            TypeError::PredicateVarIsTypeVar(pred.to_string())
        } else {
            TypeError::UnboundPredicateVar(pred.to_string())
        });
    };
    if domains.len() != args.len() {
        return Err(TypeError::PredicateArity {
            var: pred.to_string(),
            expected: domains.len(),
            got: args.len(),
        });
    }
    let domains = domains.to_vec();
    args.iter()
        .zip(&domains)
        .try_for_each(|(arg, domain)| check_term(ctx, arg, domain))
}

/// A `natind` motive `Nat → P`, well formed with `binder : Nat` in scope, so
/// the witnesses of a dependent `P` may name it.
///
/// # Errors
///
/// What [`check_type_wf`] raises for the motive under that context.
pub(super) fn check_motive(ctx: &Context, binder: &str, motive_ty: &Type) -> TypeResult<()> {
    check_type_wf(&ctx.with_term(binder, Type::Nat), motive_ty)
}

fn witness_checks(ctx: &Context, eq: &Type, witness: &Term, base: &Type) -> TypeResult<()> {
    check_term(ctx, witness, base).map_err(|cause| TypeError::IllTypedEqWitness {
        eq: eq.clone(),
        cause: Box::new(cause),
    })
}
