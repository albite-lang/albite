//! A disjunction at the surface (ADR 7.10.26l D2, D3).
//!
//! A disjunction is a binary sum whose two sides sort `Prop`. `inl(e)` and
//! `inr(e)` build one against an expected disjunction, and `match h { inl(a)
//! => …, inr(b) => … }` takes one apart into the core's non-dependent
//! `Case`. Both are built in only where lookup fails: a value or constructor
//! named `inl` or `inr` in scope keeps today's meaning, so no program the
//! compiler accepted before changes meaning.
//!
//! A declared data type and a written sum are the same core type (§ 1.2), so
//! the forms are admitted on a sum that does not sort data, never on "a sum
//! that was written". The kernel types `Inl`, `Inr` and `Case` with the rules
//! it already has; nothing here is trusted.

use crate::ast::{self, Expr, Path, Pattern};
use crate::elaborate::env::ResolvedValue;
use crate::elaborate::error::{ElabError, ElabErrorKind, ExpectedContext, SortErrorKind};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};
use albite_core::types::Sort;
use albite_core::{Term, Type};

/// Which side of a disjunction a built-in form names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    /// The side `name` names: `inl` the left, `inr` the right.
    #[must_use]
    pub(crate) fn of_name(name: &str) -> Option<Side> {
        match name {
            "inl" => Some(Side::Left),
            "inr" => Some(Side::Right),
            _ => None,
        }
    }
}

/// Whether `ty` is a disjunction: a sum that sorts `Prop`.
#[must_use]
pub(crate) fn is_disjunction(ty: &Type) -> bool {
    matches!(ty, Type::Sum(..)) && ty.sort_of() == Some(Sort::Prop)
}

/// Whether `inl`/`inr` may build a value of `ty`: any sum that does not sort
/// data. A poisoned or deferred side sorts `None` and passes as transit, as
/// `binds_data` lets it (D2).
#[must_use]
fn admits_sum_intro(ty: &Type) -> bool {
    matches!(ty, Type::Sum(..)) && ty.sort_of() != Some(Sort::Type0)
}

/// The two arms of a disjunction `match`, by side, each with its binder.
struct DisjunctionArms<'m> {
    left: (&'m ast::MatchArm, String),
    right: (&'m ast::MatchArm, String),
}

impl Elaborator<'_> {
    /// The side `func` names when `func(args)` is the built-in `inl(e)` or
    /// `inr(e)`: a simple path `inl`/`inr` that resolves to no value or
    /// constructor, applied to one argument.
    fn builtin_sum_intro(&self, func: &Expr, args: &[Expr]) -> Option<Side> {
        let Expr::Path(path) = func else {
            return None;
        };
        let side = Side::of_name(&path.item_name().name).filter(|_| path.is_simple())?;
        let unresolved = matches!(
            self.env
                .resolve_value_path(path, self.depth, &self.current_module),
            Ok(None)
        );
        (unresolved && args.len() == 1).then_some(side)
    }

    /// `inl(e)` with no expected type: E0011, since the other side cannot be
    /// known (D2). `None` when `func(args)` is not the built-in.
    pub(in crate::elaborate::exprs) fn refuse_inferred_sum_intro(
        &self,
        func: &Expr,
        args: &[Expr],
        span: Span,
    ) -> Option<ElabError> {
        self.builtin_sum_intro(func, args)
            .map(|_| ElabError::new(span, ElabErrorKind::CannotInferType))
    }

    /// `inl(e)` or `inr(e)` against `expected`: check `e` against that side
    /// and emit `Inl(A + B, e)` or `Inr(A + B, e)` (D2). `None` when
    /// `func(args)` is not the built-in, so the caller takes today's path.
    pub(in crate::elaborate::exprs) fn check_sum_intro(
        &mut self,
        func: &Expr,
        args: &[Expr],
        expected: &Type,
    ) -> ElabResult<Option<Term>> {
        let Some(side) = self.builtin_sum_intro(func, args) else {
            return Ok(None);
        };
        let sum = self.normalize_for_comparison(expected);
        let (left, right) = match &sum {
            Type::Sum(left, right) if admits_sum_intro(&sum) => (left, right),
            // Poison passes through (ADR 15.8.26d): its fault is already reported.
            Type::Error => return Ok(Some(Term::Sorry)),
            _ => return Err(sum_intro_refusal(func, side, expected)),
        };
        Ok(Some(match side {
            Side::Left => Term::inl(sum.clone(), self.check(&args[0], left)?),
            Side::Right => Term::inr(sum.clone(), self.check(&args[0], right)?),
        }))
    }

    /// `match h { inl(a) => e₁, inr(b) => e₂ }` on a disjunction, lowered to
    /// `Case(h, a, e₁, b, e₂)` (D3). Non-dependent: each arm is checked
    /// against `expected` as written. `None` when the scrutinee is not a
    /// disjunction or the arms are not exactly `inl` and `inr`, each binding
    /// a name or `_` — every other shape keeps today's route and error.
    pub(in crate::elaborate::exprs) fn try_elab_disjunction_match(
        &mut self,
        scrutinee: &Term,
        scrutinee_ty: &Type,
        arms: &[ast::MatchArm],
        expected: Option<&Type>,
    ) -> ElabResult<Option<(Term, Type)>> {
        let sum = self.normalize_for_comparison(scrutinee_ty);
        let Type::Sum(left_ty, right_ty) = &sum else {
            return Ok(None);
        };
        if !is_disjunction(&sum) {
            return Ok(None);
        }
        let Some(arms) = self.disjunction_arms(arms) else {
            return Ok(None);
        };
        let (left_arm, left_var) = arms.left;
        let (right_arm, right_var) = arms.right;
        let (left_body, result_ty) =
            self.elab_disjunction_arm(left_arm, &left_var, left_ty, expected)?;
        self.push_context(ExpectedContext::branch_unification(left_arm.body.span()));
        let right = self.elab_disjunction_arm(right_arm, &right_var, right_ty, Some(&result_ty));
        self.pop_context();
        let (right_body, _) = right?;
        let term = Term::case(
            scrutinee.clone(),
            left_var,
            left_body,
            right_var,
            right_body,
        );
        Ok(Some((term, result_ty)))
    }

    /// The arms as a disjunction's, or `None`: exactly two, unguarded, one
    /// `inl(p)` and one `inr(q)` in either order, neither name a
    /// constructor in scope, each `p` a name or `_`.
    fn disjunction_arms<'m>(&self, arms: &'m [ast::MatchArm]) -> Option<DisjunctionArms<'m>> {
        let [first, second] = arms else {
            return None;
        };
        let (first_side, first_var) = self.disjunction_arm(first)?;
        let (second_side, second_var) = self.disjunction_arm(second)?;
        match (first_side, second_side) {
            (Side::Left, Side::Right) => Some(DisjunctionArms {
                left: (first, first_var),
                right: (second, second_var),
            }),
            (Side::Right, Side::Left) => Some(DisjunctionArms {
                left: (second, second_var),
                right: (first, first_var),
            }),
            _ => None,
        }
    }

    /// One arm's side and binder, when it is `inl(p)` or `inr(p)`.
    fn disjunction_arm(&self, arm: &ast::MatchArm) -> Option<(Side, String)> {
        let Pattern::Constructor(path, fields, _) = &arm.pattern else {
            return None;
        };
        let side = Side::of_name(&path.item_name().name).filter(|_| path.is_simple())?;
        if arm.guard.is_some() || self.names_a_constructor(path) {
            return None;
        }
        match fields.as_slice() {
            [Pattern::Var(ident)] => Some((side, ident.name.clone())),
            [Pattern::Wildcard(_)] => Some((side, "_".to_string())),
            _ => None,
        }
    }

    /// Whether `path` resolves to a user constructor, which wins (D3).
    fn names_a_constructor(&self, path: &Path) -> bool {
        matches!(
            self.env
                .resolve_value_path(path, self.depth, &self.current_module),
            Ok(Some(ResolvedValue::Constructor(_)))
        )
    }

    /// One arm's body with `var : side` bound, checked against `expected`
    /// or inferred; returns the body and its type.
    fn elab_disjunction_arm(
        &mut self,
        arm: &ast::MatchArm,
        var: &str,
        side: &Type,
        expected: Option<&Type>,
    ) -> ElabResult<(Term, Type)> {
        self.env.push_scope();
        self.env
            .bind_local(var.to_string(), side.clone(), self.depth);
        self.depth += 1;
        let body = match expected {
            Some(expected) => self
                .check(&arm.body, expected)
                .map(|t| (t, expected.clone())),
            None => self.infer(&arm.body),
        };
        self.depth -= 1;
        self.env.pop_scope();
        body
    }
}

/// E0024 at `func`: `inl`/`inr` against a type that is not a disjunction.
fn sum_intro_refusal(func: &Expr, side: Side, expected: &Type) -> ElabError {
    let ctor = match side {
        Side::Left => "inl",
        Side::Right => "inr",
    };
    ElabError::new(
        func.span(),
        ElabErrorKind::Sort(SortErrorKind::SumIntroNotDisjunction {
            ctor: ctor.to_string(),
            expected: expected.clone(),
        }),
    )
}
