//! Type Building Helpers
//!
//! Constructs Core types from AST item signatures. Signature collection binds
//! a signature's parameters left to right before elaborating its result, so
//! a return type, a proposition or a later parameter's type may mention an
//! earlier parameter; a mentioned parameter folds into a `Pi` (ADR 30.9.26d D2).

use crate::ast;
use crate::span::{Span, Spanned};

use albite_core::Type;

use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;

impl<'a> Elaborator<'a> {
    /// Build the type of a function from its signature.
    pub(super) fn build_function_type(&mut self, func: &ast::FunctionDef) -> ElabResult<Type> {
        self.build_signature_type(&func.type_params, &func.params, |elab| {
            match func.return_type {
                Some(ref ret) => elab.elab_type(ret),
                // No return type annotation: inferred during elaboration;
                // Unit is the placeholder until then.
                None => Ok(Type::Unit),
            }
        })
    }

    /// Build the type of a theorem from its signature.
    pub(super) fn build_theorem_type(&mut self, thm: &ast::TheoremDef) -> ElabResult<Type> {
        self.build_signature_type(&thm.type_params, &thm.params, |elab| {
            elab.elab_type(&thm.prop)
        })
    }

    /// Build the type of an axiom from its signature.
    pub(super) fn build_axiom_type(&mut self, axiom: &ast::AxiomDef) -> ElabResult<Type> {
        self.build_signature_type(&axiom.type_params, &axiom.params, |elab| {
            elab.elab_type(&axiom.prop)
        })
    }

    /// `∀type_params. Π/→ params. result`, with the type parameters popped on
    /// every path, the error path included. A kinded parameter folds into a
    /// `ForallPred` where a plain one folds into a `Forall` (ADR 6.10.26h D4).
    fn build_signature_type(
        &mut self,
        type_params: &[ast::TypeParam],
        params: &[ast::Param],
        result: impl FnOnce(&mut Self) -> ElabResult<Type>,
    ) -> ElabResult<Type> {
        let mut kinds = Vec::with_capacity(type_params.len());
        let mut kind_error = None;
        for tp in type_params {
            // Each kind is read with the parameters to its left in scope.
            match self.elab_type_param_kind(tp) {
                Ok(kind) => {
                    self.env
                        .push_kinded_type_var(tp.name.name.clone(), kind.clone());
                    kinds.push(kind);
                }
                Err(error) => {
                    kind_error = Some(error);
                    break;
                }
            }
        }
        let built = match kind_error {
            Some(error) => Err(error),
            None => self.bind_params_then(params, result),
        };
        for _ in &kinds {
            self.env.pop_type_var();
        }
        let mut ty = built?;
        for (tp, kind) in type_params.iter().zip(kinds).rev() {
            ty = match kind {
                Some(domains) => Type::forall_pred(&tp.name.name, domains, ty),
                None => Type::forall(&tp.name.name, ty),
            };
        }
        Ok(ty)
    }

    /// Bind `params` left to right in a fresh scope, run `result`, and fold
    /// the parameters over it with [`Type::pi_or_arrow`]. The scope and
    /// `self.depth` are restored before the closure's `Result` is returned, so
    /// a failed signature never leaks its parameters into the next one.
    pub(super) fn bind_params_then(
        &mut self,
        params: &[ast::Param],
        result: impl FnOnce(&mut Self) -> ElabResult<Type>,
    ) -> ElabResult<Type> {
        let saved_depth = self.depth;
        self.env.push_scope();
        let built = self.bind_params_and_fold(params, result);
        self.env.pop_scope();
        self.depth = saved_depth;
        built
    }

    fn bind_params_and_fold(
        &mut self,
        params: &[ast::Param],
        result: impl FnOnce(&mut Self) -> ElabResult<Type>,
    ) -> ElabResult<Type> {
        let mut bound = Vec::with_capacity(params.len());
        for param in params {
            let ty = self.elab_type(&param.ty)?;
            // A complex pattern is pass 2's error to report; here it binds nothing.
            let name = self
                .pattern_to_name(&param.pattern)
                .unwrap_or_else(|_| "_".to_string());
            self.env.bind_local(name.clone(), ty.clone(), self.depth);
            self.depth += 1;
            bound.push((name, ty));
        }
        let mut ty = result(self)?;
        // 30.9.26d D7's `Prop`-domain refusal is gone: `Prop` no longer
        // elaborates to a type at all (ADR 4.10.26i D3).
        for (name, param_ty) in bound.into_iter().rev() {
            ty = Type::pi_or_arrow(name, param_ty, ty);
        }
        Ok(ty)
    }
}
