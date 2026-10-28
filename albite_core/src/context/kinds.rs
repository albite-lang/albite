//! Predicate variables in the context (ADR 6.10.26h D6).
//!
//! `with_pred_var(P, [A₁, …, Aₙ])` binds `P : A₁ → … → Aₙ → Prop`. It is
//! not a type variable — `has_type_var(P)` stays false, so `P` used bare as a
//! type is malformed — and a `PredApp(P, …)` resolves only against it, so a
//! `with_type_var(P)` binding never licenses one.

use super::{Binding, Context};
use crate::types::Type;

impl Context {
    /// Extend context with a predicate variable: Γ, P : domains → Prop
    #[must_use]
    pub fn with_pred_var(&self, var: impl Into<String>, domains: Vec<Type>) -> Self {
        let mut ctx = self.clone();
        ctx.bindings.push(Binding::PredVar(var.into(), domains));
        ctx
    }

    /// The domains of the most recent binding of `var` when it is a
    /// predicate variable; `None` when it is unbound or most recently bound
    /// as a type variable.
    #[must_use]
    pub fn lookup_pred_var(&self, var: &str) -> Option<&[Type]> {
        self.bindings
            .iter()
            .rev()
            .find_map(|binding| match binding {
                Binding::PredVar(p, domains) if p == var => Some(Some(domains.as_slice())),
                Binding::TypeVar(v) if v == var => Some(None),
                _ => None,
            })
            .flatten()
    }
}

/// `P : Nat → Nat → Prop`, for the context's `Display`.
pub(super) fn render_pred_var(var: &str, domains: &[Type]) -> String {
    let domains: Vec<String> = domains.iter().map(ToString::to_string).collect();
    format!("{var} : {} → Prop", domains.join(" → "))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 6.10.26h AC1: a predicate variable is not a type variable, and the
    // most recent binding of a name decides which it is.
    #[test]
    fn a_pred_var_resolves_only_as_a_pred_var() {
        let ctx = Context::new().with_pred_var("P", vec![Type::Nat]);
        assert_eq!(ctx.lookup_pred_var("P"), Some(&[Type::Nat][..]));
        assert!(!ctx.has_type_var("P"));
        assert!(ctx.type_vars().is_empty());
        assert_eq!(ctx.lookup_pred_var("Q"), None);
        let shadowed = ctx.with_type_var("P");
        assert_eq!(shadowed.lookup_pred_var("P"), None);
        let back = shadowed.with_pred_var("P", vec![Type::Bool, Type::Nat]);
        assert_eq!(back.lookup_pred_var("P").map(<[Type]>::len), Some(2));
        // A later type variable of ANOTHER name does not hide the predicate.
        let beside = ctx.with_type_var("Q");
        assert_eq!(beside.lookup_pred_var("P"), Some(&[Type::Nat][..]));
    }

    #[test]
    fn display_names_the_kind() {
        let ctx = Context::new().with_pred_var("P", vec![Type::Nat, Type::Bool]);
        assert_eq!(ctx.to_string(), "P : Nat → Bool → Prop");
    }
}
