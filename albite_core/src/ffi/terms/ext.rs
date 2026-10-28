//! Extended term constructors for FFI (case, fold/unfold, polymorphism, refs, etc.)
//!
//! All constructors are O(1) node pushes (ADR 2.7.26a §4).

use std::ffi::CStr;
use std::os::raw::c_char;

use super::nodes::{materialize_term, TermNode};
use super::{valid_terms, valid_types};
use crate::ffi::{with_arena, with_arena_ref, TermHandle, TypeHandle, INVALID_HANDLE};

// ============================================================================
// Sum Type and Pattern Matching (Phase 3C-5)
// ============================================================================

/// Construct case analysis on a sum type: case t of inl x => t1 | inr y => t2
///
/// # Safety
/// `left_var` and `right_var` must be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn tg_term_case(
    scrutinee: TermHandle,
    left_var: *const c_char,
    left_body: TermHandle,
    right_var: *const c_char,
    right_body: TermHandle,
) -> TermHandle {
    if left_var.is_null() || right_var.is_null() {
        return INVALID_HANDLE;
    }
    let left_var_str = match CStr::from_ptr(left_var).to_str() {
        Ok(s) => s,
        Err(_) => return INVALID_HANDLE,
    };
    let right_var_str = match CStr::from_ptr(right_var).to_str() {
        Ok(s) => s,
        Err(_) => return INVALID_HANDLE,
    };

    with_arena!(|arena| {
        if !valid_terms(arena, &[scrutinee, left_body, right_body]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::Case(
            scrutinee,
            left_var_str.to_owned(),
            left_body,
            right_var_str.to_owned(),
            right_body,
        ))
    })
}

// ============================================================================
// Recursive Types (Phase 3C-5)
// ============================================================================

/// Construct fold: fold [μα.τ] t
///
/// Packs a value into a recursive type.
/// - t : τ[α := μα.τ]
/// - Result: μα.τ
#[no_mangle]
pub extern "C" fn tg_term_fold(mu_ty: TypeHandle, t: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_types(arena, &[mu_ty]) || !valid_terms(arena, &[t]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::Fold(mu_ty, t))
    })
}

/// Construct unfold: unfold [μα.τ] t
///
/// Unpacks a recursive type.
/// - t : μα.τ
/// - Result: τ[α := μα.τ]
#[no_mangle]
pub extern "C" fn tg_term_unfold(mu_ty: TypeHandle, t: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_types(arena, &[mu_ty]) || !valid_terms(arena, &[t]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::Unfold(mu_ty, t))
    })
}

// ============================================================================
// Polymorphism (Phase 3C-5)
// ============================================================================

/// Construct type abstraction: Λα. t
///
/// # Safety
/// `ty_var` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn tg_term_type_abs(ty_var: *const c_char, body: TermHandle) -> TermHandle {
    if ty_var.is_null() {
        return INVALID_HANDLE;
    }
    let ty_var_str = match CStr::from_ptr(ty_var).to_str() {
        Ok(s) => s,
        Err(_) => return INVALID_HANDLE,
    };

    with_arena!(|arena| {
        if !valid_terms(arena, &[body]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::TyAbs(ty_var_str.to_owned(), body))
    })
}

/// Construct type application: t [τ]
#[no_mangle]
pub extern "C" fn tg_term_type_app(t: TermHandle, ty: TypeHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_terms(arena, &[t]) || !valid_types(arena, &[ty]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::TyApp(t, ty))
    })
}

// ============================================================================
// References (Phase 3C-5)
// ============================================================================

/// Construct a new reference: ref v
#[no_mangle]
pub extern "C" fn tg_term_ref_new(v: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_terms(arena, &[v]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::RefNew(v))
    })
}

/// Construct reference dereference: get r
#[no_mangle]
pub extern "C" fn tg_term_ref_get(r: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_terms(arena, &[r]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::RefGet(r))
    })
}

/// Construct reference assignment: set r v
#[no_mangle]
pub extern "C" fn tg_term_ref_set(r: TermHandle, v: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_terms(arena, &[r, v]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::RefSet(r, v))
    })
}

// ============================================================================
// Term Introspection (Phase 3C-5)
// ============================================================================

/// Check if two terms are equal (structurally/α-equivalent)
///
/// Materializes both sides — term equality is a cold path (Eq-type proofs),
/// unlike `tg_types_equal` which runs on the node DAG.
#[no_mangle]
pub extern "C" fn tg_terms_equal(t1: TermHandle, t2: TermHandle) -> bool {
    with_arena_ref!(|arena| {
        match (materialize_term(arena, t1), materialize_term(arena, t2)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    })
}

/// Check two terms for δ-free definitional equality (ADR 30.9.26g D4): the
/// self-host's `refl`/`trans` check, and the same function the bootstrap's
/// fast path calls. Monotone over [`tg_terms_equal`]; `false` on an invalid
/// handle; no global is ever unfolded, so it is safe before admission.
#[no_mangle]
pub extern "C" fn tg_terms_convertible(t1: TermHandle, t2: TermHandle) -> bool {
    with_arena_ref!(|arena| {
        match (materialize_term(arena, t1), materialize_term(arena, t2)) {
            (Some(a), Some(b)) => crate::conversion::convertible_without_delta(&a, &b),
            _ => false,
        }
    })
}

/// Construct a lambda abstraction (alias for `tg_term_lambda`).
///
/// This is a shorter name used by the self-hosted compiler.
///
/// # Safety
/// `var_name` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn tg_term_abs(
    var_name: *const c_char,
    ty: TypeHandle,
    body: TermHandle,
) -> TermHandle {
    super::core::tg_term_lambda(var_name, ty, body)
}

/// Construct a sorry term (placeholder for incomplete proofs).
///
/// Sorry terms have a given type but no computational content.
#[no_mangle]
pub extern "C" fn tg_term_sorry(_ty: TypeHandle) -> TermHandle {
    // Note: The type argument is ignored - Term::Sorry doesn't carry a type
    with_arena!(|arena| arena.alloc_term_node(TermNode::Sorry))
}

/// Construct a foreign call with no arguments yet: `extern_call symbol []`
/// (ADR 3.10.26q D2).
///
/// Arguments are appended one at a time with [`tg_term_extern_call_arg`], so
/// no FFI function has to pass a variable-length handle list. `symbol` is the
/// prefixed `__c_<name>` the bootstrap builds.
///
/// # Safety
/// `symbol` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn tg_term_extern_call(symbol: *const c_char) -> TermHandle {
    if symbol.is_null() {
        return INVALID_HANDLE;
    }
    alloc_extern_call(CStr::from_ptr(symbol).to_str().ok())
}

/// The argument-less foreign call node for an already-decoded symbol;
/// `None` (a symbol that was not UTF-8) is `INVALID_HANDLE`.
fn alloc_extern_call(symbol: Option<&str>) -> TermHandle {
    let Some(symbol) = symbol else {
        return INVALID_HANDLE;
    };
    with_arena!(|arena| arena.alloc_term_node(TermNode::ExternCall(symbol.to_owned(), Vec::new())))
}

/// Append one argument to a foreign call, as a **new** node (ADR 3.10.26q D2).
///
/// The arena is immutable: `call` still names the shorter call afterwards.
/// `INVALID_HANDLE` when either handle is invalid or `call` is not a foreign
/// call.
#[no_mangle]
pub extern "C" fn tg_term_extern_call_arg(call: TermHandle, arg: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_terms(arena, &[arg]) {
            return INVALID_HANDLE;
        }
        let Some(TermNode::ExternCall(symbol, args)) = arena.get_term_node(call) else {
            return INVALID_HANDLE;
        };
        let mut extended = args.clone();
        extended.push(arg);
        let node = TermNode::ExternCall(symbol.clone(), extended);
        arena.alloc_term_node(node)
    })
}

/// Construct an early return term (ADR 13.5.26d).
///
/// Wraps the inner term in `Term::Return`. Type is ⊥ (Void).
#[no_mangle]
pub extern "C" fn tg_term_return(inner: TermHandle) -> TermHandle {
    with_arena!(|arena| {
        if !valid_terms(arena, &[inner]) {
            return INVALID_HANDLE;
        }
        arena.alloc_term_node(TermNode::Return(inner))
    })
}
