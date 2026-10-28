//! The kernel's dependent pair (ADR 4.10.26h AC 1): the introduction rule
//! carried by `Annot`, the projections, and D4's β through the annotation.
//! The must-fail twins are in `twins.rs`. Shared builders below.

use std::collections::HashMap;

use super::*;
use crate::context::Context;
use crate::conversion::{convertible, Convertible, CONVERSION_STEP_BUDGET};
use crate::terms::Term;
use crate::types::Type;

mod twins;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `Σ k:Nat. Eq<Nat, n + k, n>` — "some `k` leaves `n` unchanged".
fn zero_witness_statement() -> Type {
    Type::sigma(
        "k",
        Type::Nat,
        eq_nat(
            Term::nat_add(Term::var("n"), Term::var("k")),
            Term::var("n"),
        ),
    )
}

/// `Annot((witness, refl n), Σ k:Nat. n + k == n)`.
fn witnessed_by(witness: Term) -> Term {
    Term::annot(
        Term::pair(witness, Term::refl(Type::Nat, Term::var("n"))),
        zero_witness_statement(),
    )
}

/// `Σ k:Nat. Eq<Nat, k, k>`, every `k` a witness — for the neutral cases.
fn reflexive_statement(binder: &str) -> Type {
    Type::sigma(
        binder,
        Type::Nat,
        eq_nat(Term::var(binder), Term::var(binder)),
    )
}

fn over_n() -> Context {
    Context::new().with_term("n", Type::Nat).for_proof()
}

fn over_p() -> Context {
    Context::new()
        .with_term("p", reflexive_statement("k"))
        .for_proof()
}

fn converts(left: &Term, right: &Term) -> bool {
    convertible(left, right, &HashMap::new(), CONVERSION_STEP_BUDGET) == Convertible::Yes
}

/// 4.10.26h AC1: an annotated pair checks against its instance, `n + 0 ≡ n`.
#[test]
fn an_annotated_pair_checks_against_its_instance() {
    assert_eq!(
        type_of(&over_n(), &witnessed_by(Term::Zero)),
        Ok(zero_witness_statement())
    );
    // The executable judgment has the rule too (D2: both modes), but compares
    // `Eq` witnesses without conversion, so its instance must match as written.
    let syntactic = Type::sigma("k", Type::Nat, eq_nat(Term::var("k"), Term::Zero));
    let pair = Term::annot(
        Term::pair(Term::Zero, Term::refl(Type::Nat, Term::Zero)),
        syntactic.clone(),
    );
    assert_eq!(type_of(&Context::new(), &pair), Ok(syntactic));
}

/// 4.10.26h AC1: spans around the pair are transparent to the rule.
#[test]
fn a_spanned_pair_is_still_a_dependent_pair() {
    let spanned = Term::annot(
        Term::spanned(
            Term::pair(Term::Zero, Term::refl(Type::Nat, Term::var("n"))),
            crate::terms::TermSpan::new(0, 1),
        ),
        zero_witness_statement(),
    );
    assert_eq!(type_of(&over_n(), &spanned), Ok(zero_witness_statement()));
}

/// 4.10.26h AC1: `fst p : A`.
#[test]
fn fst_types_as_the_binder_type() {
    assert_eq!(
        type_of(&over_p(), &Term::fst(Term::var("p"))),
        Ok(Type::Nat)
    );
}

/// 4.10.26h AC1: `snd p : B[x := fst p]`, with `p` as written.
#[test]
fn snd_types_at_the_projected_instance() {
    let fst_p = || Term::fst(Term::var("p"));
    assert_eq!(
        type_of(&over_p(), &Term::snd(Term::var("p"))),
        Ok(eq_nat(fst_p(), fst_p()))
    );
}

/// 4.10.26h AC1 (D4): `fst (Annot((a, b), Σ)) ≡ a`, through the annotation.
#[test]
fn fst_of_an_annotated_pair_converts_to_its_witness() {
    assert!(converts(&Term::fst(witnessed_by(Term::Zero)), &Term::Zero));
    assert!(!converts(
        &Term::fst(witnessed_by(Term::Zero)),
        &Term::succ(Term::Zero)
    ));
}

/// 4.10.26h AC1 (D4): `snd (Annot((a, b), Σ)) ≡ b`.
#[test]
fn snd_of_an_annotated_pair_converts_to_its_evidence() {
    let evidence = Term::refl(Type::Nat, Term::var("n"));
    assert!(converts(&Term::snd(witnessed_by(Term::Zero)), &evidence));
}

/// 4.10.26h AC1: a neutral pair re-annotated under another binder name still
/// checks, and its `snd` converts to the instance under either spelling —
/// the equality arms rename both binders to one.
#[test]
fn snd_of_a_neutral_pair_converts_to_its_alpha_renamed_instance() {
    let renamed = reflexive_statement("j");
    assert_eq!(
        type_of(&over_p(), &Term::annot(Term::var("p"), renamed.clone())),
        Ok(renamed)
    );
    let fst_p = || Term::fst(Term::var("p"));
    assert_eq!(
        check(
            &over_p(),
            &Term::snd(Term::var("p")),
            &eq_nat(fst_p(), fst_p())
        ),
        Ok(())
    );
}

/// 4.10.26h AC1: `check_proof` admits `λn:Nat. Annot((0, refl n), Σ…)` with
/// no global unfolded.
#[test]
fn check_proof_admits_a_hand_built_witness() {
    let proof = Term::lambda("n", Type::Nat, witnessed_by(Term::Zero));
    let statement = Type::pi("n", Type::Nat, zero_witness_statement());
    assert_eq!(check_proof(&Context::new(), &proof, &statement), Ok(()));
}
