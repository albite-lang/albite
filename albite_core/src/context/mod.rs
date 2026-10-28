//! Typing Context
//!
//! A typing context Γ contains:
//! - term variable bindings: x : τ
//! - type variable bindings: α (in scope)
//! - global bindings: `name : τ`, marked admitted or opaque for proofs
//! - admitted bodies: `name : τ := t`, a global the kernel typed itself and
//!   may unfold (ADR 2.10.26l D3; `bodies.rs`)
//!
//! It also carries the judgment's mode: a proof-mode context (ADR 29.9.26e
//! D2) is inherited by every context extended from it, so a rule that binds
//! a variable cannot silently drop back to the executable judgment.

mod bodies;
mod kinds; // predicate variables (ADR 6.10.26h D6)

use std::collections::HashSet;
use std::fmt;

use crate::typecheck::AdmittedBody;
use crate::types::{TyVar, Type};

pub(crate) use bodies::ContextUnfoldings;

/// A term variable name
pub type Var = String;

/// A binding in the context
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Term variable binding x : τ
    Term(Var, Type),
    /// Type variable binding α
    TypeVar(TyVar),
    /// A top-level definition `name : τ`, with the caller's admission verdict
    /// (ADR 29.9.26e D4).
    Global(Var, Type, GlobalAdmission),
    /// An admitted global with the body the kernel typed itself, which
    /// conversion may unfold (ADR 2.10.26l D3). Admitted by construction: an
    /// [`AdmittedBody`] is made only by the kernel's own admission, so an
    /// `Opaque` global can never carry one.
    Body(Var, Type, AdmittedBody),
    /// A predicate variable with its domains, `P : A₁ → … → Aₙ → Prop` (ADR
    /// 6.10.26h D6). Distinct from `TypeVar`: it is not a type, so it never
    /// joins [`Context::type_vars`] and a bare `TyVar(P)` stays malformed.
    PredVar(TyVar, Vec<Type>),
}

/// Whether a global may be used in a proof (ADR 29.9.26e D4).
///
/// The kernel has no definitions, so it cannot compute admission: the caller
/// records its verdict here and the proof judgment honours it. This marker is
/// the explicit axiom boundary a `#[partial]` definition must not cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalAdmission {
    /// Certified total by the caller: usable in a proof.
    Admitted,
    /// Partial, rejected or unknown: usable only in executable code.
    Opaque,
}

/// A typing context Γ
///
/// Contains term bindings (x : τ) and type variable bindings (α).
/// Implemented as a list for simplicity (later bindings shadow earlier ones).
#[derive(Debug, Clone, Default)]
pub struct Context {
    bindings: Vec<Binding>,
    /// Whether this is the proof judgment (ADR 29.9.26e D2).
    proof: bool,
}

impl Context {
    /// Create an empty context
    #[must_use]
    pub fn new() -> Self {
        Context {
            bindings: Vec::new(),
            proof: false,
        }
    }

    /// The same context, under the proof judgment. Every context extended
    /// from the result is a proof context too.
    #[must_use]
    pub fn for_proof(&self) -> Self {
        let mut ctx = self.clone();
        ctx.proof = true;
        ctx
    }

    /// Whether this context belongs to the proof judgment.
    #[must_use]
    pub fn is_proof(&self) -> bool {
        self.proof
    }

    /// Extend context with a global definition: Γ, name : τ (admission).
    #[must_use]
    pub fn with_global(
        &self,
        name: impl Into<String>,
        ty: Type,
        admission: GlobalAdmission,
    ) -> Self {
        let mut ctx = self.clone();
        ctx.bindings
            .push(Binding::Global(name.into(), ty, admission));
        ctx
    }

    /// Extend context with a term binding: Γ, x : τ
    #[must_use]
    pub fn with_term(&self, var: impl Into<String>, ty: Type) -> Self {
        let mut ctx = self.clone();
        ctx.bindings.push(Binding::Term(var.into(), ty));
        ctx
    }

    /// Extend context with a type variable: Γ, α
    #[must_use]
    pub fn with_type_var(&self, var: impl Into<String>) -> Self {
        let mut ctx = self.clone();
        ctx.bindings.push(Binding::TypeVar(var.into()));
        ctx
    }

    /// Look up a term variable's type: x : τ ∈ Γ
    #[must_use]
    pub fn lookup_term(&self, var: &str) -> Option<&Type> {
        // Search from the end (most recent binding) to handle shadowing
        self.term_binding(var).map(|(ty, _)| ty)
    }

    /// The admission marker of the binding `var` resolves to: `None` when it
    /// resolves to a `with_term` binding or to nothing (ADR 29.9.26e D4).
    #[must_use]
    pub fn lookup_global_admission(&self, var: &str) -> Option<GlobalAdmission> {
        self.term_binding(var).and_then(|(_, admission)| admission)
    }

    /// The most recent term or global binding of `var`, with its marker.
    fn term_binding(&self, var: &str) -> Option<(&Type, Option<GlobalAdmission>)> {
        self.bindings
            .iter()
            .rev()
            .find_map(|binding| match binding {
                Binding::Term(v, ty) if v == var => Some((ty, None)),
                Binding::Global(v, ty, admission) if v == var => Some((ty, Some(*admission))),
                Binding::Body(v, ty, _) if v == var => Some((ty, Some(GlobalAdmission::Admitted))),
                _ => None,
            })
    }

    /// Check if a type variable is in scope: α ∈ Γ
    #[must_use]
    pub fn has_type_var(&self, var: &str) -> bool {
        self.bindings
            .iter()
            .any(|b| matches!(b, Binding::TypeVar(v) if v == var))
    }

    /// Get all type variables in scope
    #[must_use]
    pub fn type_vars(&self) -> HashSet<TyVar> {
        self.bindings
            .iter()
            .filter_map(|b| {
                if let Binding::TypeVar(v) = b {
                    Some(v.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all term variables in scope with their types
    #[must_use]
    pub fn term_vars(&self) -> Vec<(&str, &Type)> {
        let mut vars = Vec::new();
        let mut seen = HashSet::new();
        // Reverse to get most recent bindings first
        for binding in self.bindings.iter().rev() {
            if let Binding::Term(v, ty) | Binding::Global(v, ty, _) | Binding::Body(v, ty, _) =
                binding
            {
                if !seen.contains(v.as_str()) {
                    vars.push((v.as_str(), ty));
                    seen.insert(v.as_str());
                }
            }
        }
        vars.reverse();
        vars
    }

    /// Check if the context is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Get the number of bindings
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// The raw bindings, for arena retention accounting (ADR 2.7.26a §3.4).
    pub(crate) fn bindings(&self) -> &Vec<Binding> {
        &self.bindings
    }
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.bindings.is_empty() {
            write!(f, "∅")
        } else {
            let parts: Vec<String> = self
                .bindings
                .iter()
                .map(|b| match b {
                    Binding::Term(v, ty) | Binding::Global(v, ty, GlobalAdmission::Admitted) => {
                        format!("{v} : {ty}")
                    }
                    Binding::TypeVar(v) => v.clone(),
                    Binding::Global(v, ty, GlobalAdmission::Opaque) => {
                        format!("{v} : {ty} (opaque)")
                    }
                    Binding::Body(v, ty, _) => format!("{v} : {ty} (unfolds)"),
                    Binding::PredVar(p, domains) => kinds::render_pred_var(p, domains),
                })
                .collect();
            write!(f, "{}", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_context() {
        let ctx = Context::new();
        assert!(ctx.is_empty());
        assert_eq!(ctx.lookup_term("x"), None);
        assert!(!ctx.has_type_var("α"));
    }

    #[test]
    fn test_term_binding() {
        let ctx = Context::new().with_term("x", Type::Nat);
        assert_eq!(ctx.lookup_term("x"), Some(&Type::Nat));
        assert_eq!(ctx.lookup_term("y"), None);
    }

    #[test]
    fn test_type_var_binding() {
        let ctx = Context::new().with_type_var("α").with_type_var("β");
        assert!(ctx.has_type_var("α"));
        assert!(ctx.has_type_var("β"));
        assert!(!ctx.has_type_var("γ"));
    }

    #[test]
    fn test_shadowing() {
        let ctx = Context::new()
            .with_term("x", Type::Nat)
            .with_term("x", Type::Bool);
        // Most recent binding wins
        assert_eq!(ctx.lookup_term("x"), Some(&Type::Bool));
    }

    #[test]
    fn test_mixed_bindings() {
        let ctx = Context::new()
            .with_type_var("α")
            .with_term("x", Type::TyVar("α".into()))
            .with_type_var("β");

        assert!(ctx.has_type_var("α"));
        assert!(ctx.has_type_var("β"));
        assert_eq!(ctx.lookup_term("x"), Some(&Type::TyVar("α".into())));
    }

    #[test]
    fn test_display() {
        let ctx = Context::new()
            .with_type_var("α")
            .with_term("x", Type::Nat)
            .with_term("f", Type::arrow(Type::Nat, Type::Bool));

        let s = ctx.to_string();
        assert!(s.contains("α"));
        assert!(s.contains("x : Nat"));
        assert!(s.contains("f : (Nat → Bool)"));
    }

    // ADR 29.9.26e AC 5, AC 7: globals, their markers and the proof flag.
    #[test]
    fn global_binding_resolves_like_a_term_and_carries_its_marker() {
        let ctx = Context::new()
            .with_global("two", Type::Nat, GlobalAdmission::Admitted)
            .with_global("loop", Type::Void, GlobalAdmission::Opaque)
            .with_term("x", Type::Bool);
        assert_eq!(ctx.lookup_term("two"), Some(&Type::Nat));
        assert_eq!(ctx.lookup_term("loop"), Some(&Type::Void));
        assert_eq!(
            ctx.lookup_global_admission("two"),
            Some(GlobalAdmission::Admitted)
        );
        assert_eq!(
            ctx.lookup_global_admission("loop"),
            Some(GlobalAdmission::Opaque)
        );
        assert_eq!(ctx.lookup_global_admission("x"), None);
        assert_eq!(ctx.lookup_global_admission("absent"), None);
        assert_eq!(ctx.term_vars().len(), 3);
    }

    #[test]
    fn a_term_binding_shadows_a_global_of_the_same_name() {
        let ctx = Context::new()
            .with_global("n", Type::Nat, GlobalAdmission::Admitted)
            .with_term("n", Type::Bool);
        assert_eq!(ctx.lookup_term("n"), Some(&Type::Bool));
        assert_eq!(ctx.lookup_global_admission("n"), None);
        let back = ctx.with_global("n", Type::Unit, GlobalAdmission::Opaque);
        assert_eq!(back.lookup_term("n"), Some(&Type::Unit));
        assert_eq!(
            back.lookup_global_admission("n"),
            Some(GlobalAdmission::Opaque)
        );
    }

    #[test]
    fn proof_flag_is_inherited_by_every_extension() {
        let base = Context::new();
        assert!(!base.is_proof());
        let proof = base.for_proof();
        assert!(proof.is_proof());
        assert!(proof.with_term("x", Type::Nat).is_proof());
        assert!(proof.with_type_var("a").is_proof());
        assert!(proof
            .with_global("g", Type::Nat, GlobalAdmission::Admitted)
            .is_proof());
        assert!(!base.with_term("x", Type::Nat).is_proof());
    }

    #[test]
    fn display_marks_an_opaque_global() {
        let ctx = Context::new()
            .with_global("two", Type::Nat, GlobalAdmission::Admitted)
            .with_global("loop", Type::Void, GlobalAdmission::Opaque);
        let rendered = ctx.to_string();
        assert!(rendered.contains("two : Nat,"), "{rendered}");
        assert!(rendered.contains("loop : Void (opaque)"), "{rendered}");
        assert!(!rendered.contains("Nat (opaque)"), "{rendered}");
    }
}
