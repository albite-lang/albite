//! Proof construct elaboration.
//!
//! Handles:
//! - `have h: P = proof; body` - proof binding
//! - `show P { proof }` - type ascription
//! - `assume h: P; body` - lambda introduction
//! - `absurd(h)` - refutation, in check mode (ADR 7.10.26m D4)

use crate::ast::{self, Expr};
use crate::span::{Span, Spanned};
use albite_core::conversion::convertible_without_delta;
use albite_core::types::eq_witnesses_equal_modulo_spans;
use albite_core::{Term, Type};

use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::{ElabResult, Elaborator};

/// The note on `absurd`'s E0010 (D4).
const ABSURD_NOTE: &str = "`absurd` needs `Void` or an equation between `true` and `false`";

/// Whether `left == right` is `true == false` or `false == true`. In a proof
/// body each side converts δ-free, as the kernel's proof mode converts;
/// elsewhere it is equality modulo spans, the kernel's executable-mode
/// comparison, so the elaborator never emits an `Absurd` the kernel refuses.
fn refutes_true_false(left: &Term, right: &Term, in_proof_body: bool) -> bool {
    let meets = |found: &Term, wanted: &Term| {
        if in_proof_body {
            convertible_without_delta(found, wanted)
        } else {
            eq_witnesses_equal_modulo_spans(((found, found), (wanted, wanted)), &[])
        }
    };
    let oriented = |first: Term, second: Term| meets(left, &first) && meets(right, &second);
    oriented(Term::True, Term::False) || oriented(Term::False, Term::True)
}

impl<'a> Elaborator<'a> {
    /// Whether `func(args)` is the built-in `absurd(h)`: a simple path
    /// `absurd` that resolves to no value or constructor, applied to one
    /// argument, so no program accepted before changes meaning (D4).
    fn is_builtin_absurd(&self, func: &Expr, args: &[Expr]) -> bool {
        let Expr::Path(path) = func else {
            return false;
        };
        path.is_simple()
            && path.item_name().name == "absurd"
            && args.len() == 1
            && matches!(
                self.env
                    .resolve_value_path(path, self.depth, &self.current_module),
                Ok(None)
            )
    }

    /// `absurd(h)` against `expected`: `Absurd(expected, h)` when `h` is
    /// `Void` or refutes `true == false` (D4); E0010 otherwise. `None` when
    /// `func(args)` is not the built-in, so the caller takes today's path.
    pub(in crate::elaborate::exprs) fn check_absurd(
        &mut self,
        func: &Expr,
        args: &[Expr],
        expected: &Type,
    ) -> ElabResult<Option<Term>> {
        if !self.is_builtin_absurd(func, args) {
            return Ok(None);
        }
        let (proof, found) = self.infer(&args[0])?;
        let refuted = match self.normalize_for_comparison(&found) {
            Type::Void => true,
            Type::Eq(base, left, right) => {
                *base == Type::Bool && refutes_true_false(&left, &right, self.in_proof_body)
            }
            // Poison passes through (ADR 15.8.26d): its fault is already reported.
            // Defensive: a poisoned signature stops the file before any body,
            // so no parsed input reaches this arm today.
            Type::Error => return Ok(Some(Term::Sorry)),
            _ => false,
        };
        if !refuted {
            return Err(
                ElabError::type_mismatch(args[0].span(), Type::Void, found).with_note(ABSURD_NOTE)
            );
        }
        Ok(Some(Term::absurd(expected.clone(), proof)))
    }

    /// Elaborate `have h: P = proof; body`.
    pub(in crate::elaborate::exprs) fn elab_have(
        &mut self,
        name: &ast::Ident,
        prop: &ast::TypeExpr,
        proof: &Expr,
        body: &Expr,
        expected: Option<&Type>,
    ) -> ElabResult<(Term, Type)> {
        // have h: P = proof; body  →  let h: P = proof in body
        let prop_ty = self.elab_type(prop)?;
        let proof_term = self.check(proof, &prop_ty)?;

        self.env.push_scope();
        self.env
            .bind_local(name.name.clone(), prop_ty.clone(), self.depth);
        self.depth += 1;

        let (body_term, body_ty) = if let Some(expected) = expected {
            let term = self.check(body, expected)?;
            (term, expected.clone())
        } else {
            self.infer(body)?
        };

        self.depth -= 1;
        self.env.pop_scope();

        let term = Term::let_in(&name.name, prop_ty, proof_term, body_term);
        Ok((term, body_ty))
    }

    /// Elaborate `show P { proof }`.
    pub(in crate::elaborate::exprs) fn elab_show(
        &mut self,
        prop: &ast::TypeExpr,
        proof: &Expr,
        expected: Option<&Type>,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        // show P { proof }  →  (proof : P)
        let prop_ty = self.elab_type(prop)?;

        // If we have an expected type, verify it matches
        if let Some(expected) = expected {
            if *expected != prop_ty {
                return Err(ElabError::type_mismatch(span, expected.clone(), prop_ty));
            }
        }

        let term = self.check(proof, &prop_ty)?;
        Ok((term, prop_ty))
    }

    /// Elaborate `assume h: P; body`.
    pub(in crate::elaborate::exprs) fn elab_assume(
        &mut self,
        name: &ast::Ident,
        prop: &ast::TypeExpr,
        body: &Expr,
        expected: Option<&Type>,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        // assume h: P; body  →  λ (h: P). body
        let prop_ty = self.elab_type(prop)?;

        // Expected type must be P → result_ty
        let result_expected = if let Some(expected) = expected {
            let Type::Arrow(param_ty, result_ty) = expected else {
                return Err(ElabError::new(
                    span,
                    ElabErrorKind::ExpectedType {
                        expected: "function type".to_string(),
                        found: expected.clone(),
                    },
                ));
            };
            if !self.types_equal(param_ty, &prop_ty) {
                return Err(ElabError::type_mismatch(
                    prop.span(),
                    (**param_ty).clone(),
                    prop_ty,
                ));
            }
            Some(&**result_ty)
        } else {
            None
        };

        self.env.push_scope();
        self.env
            .bind_local(name.name.clone(), prop_ty.clone(), self.depth);
        self.depth += 1;

        let (body_term, body_ty) = if let Some(expected) = result_expected {
            let term = self.check(body, expected)?;
            (term, expected.clone())
        } else {
            self.infer(body)?
        };

        self.depth -= 1;
        self.env.pop_scope();

        let term = Term::lambda(&name.name, prop_ty.clone(), body_term);
        let ty = Type::arrow(prop_ty, body_ty);
        Ok((term, ty))
    }
}
