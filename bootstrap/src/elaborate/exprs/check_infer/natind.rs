//! `natind` and `natrec` elaboration (ADR 22.5.26a).
//!
//! `natind(motive, base, step, n)` — natural number induction, with a
//! dependent motive instantiated at each case (ADR 29.9.26d).
//! `natrec(type, base, step, n)` — natural number primitive recursion.

use albite_core::{Term, Type};

use crate::ast::{Expr, LambdaParam, Motive, TypeExpr};
use crate::elaborate::error::ElabError;
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};

impl Elaborator<'_> {
    /// Elaborate `natind(motive, base, step, n)` (ADR 29.9.26d).
    ///
    /// Motive must be `|k: Nat| P`. A motive is *dependent* when `k` occurs in
    /// `P`: then the base is checked against `P[k := zero]`, the step must be
    /// the literal `fn(j: Nat, ih) => b` with `ih : P[k := j]` and
    /// `b : P[k := succ j]`, and each case meets its instance through
    /// `check_case_against` (D5). A non-dependent motive keeps the path it
    /// always had: base against `P`, step against `Nat → P → P`. Either way
    /// the result type is `P[k := n]`.
    pub(crate) fn infer_natind(
        &mut self,
        motive: &Motive,
        base_expr: &Expr,
        step_expr: &Expr,
        n_expr: &Expr,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        let (binder, body) = self.elab_natind_motive(motive, span)?;

        let (base_term, step_term) = if motive_is_dependent(&binder, &body) {
            let base_goal = motive_instance(&binder, &body, &Term::Zero);
            let base_term = self.check_case_against(base_expr, &base_goal)?;
            (
                base_term,
                self.elab_dependent_step(step_expr, &binder, &body)?,
            )
        } else {
            let step_ty = Type::arrow(Type::Nat, Type::arrow(body.clone(), body.clone()));
            (
                self.check(base_expr, &body)?,
                self.check(step_expr, &step_ty)?,
            )
        };

        let (n_term, n_ty) = self.infer(n_expr)?;
        if !self.types_equal(&n_ty, &Type::Nat) {
            return Err(ElabError::type_mismatch(n_expr.span(), Type::Nat, n_ty));
        }

        let result_ty = motive_instance(&binder, &body, &n_term);
        let motive_type = Type::arrow(Type::Nat, body);
        let term = Term::natind(binder, motive_type, base_term, step_term, n_term);
        Ok((term, result_ty))
    }

    /// A dependent motive's step `fn(j: Nat, ih[: T]) => b` (D4): bind
    /// `j : Nat` and `ih : P[k := j]`, then check `b` against `P[k := succ j]`.
    /// The Core is the same `λj:Nat. λih:P[k := j]. b` that `check` builds.
    fn elab_dependent_step(&mut self, step: &Expr, binder: &str, body: &Type) -> ElabResult<Term> {
        let Some((index_param, hypothesis_param, proof)) = dependent_step_parts(step) else {
            let (_, found) = self.infer(step)?;
            let expected = Type::arrow(
                Type::Nat,
                Type::arrow(body.clone(), successor_instance(binder, body, binder)),
            );
            return Err(ElabError::type_mismatch(step.span(), expected, found)
                .with_note("a dependent motive's step must be written fn(k: Nat, ih) => …"));
        };
        let index = self.pattern_to_name(&index_param.pattern)?;
        let hypothesis = self.pattern_to_name(&hypothesis_param.pattern)?;
        if let Some(shadowed) = reverse_capture(binder, body, &[&index, &hypothesis]) {
            let conflated = motive_instance(binder, body, &Term::var(shadowed.clone()));
            return Err(ElabError::type_mismatch(step.span(), body.clone(), conflated)
                .with_note(format!(
                    "the step's parameter `{shadowed}` shadows `{shadowed}`, which the motive mentions; rename it"
                )));
        }
        if let Some(ann) = &index_param.ty {
            let ann_ty = self.elab_type(ann)?;
            if !self.types_equal(&ann_ty, &Type::Nat) {
                return Err(ElabError::type_mismatch(ann.span(), Type::Nat, ann_ty));
            }
        }

        let hypothesis_ty = motive_instance(binder, body, &Term::var(index.clone()));
        let successor_goal = successor_instance(binder, body, &index);
        let proof_term = self.with_scoped_binding(&index, Type::Nat, |elab| {
            elab.elab_step_body(
                hypothesis_param,
                &hypothesis,
                &hypothesis_ty,
                proof,
                successor_goal,
            )
        })?;
        Ok(Term::lambda(
            index,
            Type::Nat,
            Term::lambda(hypothesis, hypothesis_ty, proof_term),
        ))
    }

    /// Inside the step's `j` scope: check a written IH annotation against its
    /// instance, bind the IH, and check the body against the successor goal.
    fn elab_step_body(
        &mut self,
        hypothesis_param: &LambdaParam,
        hypothesis: &str,
        hypothesis_ty: &Type,
        proof: &Expr,
        successor_goal: Type,
    ) -> ElabResult<Term> {
        if let Some(ann) = &hypothesis_param.ty {
            let ann_ty = self.elab_type(ann)?;
            if !self.types_equal(&ann_ty, hypothesis_ty) {
                return Err(ElabError::type_mismatch(
                    ann.span(),
                    hypothesis_ty.clone(),
                    ann_ty,
                ));
            }
        }
        self.with_scoped_binding(hypothesis, hypothesis_ty.clone(), |elab| {
            elab.check_case_against(proof, &successor_goal)
        })
    }

    /// Elaborate `natrec(type, base, step, n)`.
    ///
    /// Result type T. Base : T. Step : Nat → T → T.
    pub(crate) fn infer_natrec(
        &mut self,
        result_type_expr: &TypeExpr,
        base_expr: &Expr,
        step_expr: &Expr,
        n_expr: &Expr,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        // 1. Elaborate result type
        let result_ty = self.elab_type(result_type_expr)?;

        // 2. Elaborate base case — check against T
        let base_term = self.check(base_expr, &result_ty)?;

        // 3. Elaborate step case — Nat → T → T
        let step_ty = Type::Arrow(
            Box::new(Type::Nat),
            Box::new(Type::Arrow(
                Box::new(result_ty.clone()),
                Box::new(result_ty.clone()),
            )),
        );
        let step_term = self.check(step_expr, &step_ty)?;

        // 4. Elaborate target — must be Nat
        let (n_term, n_ty) = self.infer(n_expr)?;
        if !self.types_equal(&n_ty, &Type::Nat) {
            return Err(ElabError::type_mismatch(n_expr.span(), Type::Nat, n_ty));
        }

        // 5. Produce Term::NatRec
        let term = Term::natrec(result_ty.clone(), base_term, step_term, n_term);

        Ok((term, result_ty))
    }

    /// Elaborate a motive for `natind`, validating that the domain is `Nat`.
    /// Returns the binder and the body (ADR 29.9.26d D2).
    fn elab_natind_motive(&mut self, motive: &Motive, span: Span) -> ElabResult<(String, Type)> {
        match motive {
            Motive::Lambda(param, body, _motive_span) => {
                let param_name = self.pattern_to_name(&param.pattern)?;

                let param_ty = match &param.ty {
                    Some(ann) => {
                        let ann_ty = self.elab_type(ann)?;
                        if !self.types_equal(&ann_ty, &Type::Nat) {
                            return Err(ElabError::natind_motive_not_nat(ann.span(), ann_ty));
                        }
                        ann_ty
                    }
                    None => {
                        return Err(ElabError::motive_not_predicate(param.span, Type::Unit));
                    }
                };

                self.env.push_scope();
                self.env
                    .bind_local(param_name.clone(), param_ty, self.depth);
                self.depth += 1;

                let body_ty = self
                    .elab_type(body)
                    .map_err(|_| ElabError::motive_body_not_type(body.span()));

                self.depth -= 1;
                self.env.pop_scope();

                body_ty.map(|body_ty| (param_name, body_ty))
            }
            Motive::Expr(expr) => {
                let (_, expr_ty) = self.infer(expr).unwrap_or((Term::Sorry, Type::Unit));
                Err(ElabError::motive_not_predicate(expr.span(), expr_ty))
            }
        }
    }
}

// ─── Pure helpers over the motive (ADR 29.9.26d AC 6) ──────────────────────

/// A motive `k. P` is dependent when `k` occurs free in `P` — structurally,
/// only inside an `Eq` argument, since no other `Type` variant holds a term.
/// An inner `natind` that rebinds `k` does not count (D3).
pub(crate) fn motive_is_dependent(binder: &str, body: &Type) -> bool {
    body.free_term_vars().contains(binder)
}

/// `P[k := replacement]`, capture-avoiding (D3).
pub(crate) fn motive_instance(binder: &str, body: &Type, replacement: &Term) -> Type {
    body.substitute_term(binder, replacement)
}

/// `P[k := succ(j)]`: the goal of a step whose index is `j`.
pub(crate) fn successor_instance(binder: &str, body: &Type, index: &str) -> Type {
    motive_instance(binder, body, &Term::succ(Term::var(index)))
}

/// The first of `step_names` that the motive mentions other than as its
/// binder (D4's reverse capture): `P[k := j]` would conflate the step's `j`
/// with that outer one.
pub(crate) fn reverse_capture(binder: &str, body: &Type, step_names: &[&str]) -> Option<String> {
    let mut outer = body.free_term_vars();
    outer.remove(binder);
    step_names
        .iter()
        .find(|name| outer.contains(**name))
        .map(|name| (*name).to_owned())
}

/// The parts of a literal two-parameter step `fn(j[: Nat], ih[: T]) => b`,
/// seen through parentheses; `None` for any other step expression (D4).
pub(crate) fn dependent_step_parts(step: &Expr) -> Option<(&LambdaParam, &LambdaParam, &Expr)> {
    match step {
        Expr::Paren(inner, _) => dependent_step_parts(inner),
        Expr::Lambda(params, body, _) if params.len() == 2 => Some((&params[0], &params[1], body)),
        _ => None,
    }
}
