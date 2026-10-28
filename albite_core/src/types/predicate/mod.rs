//! Predicate binders: `PredApp`, `ForallPred` and `PredAbs` (ADR 6.10.26h D4).
//!
//! `∀P : A₁ → … → Aₙ → Prop. τ` is instantiated only at a predicate
//! `λx₁:A₁ … xₙ:Aₙ. T`, and the instantiation is *hereditary*: every
//! `P<t₁, …, tₙ>` becomes `T[xs := ts]` at once, so no type-level redex is
//! ever left for conversion to reduce. The substitution of the `xs` is
//! simultaneous and capture-avoiding — each `xᵢ` is first renamed to a name
//! free in none of the `ts`, then [`Type::substitute_term`] renames any binder
//! of `T` that would capture.

use std::collections::HashSet;

use super::{TyVar, Type};
use crate::terms::substitution::capture::fresh_name;
use crate::terms::{Term, Var};

impl Type {
    /// `P<args…>`.
    #[must_use]
    pub fn pred_app(var: impl Into<String>, args: Vec<Term>) -> Type {
        Type::PredApp(var.into(), args)
    }

    /// `∀P : domains → Prop. body`.
    #[must_use]
    pub fn forall_pred(var: impl Into<String>, domains: Vec<Type>, body: Type) -> Type {
        Type::ForallPred(var.into(), domains, Box::new(body))
    }

    /// `λparams. body`, an instantiating predicate.
    #[must_use]
    pub fn pred_abs(params: Vec<(Var, Type)>, body: Type) -> Type {
        Type::PredAbs(params, Box::new(body))
    }

    /// [`Type::substitute`]'s arm for the three predicate forms.
    ///
    /// # Panics
    ///
    /// When `var` names a `PredApp`'s variable and `replacement` is not a
    /// predicate (`PredAbs`), a renaming (`TyVar`) or poison: that would put
    /// a data type where a proposition was applied. The kernel's `TyApp`
    /// rule refuses every such instantiation first (D6), so a checked term
    /// never reaches it.
    pub(super) fn substitute_predicate_form(&self, var: &str, replacement: &Type) -> Type {
        match self {
            Type::PredApp(pred, args) => {
                let args: Vec<Term> = args
                    .iter()
                    .map(|a| a.substitute_type(var, replacement))
                    .collect();
                if pred != var {
                    return Type::pred_app(pred.clone(), args);
                }
                replace_predicate_variable(pred, &args, replacement)
            }
            Type::ForallPred(pred, domains, body) => {
                let domains = domains
                    .iter()
                    .map(|d| d.substitute(var, replacement))
                    .collect();
                let body = if pred == var {
                    body.as_ref().clone()
                } else {
                    body.substitute(var, replacement)
                };
                Type::forall_pred(pred.clone(), domains, body)
            }
            Type::PredAbs(params, body) => Type::pred_abs(
                params
                    .iter()
                    .map(|(x, d)| (x.clone(), d.substitute(var, replacement)))
                    .collect(),
                body.substitute(var, replacement),
            ),
            Type::Error => Type::Error,
            _ => unreachable!("substitute_predicate_form on a non-predicate form"),
        }
    }

    /// [`Type::free_type_vars`]'s arm for the three predicate forms. A
    /// `PredApp`'s variable counts, so a binder's scope is visible to
    /// callers that rename or close over type variables.
    pub(super) fn predicate_form_free_type_vars(&self) -> HashSet<TyVar> {
        let mut set = HashSet::new();
        match self {
            Type::PredApp(pred, args) => {
                set.insert(pred.clone());
                for arg in args {
                    set.extend(arg.free_type_vars());
                }
            }
            Type::ForallPred(pred, domains, body) => {
                set.extend(body.free_type_vars());
                set.remove(pred);
                for domain in domains {
                    set.extend(domain.free_type_vars());
                }
            }
            Type::PredAbs(params, body) => {
                set.extend(body.free_type_vars());
                for (_, domain) in params {
                    set.extend(domain.free_type_vars());
                }
            }
            // Poison has no free variables; nothing else is dispatched here.
            Type::Error => {}
            _ => unreachable!("predicate_form_free_type_vars on a non-predicate form"),
        }
        set
    }

    /// [`Type::is_well_formed`]'s arm for the three predicate forms. A
    /// predicate variable is not a type variable, so it is never in
    /// `type_vars`: the kernel resolves it against its context's
    /// `with_pred_var` bindings (`typecheck/eq_witnesses.rs`, D6), as it
    /// types `Eq` witnesses there. So a `PredApp` is well formed here.
    pub(super) fn predicate_form_well_formed(&self, type_vars: &HashSet<TyVar>) -> bool {
        self.children()
            .into_iter()
            .all(|child| child.is_well_formed(type_vars))
    }

    /// The source-like spelling of a predicate form, rendering each child
    /// type with `child`: `P<t, u>`, `∀P: Nat → Prop. τ`, `λ(x: Nat). τ`.
    pub(super) fn render_predicate_form(&self, child: impl Fn(&Type) -> String) -> String {
        match self {
            Type::PredApp(pred, args) => {
                let args: Vec<String> = args.iter().map(ToString::to_string).collect();
                format!("{pred}<{}>", args.join(", "))
            }
            Type::ForallPred(pred, domains, body) => {
                let domains: Vec<String> = domains.iter().map(&child).collect();
                format!("∀{pred}: {} → Prop. {}", domains.join(" → "), child(body))
            }
            Type::PredAbs(params, body) => {
                let params: Vec<String> = params
                    .iter()
                    .map(|(x, domain)| format!("{x}: {}", child(domain)))
                    .collect();
                format!("λ({}). {}", params.join(", "), child(body))
            }
            Type::Error => "<type error>".to_string(),
            _ => unreachable!("render_predicate_form on a non-predicate form"),
        }
    }

    /// Whether the prenex prefix of `Forall`s holds a `ForallPred`: the
    /// signatures introduced by annotation (D5).
    #[must_use]
    pub fn has_predicate_prefix(&self) -> bool {
        match self {
            Type::ForallPred(..) => true,
            Type::Forall(_, body) => body.has_predicate_prefix(),
            _ => false,
        }
    }

    /// Whether a `PredApp`, `ForallPred` or `PredAbs` occurs anywhere in
    /// `self`: the kernel's well-formedness walk runs in every mode only
    /// for these (D6).
    #[must_use]
    pub fn mentions_predicate_form(&self) -> bool {
        matches!(
            self,
            Type::PredApp(..) | Type::ForallPred(..) | Type::PredAbs(..)
        ) || self
            .children()
            .into_iter()
            .any(Type::mentions_predicate_form)
    }
}

/// Two `PredAbs` bodies with each binder pair renamed to one shared name,
/// ready for a syntactic comparison — [`Type::Pi`]'s rule over a list (D6:
/// `PredAbs` compares up to α). `None` when the arities differ.
pub(crate) fn pred_abs_bodies_unified(
    (left_params, left_body): (&[(Var, Type)], &Type),
    (right_params, right_body): (&[(Var, Type)], &Type),
) -> Option<(Type, Type)> {
    if left_params.len() != right_params.len() {
        return None;
    }
    let mut bodies = (left_body.clone(), right_body.clone());
    for ((left, _), (right, _)) in left_params.iter().zip(right_params) {
        bodies = crate::terms::substitution::capture::pi_codomains_unified(
            (left, &bodies.0),
            (right, &bodies.1),
        );
    }
    Some(bodies)
}

/// `P<args>[P := replacement]`: hereditary at a predicate, a rename at a
/// variable, poison at poison; anything else is the invariant violation
/// [`Type::substitute_predicate_form`] documents.
fn replace_predicate_variable(pred: &str, args: &[Term], replacement: &Type) -> Type {
    match replacement {
        Type::PredAbs(params, body) => instantiate_predicate(params, body, args),
        Type::TyVar(renamed) => Type::pred_app(renamed.clone(), args.to_vec()),
        Type::Error => Type::Error,
        other => panic!("predicate variable `{pred}` instantiated at a non-predicate `{other}`"),
    }
}

/// `body[params := args]`, simultaneously and capture-avoiding (D4).
///
/// The arity is the kernel's to check (D6); a mismatch here substitutes the
/// common prefix, which only an unchecked term can reach.
fn instantiate_predicate(params: &[(Var, Type)], body: &Type, args: &[Term]) -> Type {
    let mut taken: HashSet<String> = body.free_term_vars();
    for arg in args {
        taken.extend(arg.free_vars());
    }
    taken.extend(params.iter().map(|(x, _)| x.clone()));
    // Rename every parameter apart from the arguments first, so substituting
    // one argument can never be re-substituted by the next parameter.
    let mut renamed = body.clone();
    let mut fresh_params = Vec::with_capacity(params.len());
    for (param, _) in params {
        let fresh = fresh_name(param, &taken);
        taken.insert(fresh.clone());
        renamed = renamed.substitute_term(param, &Term::Var(fresh.clone()));
        fresh_params.push(fresh);
    }
    for (fresh, arg) in fresh_params.iter().zip(args) {
        renamed = renamed.substitute_term(fresh, arg);
    }
    renamed
}

#[cfg(test)]
mod tests;
