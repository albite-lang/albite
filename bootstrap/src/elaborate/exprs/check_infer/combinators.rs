//! Derived equality combinators (ADR 21.5.26d, Phase 3).
//!
//! `sym`, `trans`, `cong` are elaborator desugaring into `Term::Subst` — no new
//! core calculus primitives. Each combinator infers its arguments' types and
//! constructs the appropriate `Term::Subst` with the computed result type.
//!
//! Each `Subst` stores the motive it substitutes into, `Π x:τ. P`, so the
//! kernel can recompute `P[x := a]` and `P[x := b]` (ADR 1.10.26e D3).

use albite_core::{Term, Type};

use crate::ast::Expr;
use crate::elaborate::error::ElabError;
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::Span;

/// The binder of every motive `sym`, `trans` and `cong` synthesise (ADR
/// 1.10.26e P0.4). `#` cannot start an identifier, so no source name can meet
/// it; it is neither conversion's `?<n>` nor `fresh_name`'s `′` suffix.
pub(crate) const SYNTHESISED_MOTIVE_BINDER: &str = "#x";

/// `Π #x:base. body(#x)`, the motive a derived combinator stores — and
/// `rewrite`'s (ADR 2.10.26e D4).
pub(crate) fn synthesised_motive(base_ty: &Type, body: impl FnOnce(Term) -> Type) -> Type {
    let bound = Term::var(SYNTHESISED_MOTIVE_BINDER);
    Type::pi_or_arrow(SYNTHESISED_MOTIVE_BINDER, base_ty.clone(), body(bound))
}

impl Elaborator<'_> {
    /// Validate a compiler-synthesised motive type (ADR 21.5.26g, P4).
    ///
    /// Derived combinators (`sym`, `trans`, `cong`) construct motives as
    /// `Π x:base_ty. P` (ADR 1.10.26e D3). This function validates the
    /// synthesised motive through the same domain check as user-written
    /// motives — no trusted bypass. `rewrite`'s motive passes it too (ADR
    /// 2.10.26e D4).
    pub(crate) fn validate_synthesised_motive(
        &self,
        motive: &Type,
        base_ty: &Type,
        span: Span,
    ) -> ElabResult<()> {
        match motive {
            Type::Pi(_, domain, _) | Type::Arrow(domain, _) => {
                if !self.types_equal(domain, base_ty) {
                    return Err(ElabError::motive_domain_mismatch(
                        span,
                        base_ty.clone(),
                        (**domain).clone(),
                    ));
                }
                Ok(())
            }
            _ => Err(ElabError::motive_not_predicate(span, motive.clone())),
        }
    }

    /// `sym(h)` where `h : Eq(τ, a, b)` → `Eq(τ, b, a)`
    ///
    /// Desugars to: `subst(h, |x| Eq(τ, x, a), refl(τ, a))`
    pub(crate) fn infer_sym(&mut self, proof_expr: &Expr, span: Span) -> ElabResult<(Term, Type)> {
        let (proof_term, proof_ty) = self.infer(proof_expr)?;
        let (base_ty, a, b) = extract_eq(&proof_ty, span)?;
        let term = self.sym_term(&base_ty, &a, proof_term, span)?;
        // Result type: Eq(τ, b, a) = motive(b)
        Ok((term, Type::eq(base_ty, b, a)))
    }

    /// `sym`'s one lowering, shared with `rewrite` (ADR 2.10.26e D4): for
    /// `proof : Eq(τ, a, b)`, `subst[τ][Π x:τ. Eq(τ, x, a)](proof, refl(τ, a))`.
    pub(crate) fn sym_term(
        &self,
        base_ty: &Type,
        a: &Term,
        proof: Term,
        span: Span,
    ) -> ElabResult<Term> {
        // Motive: Π x:τ. Eq(τ, x, a)
        let motive = synthesised_motive(base_ty, |x| Type::eq(base_ty.clone(), x, a.clone()));
        self.validate_synthesised_motive(&motive, base_ty, span)?;
        // Witness: refl(τ, a) proves Eq(τ, a, a) = motive(a)
        let witness = Term::refl(base_ty.clone(), a.clone());
        Ok(Term::subst(base_ty.clone(), motive, proof, witness))
    }

    /// `trans(h1, h2)` where `h1 : Eq(τ, a, b)`, `h2 : Eq(τ, b, c)` → `Eq(τ, a, c)`
    ///
    /// Desugars to: `subst(h2, |x| Eq(τ, a, x), h1)`
    pub(crate) fn infer_trans(
        &mut self,
        h1_expr: &Expr,
        h2_expr: &Expr,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        let (h1_term, h1_ty) = self.infer(h1_expr)?;
        let (base_ty, a, b) = extract_eq(&h1_ty, span)?;

        let (h2_term, h2_ty) = self.infer(h2_expr)?;
        let (_, b2, c) = extract_eq(&h2_ty, span)?;

        // Check that h1's right endpoint matches h2's left endpoint — now, or
        // after admission when δ could decide it (ADR 28.9.26d §2.1).
        let mismatch = ElabError::trans_endpoint_mismatch(span, b.clone(), b2.clone());
        self.convert_or_defer(&b, &b2, &base_ty, mismatch)?;

        // Motive: Π x:τ. Eq(τ, a, x)
        let motive = synthesised_motive(&base_ty, |x| Type::eq(base_ty.clone(), a.clone(), x));
        self.validate_synthesised_motive(&motive, &base_ty, span)?;
        // Witness: h1 proves Eq(τ, a, b), which meets motive(b2) by conversion
        // Result type: Eq(τ, a, c) = motive(c)
        let result_ty = Type::eq(base_ty.clone(), a.clone(), c.clone());

        let term = Term::subst(base_ty, motive, h2_term, h1_term);
        Ok((term, result_ty))
    }

    /// `cong(f, h)` where `f : τ → σ`, `h : Eq(τ, a, b)` → `Eq(σ, f(a), f(b))`
    ///
    /// Desugars to: `subst(h, |x| Eq(σ, f(a), f(x)), refl(σ, f(a)))`
    pub(crate) fn infer_cong(
        &mut self,
        f_expr: &Expr,
        proof_expr: &Expr,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        let (f_term, f_ty) = self.infer(f_expr)?;
        let (proof_term, proof_ty) = self.infer(proof_expr)?;
        let (base_ty, a, b) = extract_eq(&proof_ty, span)?;

        // Extract codomain from f's type
        let codomain = match &f_ty {
            Type::Arrow(_, cod) => (**cod).clone(),
            _ => return Err(ElabError::cong_expected_function(span, f_ty)),
        };

        // Compute f(a) and f(b) as term applications
        let f_a = Term::app(f_term.clone(), a.clone());
        let f_b = Term::app(f_term.clone(), b.clone());

        // Motive: Π x:τ. Eq(σ, f(a), f(x))
        let motive = synthesised_motive(&base_ty, |x| {
            Type::eq(codomain.clone(), f_a.clone(), Term::app(f_term.clone(), x))
        });
        self.validate_synthesised_motive(&motive, &base_ty, span)?;
        // Witness: refl(σ, f(a)) proves Eq(σ, f(a), f(a)) = motive(a)
        let witness = Term::refl(codomain.clone(), f_a.clone());
        // Result type: Eq(σ, f(a), f(b)) = motive(b)
        let result_ty = Type::eq(codomain, f_a, f_b);

        let term = Term::subst(base_ty, motive, proof_term, witness);
        Ok((term, result_ty))
    }
}

/// Extract `(τ, a, b)` from an `Eq(τ, a, b)` type, or produce an error —
/// E0072, which a `rewrite` rule that is not an equation raises too.
pub(crate) fn extract_eq(ty: &Type, span: Span) -> ElabResult<(Type, Term, Term)> {
    match ty {
        Type::Eq(base, a, b) => Ok(((**base).clone(), (**a).clone(), (**b).clone())),
        _ => Err(ElabError::subst_expected_equality(span, ty.clone())),
    }
}
