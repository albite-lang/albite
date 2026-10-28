//! The dependent pair `Σ x:A. B`, in both modes (ADR 4.10.26h D2, D3).
//!
//! There is no `Term` variant for it: a dependent pair is
//! `Annot(Pair(a, b), Sigma(x, A, B))`, the annotation carrying the type into
//! the kernel. The introduction rule checks `a : A` and `b : B[x := a]`; the
//! projections answer `A` and `B[x := fst p]`. β needs nothing here —
//! conversion already peels `Annot` and reduces `fst`/`snd` of a `Pair` — and
//! there is no η, as for `Product`.

use crate::context::Context;
use crate::terms::Term;
use crate::types::{Sort, Type};

use crate::typecheck::error::TypeError;
use crate::typecheck::rules::{check, type_of, types_equal_in};
use crate::typecheck::TypeResult;

/// `(left, right)` when `term`, spans peeled, is a `Pair`.
pub(crate) fn as_pair(term: &Term) -> Option<(&Term, &Term)> {
    match term {
        Term::Pair(left, right) => Some((left, right)),
        Term::Spanned(inner, _) => as_pair(inner),
        _ => None,
    }
}

/// `Γ ⊢ Annot((a, b), Σ x:A. B) : Σ x:A. B` when `Γ ⊢ a : A` and
/// `Γ ⊢ b : B[x := a]`. The annotation's well-formedness is the caller's.
///
/// # Errors
///
/// What checking `a` against `A` raises; a `TypeMismatch` against the
/// instance `B[x := a]` when `b` has another type, so the error names it.
pub(crate) fn type_of_sigma_pair(
    ctx: &Context,
    (witness, evidence): (&Term, &Term),
    (binder, binder_ty, body): (&str, &Type, &Type),
    annotation: &Type,
) -> TypeResult<Type> {
    check(ctx, witness, binder_ty)?;
    let instance = body.substitute_term(binder, witness);
    let evidence_ty = type_of(ctx, evidence)?;
    if types_equal_in(ctx, &evidence_ty, &instance) {
        Ok(annotation.clone())
    } else {
        Err(TypeError::TypeMismatch {
            expected: instance,
            got: evidence_ty,
        })
    }
}

/// `snd p : B[x := fst p]` for `p : Σ x:A. B`, with `p` as written.
#[must_use]
pub(crate) fn snd_instance(binder: &str, body: &Type, projected: &Term) -> Type {
    body.substitute_term(binder, &Term::fst(projected.clone()))
}

/// Q4's `sort_of(A) == Type₀` (ADR 4.10.26i D1, replacing 4.10.26h D3's
/// stand-in): every `Sigma` in `ty` must bind data. A binder with no sort (a
/// poison) is transit, not a raise.
///
/// # Errors
///
/// `SigmaBinderNotData` naming the first offending binder type.
pub(crate) fn check_sigma_binders(ty: &Type) -> TypeResult<()> {
    if let Type::Sigma(_, binder_ty, _) = ty {
        if binder_ty.sort_of() == Some(Sort::Prop) {
            return Err(TypeError::SigmaBinderNotData {
                got: (**binder_ty).clone(),
            });
        }
    }
    ty.children().into_iter().try_for_each(check_sigma_binders)
}
