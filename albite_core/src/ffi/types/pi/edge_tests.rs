//! ADR 2.10.26f AC 1, the arena's edges: shared children, dangling children
//! and an unreadable `Pi` — the paths no well-formed self-host type reaches,
//! each of which must answer conservatively rather than panic.

use super::*;
use crate::ffi::types::constructors::{tg_type_nat, tg_type_product};
use crate::ffi::types::node_equality::type_handles_equal;
use crate::ffi::{tg_init, with_arena};
use crate::terms::Term;
use crate::types::Type;

fn eq_mentioning(var: &str) -> Type {
    Type::eq(Type::Nat, Term::Var(var.into()), Term::Zero)
}

#[test]
fn a_shared_child_is_walked_once_and_still_answers() {
    tg_init();
    let unmentioned = tg_type_nat();
    let shared = tg_type_product(unmentioned, unmentioned);
    let built = with_arena!(|arena| pi_or_arrow_node(arena, "n", unmentioned, shared));
    assert!(
        !tg_type_is_dependent(built),
        "a twice-reached child mentioning nothing"
    );

    let mentioned = with_arena!(|arena| import_type(arena, &eq_mentioning("n")));
    let shared = tg_type_product(mentioned, mentioned);
    let built = with_arena!(|arena| pi_or_arrow_node(arena, "n", unmentioned, shared));
    assert!(
        tg_type_is_dependent(built),
        "the first visit already found `n`"
    );
}

#[test]
fn a_dangling_child_mentions_nothing() {
    tg_init();
    let nat = tg_type_nat();
    let dangling = with_arena!(|arena| arena.alloc_type_node(TypeNode::Ptr(INVALID_HANDLE)));
    let built = with_arena!(|arena| pi_or_arrow_node(arena, "n", nat, dangling));
    assert!(!tg_type_is_dependent(built));
}

#[test]
fn an_unreadable_pi_is_unequal_and_does_not_materialize() {
    tg_init();
    let nat = tg_type_nat();
    let broken_cod =
        with_arena!(|arena| arena.alloc_type_node(TypeNode::Pi("n".into(), nat, INVALID_HANDLE)));
    let broken_dom =
        with_arena!(|arena| arena.alloc_type_node(TypeNode::Pi("n".into(), INVALID_HANDLE, nat)));
    with_arena!(|arena| {
        assert_eq!(materialize_type(arena, broken_cod), None);
        assert_eq!(materialize_type(arena, broken_dom), None);
        assert!(!type_handles_equal(arena, broken_cod, broken_cod));
        assert!(!type_handles_equal(arena, nat, broken_dom));
        let zero = crate::ffi::terms::nodes::import_term(arena, &Term::Zero);
        assert_eq!(
            instantiate_codomain_node(arena, broken_cod, zero),
            INVALID_HANDLE
        );
    });
}
