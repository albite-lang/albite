//! Type introspection predicates for FFI.
//!
//! Provides C-compatible type predicate functions (`tg_type_is_*`).
//! Type accessors, substitution, and debug utilities are in `types_accessors.rs`.
//!
//! `predicates.rs` became `predicates/` when ADR 13.8.26a's tag-table
//! reconciliation took it past the file-size cap. The seam is production FFI
//! here, the doc-comment↔arms reconciliation in [`tag_table_tests`] — which has
//! to live in this directory, since it `include_str!`s this file.

use std::ffi::{c_char, CStr};

use super::nodes::{materialize_type, TypeNode};
use super::pi::arrow_view;
use crate::ffi::{with_arena_ref, TypeHandle};

// ============================================================================
// Type Tag (discriminant)
// ============================================================================

/// Return a numeric tag identifying the top-level variant of a type.
///
/// **This is the authority for the tag table** (ADR 13.8.26a). Every other
/// mention of these numbers in the repo points here rather than re-listing
/// them; the one surviving transcription — `docs/repo-memory/codegen-pipeline.md`
/// § `TypeHandle` → `CodegenType` Bridge — is reconciled against these arms by
/// `code-health`'s `tag-table-docs`. The list below is reconciled **twice** — by
/// `tag_table_tests::the_documented_tag_table_matches_the_match_arms` under
/// `cargo test`, and by that same `tag-table-docs` check under
/// `make check-health` — so it is a refutable claim rather than a comment, and
/// deleting either guard does not quietly return it to being unguarded.
///
/// Tags (fenced so the variant names read as the code they are, ADR 28.9.26h):
///
/// ```text
///   0 = Nat, 1 = Bool, 2 = String, 3 = Unit, 4 = Void,
///   5: retired (was Prop, ADR 4.10.26i D2), never reused,
///   6 = Arrow, 7 = Product, 8 = Sum, 9 = TyVar, 10 = Forall,
///   11 = Mu, 12 = Eq, 13 = Ref, 14 = Ptr, 15 = App, 16 = Adt, 17 = Int,
///   18 = Sigma, 19 = PredApp, 20 = ForallPred, 21 = PredAbs,
///   99 = unknown / invalid handle
/// ```
#[no_mangle]
pub extern "C" fn tg_type_tag(ty: TypeHandle) -> u64 {
    with_arena_ref!(|arena| {
        match arena.get_type_node(ty) {
            Some(TypeNode::Nat) => 0,
            Some(TypeNode::Bool) => 1,
            Some(TypeNode::String) => 2,
            Some(TypeNode::Unit) => 3,
            Some(TypeNode::Void) => 4,
            Some(TypeNode::Arrow(_, _) | TypeNode::Pi(_, _, _)) => 6,
            Some(TypeNode::Product(_, _)) => 7,
            Some(TypeNode::Sum(_, _)) => 8,
            Some(TypeNode::TyVar(_)) => 9,
            Some(TypeNode::Forall(_, _)) => 10,
            Some(TypeNode::Mu(_, _)) => 11,
            Some(TypeNode::Eq(_, _, _)) => 12,
            Some(TypeNode::Ref(_)) => 13,
            Some(TypeNode::Ptr(_)) => 14,
            Some(TypeNode::App(_, _)) => 15,
            Some(TypeNode::Adt(_, _, _)) => 16,
            // Appended, not slotted beside `Nat`: the table is positional for
            // every `.tg` reader (ADR 14.9.26c §2.1).
            Some(TypeNode::Int) => 17,
            // Its own tag, never `7` (ADR 4.10.26h D6): a reader that took a
            // dependent pair for a product would build its body with `x` unbound.
            Some(TypeNode::Sigma(_, _, _)) => 18,
            // The predicate binders, each its own (ADR 6.10.26h D10)
            Some(TypeNode::PredApp(_, _)) => 19,
            Some(TypeNode::ForallPred(_, _, _)) => 20,
            Some(TypeNode::PredAbs(_, _)) => 21,
            Some(TypeNode::Error) | None => 99,
        }
    })
}

// ============================================================================
// Type Predicates (Phase 3C-5)
// ============================================================================

/// Check if a type is a μ-type (recursive type)
#[no_mangle]
pub extern "C" fn tg_type_is_mu(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::Mu(_, _))))
}

/// Check if a type is a sum type
#[no_mangle]
pub extern "C" fn tg_type_is_sum(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::Sum(_, _))))
}

/// Check if a type is a product type
#[no_mangle]
pub extern "C" fn tg_type_is_product(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::Product(_, _))))
}

/// Check if a type is an arrow (function) type. A dependent `Pi` is one too:
/// it reads as its arrow everywhere that reads arrows (ADR 2.10.26f D1).
#[no_mangle]
pub extern "C" fn tg_type_is_arrow(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| arena.get_type_node(ty).and_then(arrow_view).is_some())
}

/// Check if a type is an equality type (Eq τ t₁ t₂)
#[no_mangle]
pub extern "C" fn tg_type_is_eq(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::Eq(_, _, _))))
}

/// Check if a type is a forall type (∀α. τ)
#[no_mangle]
pub extern "C" fn tg_type_is_forall(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::Forall(_, _))))
}

/// Check if a type is a type variable (named type like record names).
#[no_mangle]
pub extern "C" fn tg_type_is_tyvar(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::TyVar(_))))
}

/// Check if a type is a type application (parametric type like List<T>).
#[no_mangle]
pub extern "C" fn tg_type_is_app(ty: TypeHandle) -> bool {
    with_arena_ref!(|arena| matches!(arena.get_type_node(ty), Some(TypeNode::App(_, _))))
}

/// Whether `name` occurs in the type: [`crate::Type::references_name`], the
/// bootstrap's own walker, so the self-host refuses `type Foo = Foo` on the
/// bootstrap's test (ADR 6.10.26k D6). `false` on an invalid handle or name.
///
/// # Safety
///
/// `name` must be null or point to a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tg_type_references_name(ty: TypeHandle, name: *const c_char) -> bool {
    if name.is_null() {
        return false;
    }
    let Ok(name) = CStr::from_ptr(name).to_str() else {
        return false;
    };
    with_arena_ref!(|arena| {
        materialize_type(arena, ty).is_some_and(|materialized| materialized.references_name(name))
    })
}

// Tests: tag_table_tests.rs
#[cfg(test)]
#[path = "tag_table_tests.rs"]
mod tag_table_tests;

/// ADR 6.10.26k D6: the extern answers what `Type::references_name` answers.
#[cfg(test)]
mod references_name_tests {
    use std::ffi::CString;

    use super::tg_type_references_name;
    use crate::ffi::types::constructors::{tg_type_arrow, tg_type_nat, tg_type_var};
    use crate::ffi::{tg_init, INVALID_HANDLE};

    fn cstr(text: &str) -> CString {
        CString::new(text).expect("no interior nul")
    }

    #[test]
    fn the_extern_finds_a_name_the_walker_finds() {
        tg_init();
        let foo = cstr("Foo");
        let at_foo = cstr("@Foo");
        let bar = cstr("Bar");
        // SAFETY: every pointer is a live, NUL-terminated `CString`.
        unsafe {
            let alias_body = tg_type_arrow(tg_type_nat(), tg_type_var(at_foo.as_ptr()));
            assert!(tg_type_references_name(alias_body, at_foo.as_ptr()));
            assert!(!tg_type_references_name(alias_body, foo.as_ptr()));
            assert!(!tg_type_references_name(alias_body, bar.as_ptr()));
            let bare = tg_type_var(foo.as_ptr());
            assert!(tg_type_references_name(bare, foo.as_ptr()));
        }
    }

    #[test]
    fn the_extern_is_false_on_an_invalid_handle_or_a_null_name() {
        tg_init();
        let foo = cstr("Foo");
        // SAFETY: a live `CString`, and a null the extern checks first.
        unsafe {
            assert!(!tg_type_references_name(INVALID_HANDLE, foo.as_ptr()));
            let bare = tg_type_var(foo.as_ptr());
            assert!(!tg_type_references_name(bare, std::ptr::null()));
        }
    }
}
