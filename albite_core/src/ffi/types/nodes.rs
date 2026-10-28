//! Handle-children node representation for arena-stored types (ADR 2.7.26a §4).
//!
//! The owned-`Type` arena retained O(N·depth) bytes because every composite
//! FFI constructor deep-cloned both children into its new slot — the arena
//! kept a full copy of every intermediate construction step (measured at
//! ~35% of the 31 GiB self-compiled RSS, §3.4). A [`TypeNode`] stores child *handles*
//! instead, so composing N nodes retains O(N): children are shared, never
//! copied. Owned `Type` trees now exist only transiently, materialized at
//! true consumption boundaries (the bootstrap kernel typechecker, display, debug)
//! and freed by Rust afterwards.
//!
//! Handle semantics are unchanged: opaque, monotonically allocated,
//! phase-local (ADR 10.5.26e). Accessors may now return an existing child
//! handle rather than a fresh copy — handles were never identity-comparable,
//! so aliasing is invisible to the self-hosted compiler.

use crate::types::Type;

use crate::ffi::{Arena, TermHandle, TypeHandle};

/// Arena node mirroring [`Type`], with children as handles.
///
/// `Eq`'s term components stay [`TermHandle`]s into the (still owned) term
/// arena. `App`/`Adt` are unreachable from the self-host constructor surface but
/// are mirrored so kernel-produced types import losslessly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TypeNode {
    Bool,
    Nat,
    Int,
    Unit,
    Void,
    String,
    Error,
    TyVar(String),
    Arrow(TypeHandle, TypeHandle),
    /// `(x: dom) → cod`, built only when `x` occurs in `cod` (ADR 2.10.26f D5).
    /// Reads as its `Arrow` through every arrow reader (D1, `pi::arrow_view`).
    Pi(String, TypeHandle, TypeHandle),
    /// `exists x: dom. body` (ADR 4.10.26h D6), mirrored losslessly so a
    /// kernel-produced dependent pair imports; the self-host builds none, and
    /// it reads as its own tag 18, never a product's.
    Sigma(String, TypeHandle, TypeHandle),
    Product(TypeHandle, TypeHandle),
    Sum(TypeHandle, TypeHandle),
    Forall(String, TypeHandle),
    Mu(String, TypeHandle),
    Eq(TypeHandle, TermHandle, TermHandle),
    Ptr(TypeHandle),
    Ref(TypeHandle),
    App(String, Vec<TypeHandle>),
    Adt(String, Vec<TypeHandle>, Vec<(String, TypeHandle)>),
    /// The predicate binders (ADR 6.10.26h D10), mirrored losslessly like
    /// `Sigma`; the self-host builds none, and each reads as its own tag.
    PredApp(String, Vec<TermHandle>),
    ForallPred(String, Vec<TypeHandle>, TypeHandle),
    PredAbs(Vec<(String, TypeHandle)>, TypeHandle),
}

/// Decompose an owned `Type` tree into arena nodes, returning the root
/// handle. Cost: O(tree) nodes retained — used only at import boundaries
/// (kernel typecheck results, `Eq` substitution results).
///
/// A `Pi` is imported losslessly (ADR 2.10.26f D4). 30.9.26d D6 erased it here
/// while the arena had no dependent form; now a citation's result can itself
/// be a `Pi` (the rest of a two-parameter lemma), and erasing it would make the
/// next argument's substitution a no-op. Erasure happens later, at the codegen
/// bridge, where a `Pi` reads as tag 6 (D1).
pub(crate) fn import_type(arena: &mut Arena, ty: &Type) -> TypeHandle {
    let node = match ty {
        Type::Pi(x, a, b) => {
            let (a, b) = (import_type(arena, a), import_type(arena, b));
            TypeNode::Pi(x.clone(), a, b)
        }
        Type::Sigma(x, a, b) => {
            let (a, b) = (import_type(arena, a), import_type(arena, b));
            TypeNode::Sigma(x.clone(), a, b)
        }
        Type::Bool => TypeNode::Bool,
        Type::Nat => TypeNode::Nat,
        Type::Int => TypeNode::Int,
        Type::Unit => TypeNode::Unit,
        Type::Void => TypeNode::Void,
        Type::String => TypeNode::String,
        Type::Error => TypeNode::Error,
        Type::TyVar(v) => TypeNode::TyVar(v.clone()),
        Type::Arrow(a, b) => {
            let (a, b) = (import_type(arena, a), import_type(arena, b));
            TypeNode::Arrow(a, b)
        }
        Type::Product(a, b) => {
            let (a, b) = (import_type(arena, a), import_type(arena, b));
            TypeNode::Product(a, b)
        }
        Type::Sum(a, b) => {
            let (a, b) = (import_type(arena, a), import_type(arena, b));
            TypeNode::Sum(a, b)
        }
        Type::Forall(v, body) => {
            let body = import_type(arena, body);
            TypeNode::Forall(v.clone(), body)
        }
        Type::Mu(v, body) => {
            let body = import_type(arena, body);
            TypeNode::Mu(v.clone(), body)
        }
        Type::Eq(t, lhs, rhs) => {
            let t = import_type(arena, t);
            let lhs = crate::ffi::terms::nodes::import_term(arena, lhs);
            let rhs = crate::ffi::terms::nodes::import_term(arena, rhs);
            TypeNode::Eq(t, lhs, rhs)
        }
        Type::Ptr(inner) => {
            let inner = import_type(arena, inner);
            TypeNode::Ptr(inner)
        }
        Type::Ref(inner) => {
            let inner = import_type(arena, inner);
            TypeNode::Ref(inner)
        }
        Type::App(name, args) => {
            let args = args.iter().map(|a| import_type(arena, a)).collect();
            TypeNode::App(name.clone(), args)
        }
        Type::Adt(name, type_args, variants) => {
            let type_args = type_args.iter().map(|a| import_type(arena, a)).collect();
            let variants = variants
                .iter()
                .map(|(ctor, payload)| (ctor.clone(), import_type(arena, payload)))
                .collect();
            TypeNode::Adt(name.clone(), type_args, variants)
        }
        Type::PredApp(pred, args) => {
            let args = args
                .iter()
                .map(|a| crate::ffi::terms::nodes::import_term(arena, a))
                .collect();
            TypeNode::PredApp(pred.clone(), args)
        }
        Type::ForallPred(pred, domains, body) => {
            let domains = domains.iter().map(|d| import_type(arena, d)).collect();
            let body = import_type(arena, body);
            TypeNode::ForallPred(pred.clone(), domains, body)
        }
        Type::PredAbs(params, body) => {
            let params = params
                .iter()
                .map(|(x, d)| (x.clone(), import_type(arena, d)))
                .collect();
            let body = import_type(arena, body);
            TypeNode::PredAbs(params, body)
        }
    };
    arena.alloc_type_node(node)
}

/// Rebuild an owned `Type` tree from arena nodes. Cost: O(tree) transient —
/// the result is freed by the caller (kernel typecheck input, display,
/// debug). `None` on an invalid/dangling handle.
pub(crate) fn materialize_type(arena: &Arena, handle: TypeHandle) -> Option<Type> {
    let ty = match arena.get_type_node(handle)? {
        TypeNode::Bool => Type::Bool,
        TypeNode::Nat => Type::Nat,
        TypeNode::Int => Type::Int,
        TypeNode::Unit => Type::Unit,
        TypeNode::Void => Type::Void,
        TypeNode::String => Type::String,
        TypeNode::Error => Type::Error,
        TypeNode::TyVar(v) => Type::TyVar(v.clone()),
        TypeNode::Arrow(a, b) => {
            Type::arrow(materialize_type(arena, *a)?, materialize_type(arena, *b)?)
        }
        TypeNode::Pi(x, a, b) => Type::pi(
            x.clone(),
            materialize_type(arena, *a)?,
            materialize_type(arena, *b)?,
        ),
        TypeNode::Sigma(x, a, b) => Type::sigma(
            x.clone(),
            materialize_type(arena, *a)?,
            materialize_type(arena, *b)?,
        ),
        TypeNode::Product(a, b) => {
            Type::product(materialize_type(arena, *a)?, materialize_type(arena, *b)?)
        }
        TypeNode::Sum(a, b) => {
            Type::sum(materialize_type(arena, *a)?, materialize_type(arena, *b)?)
        }
        TypeNode::Forall(v, body) => {
            Type::Forall(v.clone(), Box::new(materialize_type(arena, *body)?))
        }
        TypeNode::Mu(v, body) => Type::Mu(v.clone(), Box::new(materialize_type(arena, *body)?)),
        TypeNode::Eq(t, lhs, rhs) => Type::eq(
            materialize_type(arena, *t)?,
            crate::ffi::terms::nodes::materialize_term(arena, *lhs)?,
            crate::ffi::terms::nodes::materialize_term(arena, *rhs)?,
        ),
        TypeNode::Ptr(inner) => Type::ptr(materialize_type(arena, *inner)?),
        TypeNode::Ref(inner) => Type::ref_ty(materialize_type(arena, *inner)?),
        TypeNode::App(name, args) => Type::app(
            name.clone(),
            args.iter()
                .map(|a| materialize_type(arena, *a))
                .collect::<Option<Vec<_>>>()?,
        ),
        TypeNode::Adt(name, type_args, variants) => Type::adt(
            name.clone(),
            type_args
                .iter()
                .map(|a| materialize_type(arena, *a))
                .collect::<Option<Vec<_>>>()?,
            variants
                .iter()
                .map(|(ctor, payload)| materialize_type(arena, *payload).map(|p| (ctor.clone(), p)))
                .collect::<Option<Vec<_>>>()?,
        ),
        TypeNode::PredApp(pred, args) => Type::pred_app(
            pred.clone(),
            args.iter()
                .map(|a| crate::ffi::terms::nodes::materialize_term(arena, *a))
                .collect::<Option<Vec<_>>>()?,
        ),
        TypeNode::ForallPred(pred, domains, body) => Type::forall_pred(
            pred.clone(),
            domains
                .iter()
                .map(|d| materialize_type(arena, *d))
                .collect::<Option<Vec<_>>>()?,
            materialize_type(arena, *body)?,
        ),
        TypeNode::PredAbs(params, body) => Type::pred_abs(
            params
                .iter()
                .map(|(x, d)| materialize_type(arena, *d).map(|d| (x.clone(), d)))
                .collect::<Option<Vec<_>>>()?,
            materialize_type(arena, *body)?,
        ),
    };
    Some(ty)
}

/// Heap bytes owned by ONE node (strings + handle-vec slabs) — the node
/// arena's per-allocation retention contribution (ADR 2.7.26a §3.4
/// accounting, node semantics: no child recursion, children are shared).
pub(crate) fn node_heap_bytes(node: &TypeNode) -> u64 {
    let handle_size = size_of::<TypeHandle>() as u64;
    match node {
        TypeNode::Bool
        | TypeNode::Nat
        | TypeNode::Int
        | TypeNode::Unit
        | TypeNode::Void
        | TypeNode::String
        | TypeNode::Error
        | TypeNode::Arrow(..)
        | TypeNode::Product(..)
        | TypeNode::Sum(..)
        | TypeNode::Eq(..)
        | TypeNode::Ptr(..)
        | TypeNode::Ref(..) => 0,
        TypeNode::TyVar(v)
        | TypeNode::Forall(v, _)
        | TypeNode::Mu(v, _)
        | TypeNode::Pi(v, _, _)
        | TypeNode::Sigma(v, _, _) => v.capacity() as u64,
        TypeNode::App(name, args) => {
            name.capacity() as u64 + (args.capacity() as u64) * handle_size
        }
        TypeNode::Adt(name, type_args, variants) => {
            name.capacity() as u64
                + (type_args.capacity() as u64) * handle_size
                + (variants.capacity() as u64) * size_of::<(String, TypeHandle)>() as u64
                + variants
                    .iter()
                    .map(|(n, _)| n.capacity() as u64)
                    .sum::<u64>()
        }
        TypeNode::PredApp(pred, args) => {
            pred.capacity() as u64 + (args.capacity() as u64) * size_of::<TermHandle>() as u64
        }
        TypeNode::ForallPred(pred, domains, _) => {
            pred.capacity() as u64 + (domains.capacity() as u64) * handle_size
        }
        TypeNode::PredAbs(params, _) => {
            (params.capacity() as u64) * size_of::<(String, TypeHandle)>() as u64
                + params.iter().map(|(x, _)| x.capacity() as u64).sum::<u64>()
        }
    }
}

// Tests: node_tests/nodes_tests.rs
#[cfg(test)]
#[path = "node_tests/nodes_tests.rs"]
mod tests;
