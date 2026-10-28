//! ADR 2.10.26f AC 1: the arena holds a `Pi` and reads it as an arrow.
//!
//! Driven through the externs where the self-host calls them, and through
//! `import_type` where a test needs a shape (an `Eq` over open terms) no
//! extern builds directly.

use std::ffi::CString;

use super::*;
use crate::ffi::check::tg_types_equal;
use crate::ffi::termination::{
    tg_termination_add_def, tg_termination_check, tg_termination_declare, tg_termination_note_item,
    tg_termination_plan, tg_termination_reset,
};
use crate::ffi::terms::core::{tg_term_app, tg_term_global};
use crate::ffi::terms::core_data::tg_term_zero;
use crate::ffi::types::accessors::{tg_type_get_arrow_codomain, tg_type_get_arrow_domain};
use crate::ffi::types::accessors_introspection::tg_type_substitute;
use crate::ffi::types::constructors::{tg_type_arrow, tg_type_nat};
use crate::ffi::types::node_equality::type_handles_equal;
use crate::ffi::types::predicates::{tg_type_is_arrow, tg_type_tag};
use crate::ffi::{tg_init, with_arena};
use crate::terms::Term;
use crate::types::{types_equal_alpha, Type};

/// A leaked null-terminated C string, as every caller of the seam supplies.
fn cstr(text: &str) -> *const c_char {
    CString::new(text)
        .expect("no interior nul")
        .into_raw()
        .cast_const()
}

fn var(name: &str) -> Term {
    Term::Var(name.into())
}

fn add(a: Term, b: Term) -> Term {
    Term::NatAdd(Box::new(a), Box::new(b))
}

fn import(ty: &Type) -> TypeHandle {
    with_arena!(|arena| import_type(arena, ty))
}

fn materialize(handle: TypeHandle) -> Option<Type> {
    with_arena_ref!(|arena| materialize_type(arena, handle))
}

fn node(handle: TypeHandle) -> TypeNode {
    with_arena_ref!(|arena| arena.get_type_node(handle).cloned().expect("valid handle"))
}

/// `Eq<Nat, n + 0, n>`, the statement of `add_zero_right`.
fn add_zero_right_statement(n: &str) -> Type {
    Type::eq(Type::Nat, add(var(n), Term::Zero), var(n))
}

/// `(n: Nat) → (m: Nat) → Eq<Nat, n + m, n + m>`, the type of `two`.
fn two_type() -> Type {
    let sum = add(var("n"), var("m"));
    Type::pi(
        "n",
        Type::Nat,
        Type::pi("m", Type::Nat, Type::eq(Type::Nat, sum.clone(), sum)),
    )
}

// 2.10.26f AC1: a Pi only when the binder occurs free (D5).
#[test]
fn tg_type_pi_builds_a_pi_only_when_the_binder_occurs_free() {
    tg_init();
    let nat = tg_type_nat();
    let open = import(&add_zero_right_statement("n"));
    let pi = unsafe { tg_type_pi(cstr("n"), nat, open) };
    assert!(matches!(node(pi), TypeNode::Pi(ref x, d, c) if x == "n" && d == nat && c == open));

    let absent = unsafe { tg_type_pi(cstr("k"), nat, open) };
    assert!(matches!(node(absent), TypeNode::Arrow(d, c) if d == nat && c == open));

    let plain = unsafe { tg_type_pi(cstr("n"), nat, nat) };
    assert!(matches!(node(plain), TypeNode::Arrow(..)));

    // Mentioned only under an inner `Pi` that rebinds `n`: shadowed, so an arrow.
    let shadowed = import(&Type::pi("n", Type::Nat, add_zero_right_statement("n")));
    let outer = unsafe { tg_type_pi(cstr("n"), nat, shadowed) };
    assert!(matches!(node(outer), TypeNode::Arrow(..)));

    // Mentioned in the inner `Pi`'s DOMAIN, which the inner binder does not scope.
    let in_domain = import(&Type::pi("h", add_zero_right_statement("n"), Type::Nat));
    let outer = unsafe { tg_type_pi(cstr("n"), nat, in_domain) };
    assert!(matches!(node(outer), TypeNode::Pi(..)));
}

#[test]
fn tg_type_pi_finds_the_binder_through_every_type_former() {
    tg_init();
    let nat = tg_type_nat();
    let stmt = add_zero_right_statement("n");
    let wrapped = [
        Type::arrow(Type::Nat, stmt.clone()),
        Type::arrow(stmt.clone(), Type::Nat),
        Type::product(Type::Nat, stmt.clone()),
        Type::sum(stmt.clone(), Type::Unit),
        Type::forall("T", stmt.clone()),
        Type::mu("a", stmt.clone()),
        Type::ptr(stmt.clone()),
        Type::ref_ty(stmt.clone()),
        Type::app("Box", vec![Type::Nat, stmt.clone()]),
        Type::adt("Wrap", vec![stmt.clone()], vec![]),
        Type::adt("Wrap", vec![], vec![("W".into(), stmt.clone())]),
        Type::eq(stmt.clone(), Term::Zero, Term::Zero),
        Type::eq(Type::Nat, Term::Zero, var("n")),
    ];
    for ty in wrapped {
        let cod = import(&ty);
        let built = unsafe { tg_type_pi(cstr("n"), nat, cod) };
        assert!(
            matches!(node(built), TypeNode::Pi(..)),
            "missed `n` in {ty:?}"
        );
    }
    let unmentioned = import(&Type::adt(
        "Wrap",
        vec![Type::Nat],
        vec![("W".into(), Type::Nat)],
    ));
    assert!(matches!(
        node(unsafe { tg_type_pi(cstr("n"), nat, unmentioned) }),
        TypeNode::Arrow(..)
    ));
}

#[test]
fn tg_type_pi_refuses_a_null_name_and_a_dangling_child() {
    tg_init();
    let nat = tg_type_nat();
    assert_eq!(
        unsafe { tg_type_pi(std::ptr::null(), nat, nat) },
        INVALID_HANDLE
    );
    assert_eq!(
        unsafe { tg_type_pi(cstr("n"), INVALID_HANDLE, nat) },
        INVALID_HANDLE
    );
    assert_eq!(
        unsafe { tg_type_pi(cstr("n"), nat, INVALID_HANDLE) },
        INVALID_HANDLE
    );
}

#[test]
fn a_pi_reads_as_its_arrow() {
    tg_init();
    let pi = import(&Type::pi("n", Type::Nat, add_zero_right_statement("n")));
    let TypeNode::Pi(_, dom, cod) = node(pi) else {
        panic!("imported a Pi")
    };
    assert!(tg_type_is_arrow(pi));
    assert_eq!(tg_type_tag(pi), 6);
    assert_eq!(tg_type_get_arrow_domain(pi), dom);
    assert_eq!(tg_type_get_arrow_codomain(pi), cod);
    assert!(tg_type_is_dependent(pi));
    let arrow = tg_type_arrow(tg_type_nat(), tg_type_nat());
    assert!(!tg_type_is_dependent(arrow));
    assert!(!tg_type_is_dependent(tg_type_nat()));
    assert!(!tg_type_is_dependent(INVALID_HANDLE));
}

#[test]
fn instantiating_an_arrow_returns_its_codomain_handle_itself() {
    tg_init();
    let nat = tg_type_nat();
    let cod = tg_type_nat();
    let arrow = tg_type_arrow(nat, cod);
    assert_eq!(tg_type_instantiate_codomain(arrow, tg_term_zero()), cod);
    assert_eq!(
        tg_type_instantiate_codomain(nat, tg_term_zero()),
        INVALID_HANDLE
    );
    let pi = import(&Type::pi("n", Type::Nat, add_zero_right_statement("n")));
    assert_eq!(
        tg_type_instantiate_codomain(pi, INVALID_HANDLE),
        INVALID_HANDLE
    );
}

#[test]
fn instantiating_a_pi_substitutes_the_argument() {
    tg_init();
    let pi = import(&Type::pi("n", Type::Nat, add_zero_right_statement("n")));
    let three = with_arena!(|arena| crate::ffi::terms::nodes::import_term(arena, &Term::NatLit(3)));
    let result = tg_type_instantiate_codomain(pi, three);
    assert_eq!(
        materialize(result),
        Some(Type::eq(
            Type::Nat,
            add(Term::NatLit(3), Term::Zero),
            Term::NatLit(3)
        ))
    );
}

#[test]
fn a_swapped_citation_renames_rather_than_captures() {
    tg_init();
    let two = import(&two_type());
    let m = with_arena!(|arena| crate::ffi::terms::nodes::import_term(arena, &var("m")));
    let rest = tg_type_instantiate_codomain(two, m);
    // `(m′: Nat) → Eq<Nat, m + m′, m + m′>`: the inner binder was renamed, and
    // the result is still a Pi (D4), so the second argument can substitute.
    let Some(Type::Pi(binder, _, cod)) = materialize(rest) else {
        panic!("the rest of `two` stays dependent")
    };
    assert_ne!(binder, "m", "the argument `m` was captured");
    let sum = add(var("m"), var(&binder));
    assert_eq!(*cod, Type::eq(Type::Nat, sum.clone(), sum));
    let n = with_arena!(|arena| crate::ffi::terms::nodes::import_term(arena, &var("n")));
    let last = tg_type_instantiate_codomain(rest, n);
    let sum = add(var("m"), var("n"));
    assert_eq!(
        materialize(last),
        Some(Type::eq(Type::Nat, sum.clone(), sum))
    );
}

#[test]
fn import_then_materialize_round_trips_a_nested_pi() {
    tg_init();
    let ty = Type::forall("T", two_type());
    assert_eq!(materialize(import(&ty)), Some(ty));
}

#[test]
fn pi_equality_is_up_to_renaming_and_matches_the_owned_oracle() {
    tg_init();
    let stmt = |n: &str| add_zero_right_statement(n);
    let refl_at = |t: &str, x: &str| {
        Type::forall(
            t,
            Type::pi(
                x,
                Type::TyVar(t.into()),
                Type::eq(Type::TyVar(t.into()), var(x), var(x)),
            ),
        )
    };
    let pairs = [
        (
            Type::pi("n", Type::Nat, stmt("n")),
            Type::pi("k", Type::Nat, stmt("k")),
            true,
        ),
        (
            Type::pi("n", Type::Nat, stmt("n")),
            Type::pi("n", Type::Nat, Type::eq(Type::Nat, var("n"), var("n"))),
            false,
        ),
        (
            Type::pi("n", Type::Nat, stmt("n")),
            Type::pi("n", Type::Bool, stmt("n")),
            false,
        ),
        // A non-dependent Pi equals its arrow; a dependent one does not.
        (
            Type::pi("n", Type::Nat, Type::Nat),
            Type::arrow(Type::Nat, Type::Nat),
            true,
        ),
        (
            Type::arrow(Type::Nat, Type::Nat),
            Type::pi("n", Type::Nat, Type::Nat),
            true,
        ),
        (
            Type::pi("n", Type::Nat, stmt("n")),
            Type::arrow(Type::Nat, stmt("n")),
            false,
        ),
        (Type::pi("n", Type::Nat, stmt("n")), Type::Nat, false),
        (Type::Error, Type::pi("n", Type::Nat, stmt("n")), true),
        // D6's α map: the ∀ binder renames on the handle walk, the Pi's in the core.
        (refl_at("T", "x"), refl_at("U", "y"), true),
        (refl_at("T", "x"), refl_at("U", "x"), true),
    ];
    for (a, b, want) in pairs {
        let (ha, hb) = (import(&a), import(&b));
        assert_eq!(tg_types_equal(ha, hb), want, "{a:?} vs {b:?}");
        assert_eq!(types_equal_alpha(&a, &b), want, "oracle: {a:?} vs {b:?}");
        let reversed = with_arena_ref!(|arena| type_handles_equal(arena, hb, ha));
        assert_eq!(
            reversed,
            types_equal_alpha(&b, &a),
            "reversed: {b:?} vs {a:?}"
        );
    }
    assert!(!tg_types_equal(import(&two_type()), INVALID_HANDLE));
}

#[test]
fn type_variable_substitution_through_a_pi_keeps_its_binder() {
    tg_init();
    let ty = import(&Type::pi(
        "x",
        Type::TyVar("T".into()),
        Type::eq(Type::TyVar("T".into()), var("x"), var("x")),
    ));
    let result = unsafe { tg_type_substitute(ty, cstr("T"), tg_type_nat()) };
    assert_eq!(
        materialize(result),
        Some(Type::pi(
            "x",
            Type::Nat,
            Type::eq(Type::Nat, var("x"), var("x"))
        ))
    );
    // Nothing to substitute: the original handle comes back, shared.
    let unrelated = unsafe { tg_type_substitute(ty, cstr("U"), tg_type_nat()) };
    assert_eq!(unrelated, ty);
}

#[test]
fn rebuilding_keeps_a_pis_binder_and_builds_an_arrow_otherwise() {
    tg_init();
    let pi = import(&Type::pi("n", Type::Nat, add_zero_right_statement("n")));
    let (dom, cod) = (tg_type_get_arrow_domain(pi), tg_type_get_arrow_codomain(pi));
    let rebuilt = tg_type_rebuild_arrow(pi, dom, cod);
    assert!(matches!(node(rebuilt), TypeNode::Pi(ref x, d, c) if x == "n" && d == dom && c == cod));

    let arrow = tg_type_arrow(tg_type_nat(), tg_type_nat());
    let rebuilt = tg_type_rebuild_arrow(arrow, dom, cod);
    assert!(matches!(node(rebuilt), TypeNode::Arrow(d, c) if d == dom && c == cod));
    assert_eq!(
        tg_type_rebuild_arrow(arrow, dom, INVALID_HANDLE),
        INVALID_HANDLE
    );
    assert_eq!(
        tg_type_rebuild_arrow(INVALID_HANDLE, INVALID_HANDLE, cod),
        INVALID_HANDLE
    );
}

#[test]
fn a_pi_codomain_citing_a_partial_global_still_taints_the_proof() {
    tg_init();
    tg_termination_reset();
    // `thm(n: Nat) -> Eq<Nat, looper(n), n>` with `looper` #[partial]: the
    // global sits only inside the Pi's codomain, so admission must read it there.
    let looper_call = Term::App(Box::new(Term::Global("looper".into())), Box::new(var("n")));
    let thm_ty = import(&Type::pi(
        "n",
        Type::Nat,
        Type::eq(Type::Nat, looper_call, var("n")),
    ));
    let loop_ty = tg_type_arrow(tg_type_nat(), tg_type_nat());
    let self_call = || unsafe { tg_term_app(tg_term_global(cstr("looper")), tg_term_zero()) };
    unsafe {
        assert!(tg_termination_note_item(
            cstr("looper"),
            true,
            std::ptr::null(),
            false
        ));
        assert!(tg_termination_note_item(
            cstr("thm"),
            false,
            std::ptr::null(),
            true
        ));
        assert!(tg_termination_declare(cstr("looper")));
        assert!(tg_termination_declare(cstr("thm")));
        assert!(tg_termination_add_def(cstr("looper"), loop_ty, self_call()));
        assert!(tg_termination_add_def(cstr("thm"), thm_ty, tg_term_zero()));
    }
    tg_termination_plan(true);
    unsafe {
        tg_termination_add_def(cstr("looper"), loop_ty, self_call());
        tg_termination_add_def(cstr("thm"), thm_ty, tg_term_zero());
    }
    assert_eq!(
        tg_termination_check(),
        1,
        "the statement's partial global taints `thm`"
    );
}
