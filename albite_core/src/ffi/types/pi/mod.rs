//! The arena's dependent function type (ADR 2.10.26f).
//!
//! [`TypeNode::Pi`] records which parameter a codomain mentions, so a citation
//! can substitute its argument. Everything here keeps one promise: **a `Pi`
//! reads as its arrow** (D1). [`arrow_view`] answers for both shapes, and the
//! arrow predicate, the tag and the domain/codomain accessors go through it, so
//! the self-host's ~100 arrow readers and the codegen bridge see the erased
//! arrow without an arm of their own. The few readers that would carry an open
//! codomain into a result type call [`tg_type_instantiate_codomain`] instead,
//! or ask [`tg_type_is_dependent`] first.
//!
//! The substitution and the equality are the core's own bodies
//! (`Type::substitute_term`, `types_equal_with_env`), reached by a transient
//! materialization, so both compilers substitute and compare with one function
//! (D3, D6). Only a `Pi` pays that cost; every non-dependent type keeps its
//! `Arrow` node byte for byte (D5).

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::os::raw::c_char;

use super::nodes::{import_type, materialize_type, TypeNode};
use crate::ffi::terms::nodes::materialize_term;
use crate::ffi::{
    valid_types, with_arena, with_arena_ref, Arena, TermHandle, TypeHandle, INVALID_HANDLE,
};

/// `(domain, codomain)` of an `Arrow` **or** a `Pi`; `None` for anything else
/// (D1). A `Pi`'s codomain is returned as written, with its binder free.
pub(crate) fn arrow_view(node: &TypeNode) -> Option<(TypeHandle, TypeHandle)> {
    match node {
        TypeNode::Arrow(dom, cod) | TypeNode::Pi(_, dom, cod) => Some((*dom, *cod)),
        _ => None,
    }
}

/// Whether the term variable `var` occurs free in an `Eq` argument under `ty`
/// (D5). An inner `Pi` binding the same name shadows it, as in
/// `Type::free_term_vars`. Shared children are visited once: a handle's answer
/// does not depend on where it is reached from, because only a `Pi` *inside*
/// it can shadow.
fn occurs_free(arena: &Arena, var: &str, ty: TypeHandle, seen: &mut HashSet<TypeHandle>) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    let Some(node) = arena.get_type_node(ty) else {
        return false;
    };
    match node {
        TypeNode::Eq(inner, lhs, rhs) => {
            let mentions = |term: TermHandle| {
                materialize_term(arena, term).is_some_and(|t| t.free_vars().contains(var))
            };
            mentions(*lhs) || mentions(*rhs) || occurs_free(arena, var, *inner, seen)
        }
        TypeNode::Pi(binder, dom, cod) => {
            occurs_free(arena, var, *dom, seen)
                || (binder != var && occurs_free(arena, var, *cod, seen))
        }
        TypeNode::Arrow(a, b) | TypeNode::Product(a, b) | TypeNode::Sum(a, b) => {
            occurs_free(arena, var, *a, seen) || occurs_free(arena, var, *b, seen)
        }
        TypeNode::Forall(_, body)
        | TypeNode::Mu(_, body)
        | TypeNode::Ptr(body)
        | TypeNode::Ref(body) => occurs_free(arena, var, *body, seen),
        TypeNode::App(_, args) => args.iter().any(|a| occurs_free(arena, var, *a, seen)),
        TypeNode::Adt(_, args, variants) => {
            args.iter().any(|a| occurs_free(arena, var, *a, seen))
                || variants
                    .iter()
                    .any(|(_, p)| occurs_free(arena, var, *p, seen))
        }
        _ => false,
    }
}

/// `Π var:dom. cod` when `var` occurs free in `cod`, else `dom → cod`: the
/// arena's `Type::pi_or_arrow` (D5).
pub(crate) fn pi_or_arrow_node(
    arena: &mut Arena,
    var: &str,
    dom: TypeHandle,
    cod: TypeHandle,
) -> TypeHandle {
    if !valid_types(arena, &[dom, cod]) {
        return INVALID_HANDLE;
    }
    let node = if occurs_free(arena, var, cod, &mut HashSet::new()) {
        TypeNode::Pi(var.to_owned(), dom, cod)
    } else {
        TypeNode::Arrow(dom, cod)
    };
    arena.alloc_type_node(node)
}

/// Construct a dependent function type `(name: dom) → cod`, or the plain arrow
/// when `name` does not occur in `cod` (D5).
///
/// # Safety
/// `name` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn tg_type_pi(
    name: *const c_char,
    dom: TypeHandle,
    cod: TypeHandle,
) -> TypeHandle {
    if name.is_null() {
        return INVALID_HANDLE;
    }
    let Ok(name) = CStr::from_ptr(name).to_str() else {
        return INVALID_HANDLE;
    };
    with_arena!(|arena| pi_or_arrow_node(arena, name, dom, cod))
}

/// The result type of applying `fn_ty` to the term `arg` (D3): an `Arrow`'s
/// codomain handle itself, with no allocation, or a `Pi`'s codomain with the
/// argument substituted for its binder by the core's capture-avoiding
/// `Type::substitute_term`. `INVALID_HANDLE` for a non-function or an
/// unreadable handle.
#[no_mangle]
pub extern "C" fn tg_type_instantiate_codomain(fn_ty: TypeHandle, arg: TermHandle) -> TypeHandle {
    with_arena!(|arena| instantiate_codomain_node(arena, fn_ty, arg))
}

pub(crate) fn instantiate_codomain_node(
    arena: &mut Arena,
    fn_ty: TypeHandle,
    arg: TermHandle,
) -> TypeHandle {
    match arena.get_type_node(fn_ty) {
        Some(TypeNode::Arrow(_, cod)) => *cod,
        Some(TypeNode::Pi(binder, _, cod)) => {
            let (binder, cod) = (binder.clone(), *cod);
            let (Some(cod), Some(arg)) =
                (materialize_type(arena, cod), materialize_term(arena, arg))
            else {
                return INVALID_HANDLE;
            };
            import_type(arena, &cod.substitute_term(&binder, &arg))
        }
        _ => INVALID_HANDLE,
    }
}

/// True only for a `Pi` node (D11): the one question [`arrow_view`] hides.
#[no_mangle]
pub extern "C" fn tg_type_is_dependent(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::Pi(..))))
}

/// Rebuild a function type from a domain and codomain read out of `orig`,
/// keeping `orig`'s binder when it is a `Pi` (D12). Otherwise a plain arrow.
#[no_mangle]
pub extern "C" fn tg_type_rebuild_arrow(
    orig: TypeHandle,
    dom: TypeHandle,
    cod: TypeHandle,
) -> TypeHandle {
    with_arena!(|arena| rebuild_arrow_node(arena, orig, dom, cod))
}

pub(crate) fn rebuild_arrow_node(
    arena: &mut Arena,
    orig: TypeHandle,
    dom: TypeHandle,
    cod: TypeHandle,
) -> TypeHandle {
    match arena.get_type_node(orig) {
        Some(TypeNode::Pi(binder, ..)) => {
            let binder = binder.clone();
            pi_or_arrow_node(arena, &binder, dom, cod)
        }
        _ if valid_types(arena, &[dom, cod]) => arena.alloc_type_node(TypeNode::Arrow(dom, cod)),
        _ => INVALID_HANDLE,
    }
}

/// Equality of a pair in which either side is a `Pi` (D6): both sides are
/// materialized and handed to the core's owned walk **with the handle walk's
/// own α map**, so a `Pi` met under a `∀` compares its type variables up to
/// the renaming already recorded. One body, two callers.
pub(crate) fn pi_types_equal(
    arena: &Arena,
    a: TypeHandle,
    b: TypeHandle,
    env: &mut HashMap<String, String>,
) -> bool {
    match (materialize_type(arena, a), materialize_type(arena, b)) {
        (Some(a), Some(b)) => crate::types::types_equal_with_env(&a, &b, env),
        _ => false,
    }
}

#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod tests;
