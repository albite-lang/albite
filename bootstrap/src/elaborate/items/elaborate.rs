//! Pass 2: Definition Elaboration
//!
//! Elaborates top-level items to Core definitions by building
//! Core terms from item bodies.

use crate::ast;
use crate::span::Spanned;

use albite_core::terms::{SpannedTerm, TermSpan};
use albite_core::Term;

use crate::elaborate::error::{ElabError, ElabErrorKind, ExpectedContext};
use crate::elaborate::kernel::{KernelObligation, ObligationOrigin};
use crate::elaborate::tactics::TacticTrace;
use crate::elaborate::types::kinds::kinds_from_signature;
use crate::elaborate::{CoreDef, ElabResult, Elaborator};

/// `Λtype_params. term`, and, when `signature` binds a predicate, the
/// annotation that introduces it: `Annot(ΛP. term, ∀P : Ds → Prop. B)` (ADR
/// 6.10.26h D5). A bare `TyAbs` binds `P` kind-less in the kernel, so only
/// the annotation carries the kind there.
fn abstract_type_params(
    term: Term,
    type_params: &[ast::TypeParam],
    signature: &albite_core::Type,
) -> Term {
    let abstracted = type_params.iter().rev().fold(term, |body, tp| {
        Term::TyAbs(tp.name.name.clone(), Box::new(body))
    });
    if signature.has_predicate_prefix() {
        Term::annot(abstracted, signature.clone())
    } else {
        abstracted
    }
}

impl<'a> Elaborator<'a> {
    pub(super) fn elaborate_function(&mut self, func: &ast::FunctionDef) -> ElabResult<CoreDef> {
        // Get the type we computed in pass 1
        let name = self.top_level_core_name(&func.name.name);
        let span = func.name.span;
        let func_ty = self
            .env
            .lookup_value(&name)
            .ok_or_else(|| ElabError::new(span, ElabErrorKind::UndefinedVariable(name.clone())))?
            .ty
            .clone();

        // Build the term: Λα. ... λ(x:T). body, the type parameters popped
        // on every path (ADR 6.10.26b D1)
        let term = self.with_type_params(&func.type_params, &func_ty, |elab| {
            elab.elaborate_function_body(func)
        })?;
        let term = abstract_type_params(term, &func.type_params, &func_ty);

        // A `fn` that can hand back a proof is cited by the kernel at its
        // type, so its body is checked as a term-mode theorem's is (ADR
        // 13.10.26t D1). A `#[partial]` one is refused when cited (E0063).
        if !func.attrs.partial && self.carries_proof(&func_ty) {
            self.kernel_obligations.push(KernelObligation {
                def: name.clone(),
                span: func.span,
                module: self.get_current_file(),
                origin: ObligationOrigin::Term,
            });
        }

        Ok(CoreDef {
            name,
            ty: func_ty,
            term: SpannedTerm::new(term, TermSpan::new(func.span.start, func.span.end)),
            span: func.span,
        })
    }

    /// [`super::carries_proof`] against this elaborator's encodings.
    fn carries_proof(&self, ty: &albite_core::Type) -> bool {
        super::carries_proof(ty, |name| {
            self.env
                .types
                .get(name)
                .and_then(|def| def.encoded_type.as_ref())
        })
    }

    fn elaborate_function_body(&mut self, func: &ast::FunctionDef) -> ElabResult<Term> {
        self.env.push_scope();

        // Bind all parameters
        let mut param_types = Vec::new();
        for param in &func.params {
            let name = self.pattern_to_name(&param.pattern)?;
            let ty = self.elab_type(&param.ty)?;
            self.env.bind_local(name.clone(), ty.clone(), self.depth);
            self.depth += 1;
            param_types.push((name, ty));
        }

        // Elaborate body
        let body_term = if let Some(ref ret_ty) = func.return_type {
            let expected = self.elab_type(ret_ty)?;
            // Push context for better error messages
            self.push_context(ExpectedContext::return_type(ret_ty.span()));
            let result = self.with_return_context(Some(expected.clone()), |elab| {
                elab.check(&func.body, &expected)
            });
            self.pop_context();
            result?
        } else {
            self.infer(&func.body)?.0
        };

        // Build nested lambdas
        let mut term = body_term;
        for (name, ty) in param_types.into_iter().rev() {
            self.depth -= 1;
            term = Term::lambda(name, ty, term);
        }

        self.env.pop_scope();
        Ok(term)
    }

    pub(super) fn elaborate_theorem(&mut self, thm: &ast::TheoremDef) -> ElabResult<CoreDef> {
        // Get type from pass 1
        let name = self.top_level_core_name(&thm.name.name);
        let span = thm.name.span;
        let thm_ty = self
            .env
            .lookup_value(&name)
            .ok_or_else(|| ElabError::new(span, ElabErrorKind::UndefinedVariable(name.clone())))?
            .ty
            .clone();

        // Elaborate theorem body (proof); a `by` block also yields its trace
        let (term, trace) = self.with_type_params(&thm.type_params, &thm_ty, |elab| {
            elab.elaborate_theorem_body(thm)
        })?;

        // Every proof is recorded for the kernel: a tactic proof with its trace
        // (ADR 2.10.26c D5), a term-mode one bare (ADR 2.10.26d D1) — never
        // both, so a theorem is never reported twice.
        let origin = match trace {
            Some(mut trace) => {
                trace.type_params = thm
                    .type_params
                    .iter()
                    .map(|tp| tp.name.name.clone())
                    .collect();
                ObligationOrigin::Tactics(trace)
            }
            None => ObligationOrigin::Term,
        };
        self.kernel_obligations.push(KernelObligation {
            def: name.clone(),
            span: thm.span,
            module: self.get_current_file(),
            origin,
        });

        let wrapped_term = abstract_type_params(term, &thm.type_params, &thm_ty);

        Ok(CoreDef {
            name,
            ty: thm_ty,
            term: SpannedTerm::new(wrapped_term, TermSpan::new(thm.span.start, thm.span.end)),
            span: thm.span,
        })
    }

    /// The theorem's proof under its parameters' scope, which is popped and
    /// whose depth is restored on every path: an error leaving it open put a
    /// failed theorem's hypotheses in scope for the next (ADR 3.10.26e).
    fn elaborate_theorem_body(
        &mut self,
        thm: &ast::TheoremDef,
    ) -> ElabResult<(Term, Option<TacticTrace>)> {
        let saved_depth = self.depth;
        let saved_proof_body = std::mem::replace(&mut self.in_proof_body, true);
        self.env.push_scope();
        let result = self.elaborate_theorem_in_scope(thm);
        self.env.pop_scope();
        self.depth = saved_depth;
        self.in_proof_body = saved_proof_body;
        result
    }

    fn elaborate_theorem_in_scope(
        &mut self,
        thm: &ast::TheoremDef,
    ) -> ElabResult<(Term, Option<TacticTrace>)> {
        // Bind all parameters (hypotheses)
        let mut param_types = Vec::new();
        for param in &thm.params {
            let name = self.pattern_to_name(&param.pattern)?;
            let ty = self.elab_type(&param.ty)?;
            self.env.bind_local(name.clone(), ty.clone(), self.depth);
            self.depth += 1;
            param_types.push((name, ty));
        }

        // Elaborate proof body against the proposition
        let expected = self.elab_type(&thm.prop)?;
        let (body_term, trace) = match &thm.body {
            ast::TheoremBody::Term(body) => (self.check(body, &expected)?, None),
            ast::TheoremBody::Tactics(block) => {
                let (term, mut trace) = self.elaborate_tactic_block(block, &expected)?;
                trace.value_params.clone_from(&param_types);
                (term, Some(trace))
            }
        };

        // Build nested lambdas for parameters
        let mut term = body_term;
        for (name, ty) in param_types.into_iter().rev() {
            term = Term::lambda(name, ty, term);
        }
        Ok((term, trace))
    }

    pub(super) fn elaborate_axiom(&mut self, axiom: &ast::AxiomDef) -> ElabResult<CoreDef> {
        // Get type from pass 1
        let name = self.top_level_core_name(&axiom.name.name);
        let span = axiom.name.span;
        let axiom_ty = self
            .env
            .lookup_value(&name)
            .ok_or_else(|| ElabError::new(span, ElabErrorKind::UndefinedVariable(name.clone())))?
            .ty
            .clone();

        // Axioms have no proof - they use sorry
        // Build: Λα. ... λ(h:P). sorry
        // The hole is the author's, so it carries the item's span (ADR 18.9.26g).
        let mut term = Term::spanned(Term::Sorry, TermSpan::new(axiom.span.start, axiom.span.end));

        let binder_names = axiom_binder_names(&axiom_ty, axiom.params.len());
        let param_types = self.with_type_params(&axiom.type_params, &axiom_ty, |elab| {
            elab.elaborate_axiom_params(axiom, binder_names)
        })?;
        for (binder, ty) in param_types.into_iter().rev() {
            term = Term::lambda(binder, ty, term);
        }
        let term = abstract_type_params(term, &axiom.type_params, &axiom_ty);

        Ok(CoreDef {
            name,
            ty: axiom_ty,
            term: SpannedTerm::new(term, TermSpan::new(axiom.span.start, axiom.span.end)),
            span: axiom.span,
        })
    }

    /// Bind an axiom's parameters left to right, as `elaborate_theorem_body`
    /// does, so a hypothesis over an earlier parameter elaborates (ADR
    /// 30.9.26d D2). Each comes back under its λ binder name.
    fn elaborate_axiom_params(
        &mut self,
        axiom: &ast::AxiomDef,
        binder_names: Vec<String>,
    ) -> ElabResult<Vec<(String, albite_core::Type)>> {
        let saved_depth = self.depth;
        self.env.push_scope();
        let mut param_types = Vec::new();
        for (param, binder) in axiom.params.iter().zip(binder_names) {
            let name = self.pattern_to_name(&param.pattern)?;
            let ty = self.elab_type(&param.ty)?;
            self.env.bind_local(name, ty.clone(), self.depth);
            self.depth += 1;
            param_types.push((binder, ty));
        }
        self.env.pop_scope();
        self.depth = saved_depth;
        Ok(param_types)
    }

    /// Run `body` with `type_params` in scope as type variables, popping them
    /// on every path, the error path included: a leaked parameter would turn a
    /// later application of a same-named type into a spurious refusal (ADR
    /// 6.10.26b D1). Each takes the kind pass 1 collected into `signature`
    /// (ADR 6.10.26h D2), so a kind is read, and refused, once.
    fn with_type_params<R>(
        &mut self,
        type_params: &[ast::TypeParam],
        signature: &albite_core::Type,
        body: impl FnOnce(&mut Self) -> ElabResult<R>,
    ) -> ElabResult<R> {
        let kinds = kinds_from_signature(signature, type_params.len());
        for (tp, kind) in type_params.iter().zip(kinds) {
            self.env.push_kinded_type_var(tp.name.name.clone(), kind);
        }
        let result = body(self);
        for _ in type_params {
            self.env.pop_type_var();
        }
        result
    }
}

/// The λ binder each axiom parameter gets: its own name where pass 1 folded
/// it into a `Pi`, whose binder a `_` could not match, and `_` otherwise, so
/// every non-dependent axiom keeps its Core (ADR 30.9.26d D2).
fn axiom_binder_names(axiom_ty: &albite_core::Type, arity: usize) -> Vec<String> {
    use albite_core::Type;
    let mut layer = axiom_ty;
    while let Type::Forall(_, body) | Type::ForallPred(_, _, body) = layer {
        layer = body;
    }
    let mut names = Vec::with_capacity(arity);
    for _ in 0..arity {
        match layer {
            Type::Pi(binder, _, cod) => {
                names.push(binder.clone());
                layer = cod;
            }
            Type::Arrow(_, cod) => {
                names.push("_".to_string());
                layer = cod;
            }
            _ => names.push("_".to_string()),
        }
    }
    names
}
