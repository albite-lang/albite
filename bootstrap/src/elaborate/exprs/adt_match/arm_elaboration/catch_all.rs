//! Catch-all arms: a wildcard `_` and a variable name.
//!
//! Either one stands in for every constructor no arm names, so the case
//! binder it fills holds *that constructor's payload*. A wildcard reads none
//! of it. A variable must not read it either: the source says the name is
//! the whole matched value, so it is `let`-bound to the outermost scrutinee
//! at its declared type, under a fresh case binder (ADR 9.10.26e D1). Binding
//! the name as the case binder gave it the payload and the payload's type —
//! self-consistent Core the kernel could not tell from the source's meaning.

use crate::ast;
use crate::span::Spanned;
use albite_core::{Term, Type};

use crate::elaborate::error::ElabError;
use crate::elaborate::exprs::adt_match::context::CatchAllScrutinee;
use crate::elaborate::{ElabResult, Elaborator};

/// The constructor a catch-all arm stands in for, and the payload type its
/// case binder carries.
pub(super) struct CoveredCtor<'b> {
    pub name: &'b str,
    pub payload_ty: &'b Type,
}

impl CoveredCtor<'_> {
    /// The fresh case binder a catch-all arm takes for this constructor.
    fn case_binder(&self) -> String {
        format!("__catch_all_{}", self.name)
    }
}

/// What a match's variable catch-all binds, decided once per match.
pub(in crate::elaborate::exprs::adt_match) struct CatchAllPlan {
    /// The value the name holds; `None` without a variable catch-all.
    pub scrutinee: Option<CatchAllScrutinee>,
    /// The `__scrut_<n>` a non-variable scrutinee must be `let`-bound to
    /// first, so that it is evaluated once.
    pub hoisted: Option<String>,
}

/// Whether `term` is a variable under its span and annotation wrappers.
fn is_variable(term: &Term) -> bool {
    match term {
        Term::Var(_) => true,
        Term::Spanned(inner, _) | Term::Annot(inner, _) => is_variable(inner),
        _ => false,
    }
}

impl<'a> Elaborator<'a> {
    /// Decide what a variable catch-all binds (ADR 9.10.26e D1, D2).
    ///
    /// - no catch-all, or a wildcard: nothing, and the match is unchanged;
    /// - the arm a nested re-entry's outer match pushed down: the outer value;
    /// - a variable scrutinee: that variable;
    /// - anything else: a fresh `__scrut_<n>` the caller binds it to.
    pub(in crate::elaborate::exprs::adt_match) fn plan_catch_all_scrutinee(
        &mut self,
        catch_all: Option<&ast::MatchArm>,
        source: &Term,
        scrutinee_ty: &Type,
        outer: Option<&CatchAllScrutinee>,
    ) -> CatchAllPlan {
        let Some(arm) = catch_all.filter(|arm| matches!(arm.pattern, ast::Pattern::Var(_))) else {
            return CatchAllPlan {
                scrutinee: None,
                hoisted: None,
            };
        };
        if let Some(outer) = outer.filter(|outer| outer.arm_span == arm.span) {
            return CatchAllPlan {
                scrutinee: Some(outer.clone()),
                hoisted: None,
            };
        }
        let (term, hoisted) = if is_variable(source) {
            (source.clone(), None)
        } else {
            let name = self.fresh_var("scrut_");
            (Term::var(&name), Some(name))
        };
        CatchAllPlan {
            scrutinee: Some(CatchAllScrutinee {
                term,
                ty: scrutinee_ty.clone(),
                arm_span: arm.span,
            }),
            hoisted,
        }
    }

    /// Elaborate a wildcard catch-all pattern.
    /// Binds the payload to a fresh variable the body cannot name.
    pub(super) fn elab_wildcard_arm(
        &mut self,
        body: &ast::Expr,
        covered: &CoveredCtor<'_>,
        result_ty: Option<&Type>,
    ) -> ElabResult<(String, Term)> {
        let raw_var = covered.case_binder();
        self.with_scoped_binding(&raw_var, covered.payload_ty.clone(), |elab| {
            let body_term = elab.elab_arm_body(body, result_ty)?;
            Ok((raw_var.clone(), body_term))
        })
    }

    /// Elaborate a variable catch-all pattern.
    /// Binds the entire matched value to the variable name.
    ///
    /// `(__catch_all_<Ctor>, let <name> : <scrutinee type> = <scrutinee> in body)`
    pub(super) fn elab_var_arm(
        &mut self,
        body: &ast::Expr,
        covered: &CoveredCtor<'_>,
        var_name: &str,
        scrutinee: Option<&CatchAllScrutinee>,
        result_ty: Option<&Type>,
    ) -> ElabResult<(String, Term)> {
        // Defensive: `elab_adt_match` builds the scrutinee whenever the
        // classified catch-all is a variable, and only that arm reaches here.
        let Some(scrutinee) = scrutinee else {
            return Err(ElabError::internal(
                body.span(),
                "a variable catch-all reached arm elaboration without its scrutinee",
            ));
        };
        let raw_var = covered.case_binder();
        self.with_scoped_binding(&raw_var, covered.payload_ty.clone(), |elab| {
            elab.with_scoped_binding(var_name, scrutinee.ty.clone(), |elab| {
                let body_term = elab.elab_arm_body(body, result_ty)?;
                let bound = Term::let_in(
                    var_name,
                    scrutinee.ty.clone(),
                    scrutinee.term.clone(),
                    body_term,
                );
                Ok((raw_var.clone(), bound))
            })
        })
    }

    /// Check an arm body against the expected result type, or infer it.
    fn elab_arm_body(&mut self, body: &ast::Expr, result_ty: Option<&Type>) -> ElabResult<Term> {
        match result_ty {
            Some(expected) => self.check(body, expected),
            None => Ok(self.infer(body)?.0),
        }
    }
}
