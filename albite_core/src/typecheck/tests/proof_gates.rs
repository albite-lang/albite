//! The kernel's proof judgment and its positivity gate (ADR 29.9.26e).
//!
//! Each refusal sits beside an accepted twin, and each proof-only refusal
//! keeps an executable-mode assertion: `type_of` is where `#[partial]` code
//! lives, so the half of the boundary that must NOT move is pinned here too.
//! The three controls that were known-admitted in `stratification.rs` flipped
//! to the refusals below (§2.4). Global admission, poison, holes and flag
//! propagation (ACs 5–7) are in `proof_mode.rs`.

use super::*;
use crate::context::Context;
use crate::terms::termination::tests::fixtures::{list_fn, list_type, match_list, var};
use crate::terms::Term;
use crate::types::Type;

pub(super) fn tv(name: &str) -> Type {
    Type::TyVar(name.into())
}

/// `fix f: Void. f` — the unrestricted fixpoint at the empty type.
pub(super) fn loop_at_void() -> Term {
    Term::fix("f", Type::Void, Term::var("f"))
}

/// Curry's `ω (fold ω)` over `mu`, with `ω = λx:mu. (unfold x) x`.
fn curry(mu: &Type) -> Term {
    let omega = Term::lambda(
        "x",
        mu.clone(),
        Term::app(Term::unfold(mu.clone(), Term::var("x")), Term::var("x")),
    );
    Term::app(omega.clone(), Term::fold(mu.clone(), omega))
}

/// `μa. a → Void` — the negative `μ` Curry's paradox needs.
fn negative_mu() -> Type {
    Type::mu("a", Type::arrow(tv("a"), Type::Void))
}

fn assert_negative_mu(judgment: &TypeResult<Type>) {
    assert!(
        matches!(judgment, Err(TypeError::NonStrictlyPositiveMu { .. })),
        "{judgment:?}"
    );
}

/// The helper itself must fail on anything but the refusal it names.
#[test]
#[should_panic(expected = "Ok")]
fn assert_negative_mu_rejects_an_accepted_term() {
    assert_negative_mu(&Ok(Type::Void));
}

// ==========================================================================
// AC 1 — positivity at fold and unfold, in both modes (D6)
// ==========================================================================

#[test]
fn kernel_refuses_negative_mu() {
    let ctx = Context::new();
    let term = curry(&negative_mu());
    assert_negative_mu(&type_of(&ctx, &term));
    assert_negative_mu(&type_of_proof(&ctx, &term));
}

#[test]
fn kernel_refuses_negative_mu_at_unfold_alone() {
    // λx:μ. unfold x x — no fold anywhere, so only the Unfold rule can refuse.
    let mu = negative_mu();
    let term = Term::lambda(
        "x",
        mu.clone(),
        Term::app(Term::unfold(mu, Term::var("x")), Term::var("x")),
    );
    assert_negative_mu(&type_of(&Context::new(), &term));
    assert_negative_mu(&type_of_proof(&Context::new(), &term));
}

#[test]
fn kernel_refuses_negative_mu_through_an_inline_adt() {
    let mu = Type::mu(
        "a",
        Type::Adt(
            "Bad".into(),
            vec![],
            vec![("Mk".into(), Type::arrow(tv("a"), Type::Void))],
        ),
    );
    let term = Term::lambda("x", mu.clone(), Term::unfold(mu, Term::var("x")));
    assert_negative_mu(&type_of(&Context::new(), &term));
    assert_negative_mu(&type_of_proof(&Context::new(), &term));
}

/// §1.4 route (b): the `α_` spelling. The executable equality strips `α_`, so
/// `B` here IS the μ to it; positivity must see `B` as the bound variable.
#[test]
fn kernel_refuses_the_alpha_prefixed_spelling() {
    let mu = Type::mu("α_B", Type::arrow(tv("B"), Type::Void));
    let term = Term::ty_abs("B", curry(&mu));
    assert_negative_mu(&type_of(&Context::new(), &term));
    assert_negative_mu(&type_of_proof(&Context::new(), &term));
}

#[test]
fn kernel_admits_positive_mu() {
    let nat_to = Type::mu("a", Type::arrow(Type::Nat, tv("a")));
    for mu in [list_type(), nat_to] {
        // λx:μ. fold (unfold x) : μ → μ
        let term = Term::lambda(
            "x",
            mu.clone(),
            Term::fold(mu.clone(), Term::unfold(mu.clone(), Term::var("x"))),
        );
        let expected = Ok(Type::arrow(mu.clone(), mu.clone()));
        assert_eq!(type_of(&Context::new(), &term), expected);
        assert_eq!(type_of_proof(&Context::new(), &term), expected);
    }
}

// ==========================================================================
// AC 2 — the type-variable/μ shortcut is closed in proof mode (D11)
// ==========================================================================

/// `μa. Void × a` — strictly positive, so only equality can let it through.
pub(super) fn void_stream() -> Type {
    Type::mu("a", Type::product(Type::Void, tv("a")))
}

/// 29.9.26e AC2: route (a) of §1.4 is refused in proof mode and still types
/// at `Void` in executable mode.
#[test]
fn proof_refuses_the_tyvar_mu_shortcut() {
    // Fst((Λa. λx:a. unfold [μ] x) [Unit] ()) : Void
    let mu = void_stream();
    let generic = Term::ty_abs(
        "a",
        Term::lambda("x", tv("a"), Term::unfold(mu, Term::var("x"))),
    );
    let term = Term::fst(Term::app(Term::ty_app(generic, Type::Unit), Term::Unit));
    assert_eq!(type_of(&Context::new(), &term), Ok(Type::Void));
    assert!(
        matches!(
            type_of_proof(&Context::new(), &term),
            Err(TypeError::TypeMismatch { .. })
        ),
        "{:?}",
        type_of_proof(&Context::new(), &term)
    );
}

#[test]
fn proof_admits_the_same_unfold_over_the_mu_itself() {
    let mu = void_stream();
    let term = Term::ty_abs(
        "a",
        Term::lambda(
            "x",
            mu.clone(),
            Term::fst(Term::unfold(mu.clone(), Term::var("x"))),
        ),
    );
    assert_eq!(
        type_of_proof(&Context::new(), &term),
        Ok(Type::forall("a", Type::arrow(mu, Type::Void)))
    );
}

// ==========================================================================
// AC 3 — `fix` in a proof must be certified total (D3)
// ==========================================================================

#[test]
fn proof_refuses_unrestricted_fix() {
    let judgment = type_of_proof(&Context::new(), &loop_at_void());
    let Err(TypeError::FixNotStructural { name, reason }) = judgment else {
        panic!("expected FixNotStructural, got {judgment:?}");
    };
    assert_eq!(name, "f");
    assert!(!reason.is_empty());
}

/// 29.9.26e AC8: the executable judgment is unmoved — `#[partial]` code keeps
/// its unrestricted `fix`, as the rest of the kernel suite does its verdicts.
#[test]
fn executable_keeps_unrestricted_fix() {
    assert_eq!(type_of(&Context::new(), &loop_at_void()), Ok(Type::Void));
}

/// The termination fixtures' `structural_len`, with its recursive
/// `Global("len")` rebound to the fixpoint's `Var("len")`.
fn structural_len_fix() -> Term {
    let body = list_fn(
        "l",
        match_list(
            var("l"),
            Term::Zero,
            "t",
            Term::app(Term::var("len"), var("t")),
        ),
    );
    Term::fix("len", Type::arrow(list_type(), Type::Nat), body)
}

#[test]
fn proof_admits_structural_fix() {
    let expected = Ok(Type::arrow(list_type(), Type::Nat));
    assert_eq!(
        type_of_proof(&Context::new(), &structural_len_fix()),
        expected
    );
    assert_eq!(type_of(&Context::new(), &structural_len_fix()), expected);
}

#[test]
fn proof_refuses_a_fix_whose_only_root_is_a_type_parameter() {
    // Λa. fix f: a → Nat. λx:a. f x — `a` is abstract, not an inductive.
    let body = Term::lambda("x", tv("a"), Term::app(Term::var("f"), Term::var("x")));
    let term = Term::ty_abs("a", Term::fix("f", Type::arrow(tv("a"), Type::Nat), body));
    assert!(matches!(
        type_of_proof(&Context::new(), &term),
        Err(TypeError::FixNotStructural { .. })
    ));
    assert!(type_of(&Context::new(), &term).is_ok());
}

// ==========================================================================
// AC 4 — `return` is refused in a proof only (D5)
// ==========================================================================

#[test]
fn proof_refuses_return() {
    let term = Term::Return(Box::new(Term::Unit));
    assert_eq!(
        type_of_proof(&Context::new(), &term),
        Err(TypeError::ReturnInProof)
    );
}

#[test]
fn proof_admits_the_same_term_without_return() {
    assert_eq!(type_of_proof(&Context::new(), &Term::Unit), Ok(Type::Unit));
}

#[test]
fn executable_keeps_return_as_void() {
    let term = Term::Return(Box::new(Term::Unit));
    assert_eq!(type_of(&Context::new(), &term), Ok(Type::Void));
}
