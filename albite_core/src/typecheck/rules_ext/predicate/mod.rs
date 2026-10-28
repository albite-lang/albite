//! Predicate binders in the kernel (ADR 6.10.26h D5, D6).
//!
//! A `ForallPred` is admitted in exactly one place — the prenex prefix of a
//! global's signature, by [`check_signature_wf`] — and instantiated in
//! exactly one way — a `TyApp` whose argument is a `PredAbs`, by
//! [`type_of_pred_tyapp`]. Everywhere else `check_type_wf` refuses both
//! `ForallPred` and `PredAbs` (`eq_witnesses.rs`), so no quantifier ranges
//! over a kind or over `Prop` (Q6), and Girard's and Hurkens' shapes still
//! have no spelling.
//!
//! A definition with a predicate parameter is introduced by annotation,
//! `Annot(TyAbs(P, body), ForallPred(P, Ds, B))`, on the dependent pair's
//! precedent (4.10.26h D2): a bare `TyAbs` binds `P` kind-less, and the
//! annotation is what carries the kind into the context.

use crate::context::Context;
use crate::terms::Term;
use crate::types::{Sort, Type};

use crate::typecheck::error::TypeError;
use crate::typecheck::rules::{check, check_type_wf, type_of, types_equal_in};
use crate::typecheck::TypeResult;

/// A global's declared type: its prenex prefix of `Forall` and `ForallPred`
/// binders, each kind valid, then a matrix `check_type_wf` accepts.
///
/// # Errors
///
/// `PredicateKindEmpty`, `PredicateDomainOpen` or `PredicateDomainNotData`
/// for a bad kind; whatever `check_type_wf` raises for a domain or for the
/// matrix — including `PredicateBinderNotPrenex` for a `ForallPred` below
/// the prefix.
pub(crate) fn check_signature_wf(ctx: &Context, ty: &Type) -> TypeResult<()> {
    let (prefix_ctx, matrix) = open_prefix(ctx, ty)?;
    check_type_wf(&prefix_ctx, matrix)
}

/// `ctx` extended with `ty`'s prenex binders, and the matrix under them.
fn open_prefix<'t>(ctx: &Context, ty: &'t Type) -> TypeResult<(Context, &'t Type)> {
    let mut ctx = ctx.clone();
    let mut current = ty;
    loop {
        match current {
            Type::Forall(alpha, body) => {
                ctx = ctx.with_type_var(alpha.clone());
                current = body;
            }
            Type::ForallPred(pred, domains, body) => {
                check_kind(&ctx, pred, domains)?;
                ctx = ctx.with_pred_var(pred.clone(), domains.clone());
                current = body;
            }
            _ => return Ok((ctx, current)),
        }
    }
}

/// A kind is a non-empty list of closed, `Type₀`-sorted domains (D6).
fn check_kind(ctx: &Context, pred: &str, domains: &[Type]) -> TypeResult<()> {
    if domains.is_empty() {
        return Err(TypeError::PredicateKindEmpty {
            var: pred.to_string(),
        });
    }
    let in_scope = ctx.type_vars();
    for domain in domains {
        // Closed: an instantiation of an earlier `Forall` must not be able to
        // change a domain's sort (`α := Eq<…>` would make it `Prop`).
        if domain.free_type_vars().iter().any(|v| in_scope.contains(v)) {
            return Err(TypeError::PredicateDomainOpen {
                var: pred.to_string(),
                domain: domain.clone(),
            });
        }
        check_type_wf(ctx, domain)?;
        if domain.sort_of() != Some(Sort::Type0) {
            return Err(TypeError::PredicateDomainNotData {
                var: pred.to_string(),
                domain: domain.clone(),
            });
        }
    }
    Ok(())
}

/// `Γ ⊢ Annot(Λ… t, ∀…. B) : ∀…. B` for a prefix holding a `ForallPred`:
/// the signature is checked, then each `TyAbs` opens its binder — with its
/// kind under a `ForallPred` — and `t` is checked against the matrix.
///
/// # Errors
///
/// What [`check_signature_wf`] raises; `PredicateIntroShape` when the term
/// is not a `TyAbs` per prefix binder; what checking `t` raises.
pub(crate) fn type_of_pred_annot(ctx: &Context, term: &Term, ty: &Type) -> TypeResult<Type> {
    check_signature_wf(ctx, ty)?;
    if is_sorry(term) {
        return Ok(ty.clone());
    }
    let mut ctx = ctx.clone();
    let mut term = term;
    let mut current = ty.clone();
    loop {
        let (binder, next) = match &current {
            Type::Forall(alpha, body) => (alpha, body),
            Type::ForallPred(pred, _, body) => (pred, body),
            _ => break,
        };
        let Some((name, inner)) = as_ty_abs(term) else {
            return Err(TypeError::PredicateIntroShape { ty: ty.clone() });
        };
        ctx = match &current {
            Type::ForallPred(_, domains, _) => ctx.with_pred_var(name, domains.clone()),
            _ => ctx.with_type_var(name),
        };
        // The binder renamed to the abstraction's own name; a `PredApp`
        // follows a renaming (`Type::substitute`'s `TyVar` arm).
        let next = if binder == name {
            (**next).clone()
        } else {
            next.substitute(binder, &Type::TyVar(name.to_string()))
        };
        current = next;
        term = inner;
    }
    check(&ctx, term, &current)?;
    Ok(ty.clone())
}

/// `(α, body)` when `term`, spans peeled, is a `TyAbs`.
fn as_ty_abs(term: &Term) -> Option<(&str, &Term)> {
    match term {
        Term::TyAbs(alpha, body) => Some((alpha, body)),
        Term::Spanned(inner, _) => as_ty_abs(inner),
        _ => None,
    }
}

fn is_sorry(term: &Term) -> bool {
    match term {
        Term::Sorry => true,
        Term::Spanned(inner, _) => is_sorry(inner),
        _ => false,
    }
}

/// `Γ ⊢ f [λxs. T] : B[P := λxs. T]` when `f : ∀P : Ds → Prop. B`, each
/// `xᵢ : Dᵢ`, and `T` is a well-formed proposition under `xs` (D6). The
/// substitution is hereditary, so the result holds no redex.
///
/// # Errors
///
/// `PredicateInstantiatesForall` against a plain `Forall`;
/// `PredicateKindMismatch` for wrong parameters or a non-`Prop` body; what
/// `check_type_wf` raises for the body; `NotPolymorphic` otherwise.
pub(crate) fn type_of_pred_tyapp(ctx: &Context, f: &Term, predicate: &Type) -> TypeResult<Type> {
    let (params, body) = match predicate {
        Type::PredAbs(params, body) => (params, body),
        // Poison transits, as everywhere else in the kernel's rules
        Type::Error => return Ok(Type::Error),
        _ => unreachable!("type_of_pred_tyapp is dispatched on a PredAbs argument"),
    };
    let f_ty = type_of(ctx, f)?;
    let (pred, domains, scope) = match &f_ty {
        Type::ForallPred(pred, domains, scope) => (pred, domains, scope),
        Type::Forall(..) => {
            return Err(TypeError::PredicateInstantiatesForall {
                got: predicate.clone(),
            })
        }
        Type::Error => return Ok(Type::Error),
        _ => return Err(TypeError::NotPolymorphic { got: f_ty }),
    };
    let mismatch = || TypeError::PredicateKindMismatch {
        expected: domains.clone(),
        got: predicate.clone(),
    };
    if params.len() != domains.len()
        || !params
            .iter()
            .zip(domains)
            .all(|((_, d), want)| types_equal_in(ctx, d, want))
    {
        return Err(mismatch());
    }
    let body_ctx = params.iter().fold(ctx.clone(), |acc, (x, d)| {
        acc.with_term(x.clone(), d.clone())
    });
    check_type_wf(&body_ctx, body)?;
    if body.sort_of() != Some(Sort::Prop) {
        return Err(mismatch());
    }
    Ok(scope.substitute(pred, predicate))
}
