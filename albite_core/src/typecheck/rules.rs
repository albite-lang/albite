//! Typing rules implementation
//!
//! This module contains the main type synthesis function `type_of` which
//! implements the typing judgment Γ ⊢ t : τ, and the proof judgment's entry
//! points (ADR 29.9.26e).

use crate::context::{Context, GlobalAdmission};
use crate::terms::Term;
use crate::types::Type;

use super::error::TypeError;
use super::TypeResult;

/// Check if a type is well-formed under a context
///
/// # Errors
///
/// `TypeError::MalformedType` when `ty` names a type variable `ctx` does not
/// bind; in proof mode, `TypeError::IllTypedEqWitness` when a term inside one
/// of its `Eq`s does not have that `Eq`'s base type (ADR 1.10.26c D7).
pub fn check_type_wf(ctx: &Context, ty: &Type) -> TypeResult<()> {
    let type_vars = ctx.type_vars();
    if !ty.is_well_formed(&type_vars) {
        return Err(TypeError::MalformedType(ty.clone()));
    }
    super::rules_ext::check_sigma_binders(ty)?;
    // A predicate form is checked in every mode (ADR 6.10.26h D6); the cheap
    // pre-test keeps the executable judgment's walk-free path for every other type.
    if ctx.is_proof() || ty.mentions_predicate_form() {
        super::eq_witnesses::check(ctx, ty)?;
    }
    Ok(())
}

/// Check if two types are definitionally equal
///
/// Uses α-equivalence to handle bound variable renaming in μ-types and ∀-types.
/// This ensures that `μα. Unit + α` equals `μβ. Unit + β`.
#[must_use]
pub fn types_equal(ty1: &Type, ty2: &Type) -> bool {
    // Use α-equivalent type equality from types module
    crate::types::types_equal_alpha(ty1, ty2)
}

/// Type equality under `ctx`'s judgment: in proof mode, strict α-equivalence
/// up to conversion of `Eq` witnesses, unfolding only the bodies `ctx`
/// admitted (ADR 29.9.26e D11, 1.10.26c D1, 2.10.26l D3);
/// [`types_equal`] otherwise. The strict test runs first, so conversion is
/// paid only on what it refuses.
#[must_use]
pub(super) fn types_equal_in(ctx: &Context, ty1: &Type, ty2: &Type) -> bool {
    if ctx.is_proof() {
        crate::types::types_equal_strict(ty1, ty2) || super::convert::types_convert(ctx, ty1, ty2)
    } else {
        types_equal(ty1, ty2)
    }
}

/// Type check a term against an expected type
///
/// # Errors
///
/// Any error [`type_of`] raises for `term`, or `TypeError::TypeMismatch` when
/// the synthesized type is not `expected`.
pub fn check(ctx: &Context, term: &Term, expected: &Type) -> TypeResult<()> {
    let inferred = type_of(ctx, term)?;
    expect_type(ctx, inferred, expected)
}

/// Synthesize the type of a proof term (ADR 29.9.26e).
///
/// The proof judgment refuses what [`type_of`] admits for executable code: a
/// `fix` the termination engine does not certify, a `return`, a `sorry`, a
/// poisoned type, and any global the caller did not bind as admitted. It
/// compares types strictly, up to δ-free conversion of `Eq` witnesses, and
/// types those witnesses (ADR 1.10.26c). Its verdict is conditional on `ctx`'s `with_term`
/// hypotheses; a closed proof is checked under a context holding only
/// `with_global` and type-variable bindings.
///
/// # Errors
///
/// A gate's `TypeError` (`ReturnInProof`, `SorryInProof`, `PoisonInProof`,
/// `FixNotStructural`, `GlobalNotAdmitted`, `NonStrictlyPositiveMu`), or any
/// error [`type_of`] raises.
pub fn type_of_proof(ctx: &Context, term: &Term) -> TypeResult<Type> {
    super::gates::proof_prepass(ctx, term)?;
    type_of(&ctx.for_proof(), term)
}

/// Check a proof term against an expected type, strictly up to δ-free
/// conversion.
///
/// The statement is screened as the term is — the pre-pass gate, then its
/// well-formedness, witnesses included — but only after the term, so the
/// term's refusal stays the first one (ADR 1.10.26c D7).
///
/// # Errors
///
/// Any error [`type_of_proof`] raises for `term`; a gate's or
/// [`check_type_wf`]'s error for `expected`; or `TypeError::TypeMismatch`
/// when the synthesized type does not convert to `expected`.
pub fn check_proof(ctx: &Context, term: &Term, expected: &Type) -> TypeResult<()> {
    let inferred = type_of_proof(ctx, term)?;
    let proof_ctx = ctx.for_proof();
    super::gates::type_admissible_in_proof(expected)?;
    // A statement may open with a predicate prefix (ADR 6.10.26h D6)
    super::rules_ext::check_signature_wf(&proof_ctx, expected)?;
    expect_type(&proof_ctx, inferred, expected)
}

fn expect_type(ctx: &Context, inferred: Type, expected: &Type) -> TypeResult<()> {
    if types_equal_in(ctx, &inferred, expected) {
        Ok(())
    } else {
        Err(TypeError::TypeMismatch {
            expected: expected.clone(),
            got: inferred,
        })
    }
}

/// Synthesize the type of a term: Γ ⊢ t : τ
///
/// # Errors
///
/// A `TypeError` naming the first typing rule `term` violates — an unbound
/// variable, a malformed annotation, a mismatch between a sub-term's type and
/// the one its rule requires.
pub fn type_of(ctx: &Context, term: &Term) -> TypeResult<Type> {
    match term {
        Term::Var(v) => type_of_name(ctx, v, false),
        Term::Global(name) => type_of_name(ctx, name, true),

        Term::Lambda(x, ty, body) => {
            check_type_wf(ctx, ty)?;
            let body_ctx = ctx.with_term(x, ty.clone());
            let body_ty = type_of(&body_ctx, body)?;
            // A body type that mentions `x` binds it (ADR 30.9.26d D5).
            Ok(Type::pi_or_arrow(x.clone(), ty.clone(), body_ty))
        }
        Term::TyAbs(alpha, body) => {
            let body_ctx = ctx.with_type_var(alpha);
            let body_ty = type_of(&body_ctx, body)?;
            Ok(Type::forall(alpha.clone(), body_ty))
        }
        Term::Pair(t1, t2) => {
            let ty1 = type_of(ctx, t1)?;
            let ty2 = type_of(ctx, t2)?;
            Ok(Type::product(ty1, ty2))
        }
        Term::RefNew(t) => Ok(Type::ref_ty(type_of(ctx, t)?)),

        // Literal types
        Term::True | Term::False => Ok(Type::Bool),
        Term::Unit | Term::Sorry | Term::ExternCall(_, _) => Ok(Type::Unit),
        Term::Zero | Term::NatLit(_) => Ok(Type::Nat),
        Term::IntLit(_) => Ok(Type::Int),
        Term::StringLit(_) => Ok(Type::String),

        // Core language forms (delegated to rules_core)
        Term::App(t1, t2) => super::rules_core::type_of_app(ctx, t1, t2),
        Term::Let(x, ty, def, body) => super::rules_core::type_of_let(ctx, x, ty, def, body),
        Term::If(cond, then_, else_) => super::rules_core::type_of_if(ctx, cond, then_, else_),
        Term::Absurd(ty, t) => super::rules_core::type_of_absurd(ctx, ty, t),
        Term::Succ(t) => super::rules_core::type_of_succ(ctx, t),
        Term::NatRec(ty, z, s, n) => super::rules_core::type_of_natrec(ctx, ty, z, s, n),
        Term::NatInd(binder, m, z, s, n) => {
            super::rules_core::type_of_natind(ctx, (binder, m), z, s, n)
        }
        Term::Fst(t) => super::rules_core::type_of_fst(ctx, t),
        Term::Snd(t) => super::rules_core::type_of_snd(ctx, t),
        Term::Inl(sum_ty, t) => super::rules_sum::type_of_inl(ctx, sum_ty, t),
        Term::Inr(sum_ty, t) => super::rules_sum::type_of_inr(ctx, sum_ty, t),
        Term::Case(scrut, x, l, y, r) => {
            use super::rules_sum::CaseArm;
            super::rules_sum::type_of_case(
                ctx,
                scrut,
                &CaseArm { var: x, body: l },
                &CaseArm { var: y, body: r },
            )
        }
        Term::TyApp(t, ty) => super::rules_core::type_of_tyapp(ctx, t, ty),
        Term::Refl(ty, t) => super::rules_core::type_of_refl(ctx, ty, t),
        Term::Annot(t, ty) => super::rules_core::type_of_annot(ctx, t, ty),

        // Extended features (delegated to rules_ext)
        Term::Subst(ty, m, eq, p) => super::rules_ext::type_of_subst(ctx, ty, m, eq, p),
        Term::Fix(f, ty, body) => super::rules_ext::type_of_fix(ctx, f, ty, body),
        Term::Fold(mu_ty, t) => super::rules_ext::type_of_fold(ctx, mu_ty, t),
        Term::Unfold(mu_ty, t) => super::rules_ext::type_of_unfold(ctx, mu_ty, t),

        // String operations
        Term::StrConcat(t1, t2) => super::rules_ext::type_of_str_concat(ctx, t1, t2),
        Term::StrLen(t) => super::rules_ext::type_of_str_len(ctx, t),
        Term::StrEq(t1, t2) => super::rules_ext::type_of_str_eq(ctx, t1, t2),
        Term::StrCharAt(s, n) => super::rules_ext::type_of_str_char_at(ctx, s, n),
        Term::StrSubstring(s, start, len) => {
            super::rules_ext::type_of_str_substring(ctx, s, start, len)
        }

        // Arithmetic, comparison, and boolean operations
        Term::NatAdd(t1, t2)
        | Term::NatSub(t1, t2)
        | Term::NatMul(t1, t2)
        | Term::NatDiv(t1, t2)
        | Term::NatMod(t1, t2) => super::rules_ext::type_of_nat_binop(ctx, t1, t2),
        Term::NatEq(t1, t2)
        | Term::NatLt(t1, t2)
        | Term::NatLe(t1, t2)
        | Term::NatGt(t1, t2)
        | Term::NatGe(t1, t2) => super::rules_ext::type_of_nat_cmp(ctx, t1, t2),
        Term::BoolAnd(t1, t2) | Term::BoolOr(t1, t2) => {
            super::rules_ext::type_of_bool_binop(ctx, t1, t2)
        }
        Term::BoolNot(t) => super::rules_ext::type_of_bool_not(ctx, t),

        // Signed integers (ADR 14.9.26c)
        Term::IntBin(op, t1, t2) => super::rules_ext::type_of_int_bin(ctx, *op, t1, t2),
        Term::IntNeg(t) => super::rules_ext::type_of_int_neg(ctx, t),
        Term::NatToInt(t) => super::rules_ext::type_of_nat_to_int(ctx, t),
        Term::IntToNat(t) => super::rules_ext::type_of_int_to_nat(ctx, t),

        // Ref cells
        Term::RefGet(t) => super::rules_ext::type_of_ref_get(ctx, t),
        Term::RefSet(r, v) => super::rules_ext::type_of_ref_set(ctx, r, v),

        // ADTs
        Term::AdtConstruct(adt_ty, idx, payload) => {
            super::rules_ext::type_of_adt_construct(ctx, adt_ty, *idx, payload)
        }
        Term::AdtMatch(scrut, arms) => super::rules_ext::type_of_adt_match(ctx, scrut, arms),

        // Span wrapper / control flow — transparent to typing
        Term::Spanned(inner, _) => type_of(ctx, inner),
        Term::Return(t) => super::rules_ext::type_of_return(ctx, t),
        // A region (ADR 9.10.26o D3) and a call frame (ADR 13.10.26n D3) are
        // each typed as their body
        Term::Region(_, body) | Term::CallFrame(body) => type_of(ctx, body),
    }
}

/// Look a name up; in proof mode, honour the caller's admission verdict
/// (ADR 29.9.26e D4).
///
/// The check is on the lookup, not on the `Global` node: `Var` and `Global`
/// resolve against the same bindings, so a check on `Global` alone would let
/// `Var("loop")` over an opaque global through. A `Global` node must resolve
/// to an admitted global — a hypothesis is not one.
fn type_of_name(ctx: &Context, name: &str, global_node: bool) -> TypeResult<Type> {
    let ty = ctx
        .lookup_term(name)
        .cloned()
        .ok_or_else(|| TypeError::UnboundVariable(name.to_string()))?;
    if ctx.is_proof() {
        let admitted = match ctx.lookup_global_admission(name) {
            Some(admission) => admission == GlobalAdmission::Admitted,
            None => !global_node,
        };
        if !admitted {
            return Err(TypeError::GlobalNotAdmitted {
                name: name.to_string(),
            });
        }
    }
    Ok(ty)
}
