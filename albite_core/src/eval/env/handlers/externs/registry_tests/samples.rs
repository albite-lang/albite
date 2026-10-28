//! Sample arguments for the registry's handle-taking entries (split out of
//! `registry_tests/mod.rs`, ADR 4.10.26n): each builds the live node or buffer
//! its entry decomposes, so "claimed by dispatch" means the arm works.

use crate::eval::nat_to_term;
use crate::ffi::types::constructors;
use crate::terms::Term;

/// Samples for the type-arena entries (ADR 7.8.26c §5).
///
/// Every accessor sample **builds a live node first**, and that is the whole
/// point of splitting this out. `dispatch_claims` asserts only "not `Stuck`",
/// so an accessor handed `nat_to_term(0)` against an arena that happens to be
/// empty returns `INVALID_HANDLE` and *still counts as claimed*: the coverage
/// test would prove the arm exists, not that it works. Allocating the node the
/// accessor is meant to decompose is what makes the claim non-vacuous.
pub(super) fn arena_sample_arguments(name: &str) -> Vec<Term> {
    match name {
        "tg_type_nat" | "tg_type_bool" | "tg_type_unit" => vec![],
        "tg_type_arrow" | "tg_type_sum" => vec![live_nat_type(), live_bool_type()],
        "tg_type_mu" => vec![live_cstr_address("List"), live_nat_type()],
        "tg_type_tag" => vec![live_nat_type()],
        "tg_type_get_arrow_domain" | "tg_type_get_arrow_codomain" => vec![live_arrow_type()],
        "tg_type_get_sum_left" | "tg_type_get_sum_right" => vec![live_sum_type()],
        "tg_type_get_product_left" | "tg_type_get_product_right" => vec![live_product_type()],
        "tg_type_get_mu_body" | "tg_type_get_mu_var" => vec![live_mu_type()],
        "tg_type_get_forall_body" | "tg_type_get_forall_var" => vec![live_forall_type()],
        "tg_type_get_tyvar_name" => vec![live_tyvar_type()],
        "tg_string_to_cstr" => vec![Term::StringLit("List".to_string())],
        "tg_cstring_to_string" => vec![live_cstr_address("List")],
        _ => builder_sample_arguments(name),
    }
}

/// Samples for the `StringBuilder` entries (ADR 14.9.26a §2.2).
///
/// Every handle operand is a **live** builder from a real `new`: the runtime
/// aborts the process on a null or consumed handle, so `nat_to_term(0)` here
/// would not merely make the claim vacuous — it would kill the test binary.
fn builder_sample_arguments(name: &str) -> Vec<Term> {
    match name {
        "tg_string_builder_new" => vec![],
        "tg_string_builder_with_capacity" => vec![nat_to_term(16)],
        "tg_string_builder_push_str" => vec![live_builder(), Term::StringLit("x".to_string())],
        "tg_string_builder_push_char" => vec![live_builder(), nat_to_term(0x41)],
        "tg_string_builder_len" | "tg_string_builder_to_string" => vec![live_builder()],
        other => super::super::super::bytes::tests::registry_sample_arguments(other),
    }
}

/// A handle to a freshly allocated, live `StringBuilder`.
fn live_builder() -> Term {
    nat_to_term(crate::ffi::tg_string_builder_new() as usize)
}

/// A handle to a freshly allocated `Nat` type node.
pub(super) fn live_nat_type() -> Term {
    nat_to_term(constructors::tg_type_nat() as usize)
}

/// A handle to a freshly allocated `Bool` type node.
fn live_bool_type() -> Term {
    nat_to_term(constructors::tg_type_bool() as usize)
}

/// A handle to a live `Nat -> Bool`.
fn live_arrow_type() -> Term {
    nat_to_term(constructors::tg_type_arrow(
        constructors::tg_type_nat(),
        constructors::tg_type_bool(),
    ) as usize)
}

/// A handle to a live `Nat + Bool`.
fn live_sum_type() -> Term {
    nat_to_term(
        constructors::tg_type_sum(constructors::tg_type_nat(), constructors::tg_type_bool())
            as usize,
    )
}

/// A handle to a live `Nat × Bool`.
fn live_product_type() -> Term {
    nat_to_term(constructors::tg_type_product(
        constructors::tg_type_nat(),
        constructors::tg_type_bool(),
    ) as usize)
}

/// A handle to a live `μList. Nat`.
fn live_mu_type() -> Term {
    nat_to_term(crate::ffi::mu_type_from_cstr(
        crate::ffi::cstr_address_of("List"),
        constructors::tg_type_nat(),
    ) as usize)
}

/// A handle to a live `∀T. Bool`.
fn live_forall_type() -> Term {
    nat_to_term(crate::ffi::test_support::forall_type_from_cstr(
        crate::ffi::cstr_address_of("T"),
        constructors::tg_type_bool(),
    ) as usize)
}

/// A handle to a live type variable `T`.
fn live_tyvar_type() -> Term {
    nat_to_term(
        crate::ffi::test_support::tyvar_type_from_cstr(crate::ffi::cstr_address_of("T")) as usize,
    )
}

/// The address of a leaked C string holding `text`.
fn live_cstr_address(text: &str) -> Term {
    nat_to_term(crate::ffi::cstr_address_of(text))
}
