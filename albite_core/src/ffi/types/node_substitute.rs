//! τ[α := τ'] on the node DAG with structural sharing (ADR 2.7.26a §4).
//!
//! Port of `Type::substitute` to [`TypeNode`] handles. Untouched subtrees
//! return their ORIGINAL handle — only the spine of paths that actually
//! contain `α` allocates new nodes. This is what makes the self-hosted compiler's μ-unfold
//! (`tg_type_substitute`, the hottest type op) retain O(paths-to-α) instead
//! of O(tree) per call. The `substitute_matches_owned_oracle` test pins the
//! port to the owned implementation.

use super::nodes::{materialize_type, TypeNode};
use crate::ffi::{Arena, TypeHandle};
use crate::types::Type;

/// Substitute `var := replacement` in `ty`. Returns the original handle when
/// nothing changed (sharing); `None` on a dangling handle.
pub(crate) fn substitute_type_handle(
    arena: &mut Arena,
    ty: TypeHandle,
    var: &str,
    replacement: TypeHandle,
) -> Option<TypeHandle> {
    arena.get_type_node(replacement)?;
    let mut ctx = SubstCtx {
        var,
        replacement,
        owned_replacement: None,
    };
    ctx.subst(arena, ty)
}

/// One substitution's state: the variable, the replacement handle, and a
/// lazily materialized owned copy of the replacement (needed only when an
/// `Eq` node's terms require `Term::substitute_type`).
struct SubstCtx<'a> {
    var: &'a str,
    replacement: TypeHandle,
    owned_replacement: Option<Type>,
}

impl SubstCtx<'_> {
    /// Flat dispatcher over node shapes; arms delegate to the shape helpers
    /// below, each of which returns the ORIGINAL handle when nothing changed.
    fn subst(&mut self, arena: &mut Arena, ty: TypeHandle) -> Option<TypeHandle> {
        let node = arena.get_type_node(ty)?.clone();
        match node {
            TypeNode::Bool
            | TypeNode::Nat
            | TypeNode::Int
            | TypeNode::Unit
            | TypeNode::Void
            | TypeNode::String
            | TypeNode::Error => Some(ty),
            TypeNode::TyVar(v) => Some(if v == self.var { self.replacement } else { ty }),
            TypeNode::Arrow(a, b) => self.pair(arena, ty, (a, b), TypeNode::Arrow),
            // A term binder never shadows a type variable (ADR 2.10.26f D4).
            TypeNode::Pi(x, a, b) => self.pair(arena, ty, (a, b), |a, b| TypeNode::Pi(x, a, b)),
            TypeNode::Sigma(x, a, b) => {
                self.pair(arena, ty, (a, b), |a, b| TypeNode::Sigma(x, a, b))
            }
            TypeNode::Product(a, b) => self.pair(arena, ty, (a, b), TypeNode::Product),
            TypeNode::Sum(a, b) => self.pair(arena, ty, (a, b), TypeNode::Sum),
            TypeNode::Forall(v, body) => self.binder(arena, ty, (v, body), TypeNode::Forall),
            TypeNode::Mu(v, body) => self.binder(arena, ty, (v, body), TypeNode::Mu),
            TypeNode::Eq(t, lhs, rhs) => self.eq(arena, ty, (t, lhs, rhs)),
            TypeNode::Ptr(inner) => self.unary(arena, ty, inner, TypeNode::Ptr),
            TypeNode::Ref(inner) => self.unary(arena, ty, inner, TypeNode::Ref),
            TypeNode::App(name, args) => self.app(arena, ty, (name, args)),
            TypeNode::Adt(name, type_args, variants) => {
                self.adt(arena, ty, (name, type_args, variants))
            }
            TypeNode::PredApp(..) | TypeNode::ForallPred(..) | TypeNode::PredAbs(..) => {
                self.predicate_form(arena, ty)
            }
        }
    }

    /// The predicate binders (ADR 6.10.26h D10) substitute through the owned
    /// `Type::substitute`, whose hereditary arm they need; the self-host
    /// builds none, so this cold path keeps no sharing.
    fn predicate_form(&mut self, arena: &mut Arena, ty: TypeHandle) -> Option<TypeHandle> {
        if self.owned_replacement.is_none() {
            self.owned_replacement = Some(materialize_type(arena, self.replacement)?);
        }
        let repl = self.owned_replacement.as_ref().expect("just materialized");
        let old = materialize_type(arena, ty)?;
        let new = old.substitute(self.var, repl);
        Some(if new == old {
            ty
        } else {
            super::nodes::import_type(arena, &new)
        })
    }

    fn pair(
        &mut self,
        arena: &mut Arena,
        ty: TypeHandle,
        (a, b): (TypeHandle, TypeHandle),
        make: impl FnOnce(TypeHandle, TypeHandle) -> TypeNode,
    ) -> Option<TypeHandle> {
        let (na, nb) = (self.subst(arena, a)?, self.subst(arena, b)?);
        if na == a && nb == b {
            return Some(ty);
        }
        Some(arena.alloc_type_node(make(na, nb)))
    }

    fn unary(
        &mut self,
        arena: &mut Arena,
        ty: TypeHandle,
        inner: TypeHandle,
        make: fn(TypeHandle) -> TypeNode,
    ) -> Option<TypeHandle> {
        let ni = self.subst(arena, inner)?;
        if ni == inner {
            return Some(ty);
        }
        Some(arena.alloc_type_node(make(ni)))
    }

    /// Binder shadowing: no substitution under a binder of the same name.
    fn binder(
        &mut self,
        arena: &mut Arena,
        ty: TypeHandle,
        (v, body): (String, TypeHandle),
        make: fn(String, TypeHandle) -> TypeNode,
    ) -> Option<TypeHandle> {
        if v == self.var {
            return Some(ty);
        }
        let nb = self.subst(arena, body)?;
        if nb == body {
            return Some(ty);
        }
        Some(arena.alloc_type_node(make(v, nb)))
    }

    /// Eq descends into its (owned) terms via `Term::substitute_type`.
    fn eq(
        &mut self,
        arena: &mut Arena,
        ty: TypeHandle,
        (t, lhs, rhs): (TypeHandle, crate::ffi::TermHandle, crate::ffi::TermHandle),
    ) -> Option<TypeHandle> {
        let nt = self.subst(arena, t)?;
        if self.owned_replacement.is_none() {
            self.owned_replacement = Some(materialize_type(arena, self.replacement)?);
        }
        let repl = self.owned_replacement.as_ref().expect("just materialized");
        let old_lhs = crate::ffi::terms::nodes::materialize_term(arena, lhs)?;
        let old_rhs = crate::ffi::terms::nodes::materialize_term(arena, rhs)?;
        let new_lhs = old_lhs.substitute_type(self.var, repl);
        let new_rhs = old_rhs.substitute_type(self.var, repl);
        if nt == t && new_lhs == old_lhs && new_rhs == old_rhs {
            return Some(ty);
        }
        let hl = crate::ffi::terms::nodes::import_term(arena, &new_lhs);
        let hr = crate::ffi::terms::nodes::import_term(arena, &new_rhs);
        Some(arena.alloc_type_node(TypeNode::Eq(nt, hl, hr)))
    }

    fn app(
        &mut self,
        arena: &mut Arena,
        ty: TypeHandle,
        (name, args): (String, Vec<TypeHandle>),
    ) -> Option<TypeHandle> {
        let new_args = args
            .iter()
            .map(|a| self.subst(arena, *a))
            .collect::<Option<Vec<_>>>()?;
        if new_args == args {
            return Some(ty);
        }
        Some(arena.alloc_type_node(TypeNode::App(name, new_args)))
    }

    fn adt(
        &mut self,
        arena: &mut Arena,
        ty: TypeHandle,
        (name, type_args, variants): (String, Vec<TypeHandle>, Vec<(String, TypeHandle)>),
    ) -> Option<TypeHandle> {
        let new_args = type_args
            .iter()
            .map(|a| self.subst(arena, *a))
            .collect::<Option<Vec<_>>>()?;
        let new_variants = variants
            .iter()
            .map(|(ctor, payload)| self.subst(arena, *payload).map(|p| (ctor.clone(), p)))
            .collect::<Option<Vec<_>>>()?;
        if new_args == type_args && new_variants == variants {
            return Some(ty);
        }
        Some(arena.alloc_type_node(TypeNode::Adt(name, new_args, new_variants)))
    }
}

// Tests: node_tests/substitute_tests.rs
#[cfg(test)]
#[path = "node_tests/substitute_tests.rs"]
mod tests;
