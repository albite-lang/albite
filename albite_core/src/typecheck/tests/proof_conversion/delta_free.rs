//! ADR 1.10.26c AC 1 and AC 3: proof mode converts `Eq` witnesses, only proof
//! mode, never across an overflow, never by opening a definition.

use super::*;

/// `Π n:Nat. Eq Nat (n + 0) n`
fn right_zero_statement() -> Type {
    Type::pi("n", Type::Nat, eq_nat(plus(var("n"), Term::Zero), var("n")))
}

/// `λn:Nat. refl (n + 0)`
fn right_zero_proof() -> Term {
    Term::lambda("n", Type::Nat, refl_nat(plus(var("n"), Term::Zero)))
}

fn is_mismatch(verdict: &TypeResult<()>) -> bool {
    matches!(verdict, Err(TypeError::TypeMismatch { .. }))
}

// The must-fail twins below lean on `is_mismatch`; an acceptance or a
// different refusal must not read as the mismatch they assert.
#[test]
fn is_mismatch_names_only_a_type_mismatch() {
    let mismatch = TypeError::TypeMismatch {
        expected: Type::Nat,
        got: Type::Bool,
    };
    assert!(is_mismatch(&Err(mismatch)));
    assert!(!is_mismatch(&Ok(())));
    assert!(!is_mismatch(&Err(TypeError::SorryInProof)));
}

// 1.10.26c AC1
#[test]
fn proof_mode_converts_an_eq_under_a_free_variable() {
    assert_eq!(
        check_proof(
            &Context::new(),
            &right_zero_proof(),
            &right_zero_statement()
        ),
        Ok(())
    );
}

// 1.10.26c AC1: the base `refl (0 + 0)` meets `Eq Nat (0 + 0) 0` by
// conversion, so the first refusal is the step's — which needs a `cong`.
#[test]
fn proof_mode_converts_the_natind_base() {
    let motive = Type::arrow(Type::Nat, eq_nat(plus(Term::Zero, var("k")), var("k")));
    let hypothesis = eq_nat(plus(Term::Zero, var("j")), var("j"));
    let induction = Term::natind(
        "k",
        motive,
        refl_nat(plus(Term::Zero, Term::Zero)),
        step(hypothesis.clone(), var("h")),
        var("n"),
    );
    let statement = eq_nat(plus(Term::Zero, var("n")), var("n"));
    let successor = eq_nat(plus(Term::Zero, Term::succ(var("j"))), Term::succ(var("j")));
    assert_eq!(
        check_proof(&over_n(), &induction, &statement),
        Err(TypeError::TypeMismatch {
            expected: successor,
            got: hypothesis,
        })
    );
    // With `k + 0`, every case converts and the whole induction is admitted.
    let statement = eq_nat(plus(var("n"), Term::Zero), var("n"));
    assert_eq!(
        check_proof(&over_n(), &right_zero_induction(), &statement),
        Ok(())
    );
}

// 1.10.26c AC1: `n * 1` reaches `0 + n` by the `*` and `+` rules alone.
#[test]
fn proof_mode_converts_a_cited_multiplication() {
    let cited = Type::pi("m", Type::Nat, eq_nat(plus(Term::Zero, var("m")), var("m")));
    let ctx = Context::new().with_global("zero_add", cited, GlobalAdmission::Admitted);
    let citation = Term::lambda(
        "n",
        Type::Nat,
        Term::app(Term::Global("zero_add".into()), var("n")),
    );
    let statement = Type::pi(
        "n",
        Type::Nat,
        eq_nat(Term::nat_mul(var("n"), Term::NatLit(1)), var("n")),
    );
    assert_eq!(check_proof(&ctx, &citation, &statement), Ok(()));
}

// 1.10.26c AC1: conversion decides, it does not accept.
#[test]
fn a_false_equality_is_still_refused() {
    let off_by_one = Type::pi(
        "n",
        Type::Nat,
        eq_nat(plus(var("n"), Term::NatLit(1)), var("n")),
    );
    let proof = Term::lambda("n", Type::Nat, refl_nat(plus(var("n"), Term::NatLit(1))));
    assert!(is_mismatch(&check_proof(
        &Context::new(),
        &proof,
        &off_by_one
    )));

    let four = || plus(Term::NatLit(2), Term::NatLit(2));
    let verdict = check_proof(
        &Context::new(),
        &refl_nat(four()),
        &eq_nat(four(), Term::NatLit(5)),
    );
    assert!(is_mismatch(&verdict));

    // D4: a saturating engine would equate these.
    let overflow = || plus(Term::NatLit(u64::MAX), Term::NatLit(1));
    let saturated = eq_nat(overflow(), Term::NatLit(u64::MAX));
    let verdict = check_proof(&Context::new(), &refl_nat(overflow()), &saturated);
    assert!(is_mismatch(&verdict));
}

// 1.10.26c AC1: a type variable still does not equal a `μ` binding its name.
#[test]
fn conversion_does_not_reopen_the_strict_shortcuts() {
    let stream = Type::mu("a", Type::product(Type::Nat, Type::TyVar("a".into())));
    let ctx = Context::new()
        .with_type_var("a")
        .with_term("x", Type::TyVar("a".into()));
    assert_eq!(check(&ctx, &var("x"), &stream), Ok(()));
    assert!(is_mismatch(&check_proof(&ctx, &var("x"), &stream)));
}

// 1.10.26c AC1 (D2): under a `∀`, the `Eq` arm stays strict.
#[test]
fn conversion_stays_out_from_under_a_type_binder() {
    let under_forall = |right: Term| Type::forall("a", eq_nat(plus(var("n"), Term::Zero), right));
    let ctx = over_n().with_term("h", under_forall(plus(var("n"), Term::Zero)));
    let verdict = check_proof(&ctx, &var("h"), &under_forall(var("n")));
    assert!(is_mismatch(&verdict));
    // The same pair with no binder open converts.
    let bare = over_n().with_term(
        "h",
        eq_nat(plus(var("n"), Term::Zero), plus(var("n"), Term::Zero)),
    );
    let open = eq_nat(plus(var("n"), Term::Zero), var("n"));
    assert_eq!(check_proof(&bare, &var("h"), &open), Ok(()));
}

// 1.10.26c AC1 (D3's guard): by-name `let` would drop the extern call.
#[test]
fn an_effectful_witness_is_not_converted() {
    let effectful = Term::let_in(
        "u",
        Type::Unit,
        Term::extern_call("tg_effect", vec![]),
        var("n"),
    );
    let ctx = over_n().with_term("h", eq_nat(var("n"), var("n")));
    let verdict = check_proof(&ctx, &var("h"), &eq_nat(effectful, var("n")));
    assert!(is_mismatch(&verdict));
    // The pure `let` converts, so it is the guard that refused.
    let pure = Term::let_in("u", Type::Unit, Term::Unit, var("n"));
    assert_eq!(
        check_proof(&ctx, &var("h"), &eq_nat(pure, var("n"))),
        Ok(())
    );
}

// 1.10.26c AC1: the witnesses convert, the proposition is false, and
// deciding it costs more than the budget — a refusal, not a hang.
#[test]
fn a_conversion_that_exhausts_its_budget_is_refused() {
    let count = 2_000_000;
    let successor = Term::lambda(
        "_",
        Type::Nat,
        Term::lambda("a", Type::Nat, Term::succ(var("a"))),
    );
    let long = Term::natrec(Type::Nat, Term::Zero, successor, Term::NatLit(count));
    let ctx = Context::new().with_term("h", eq_nat(long, Term::Zero));
    let verdict = check_proof(&ctx, &var("h"), &eq_nat(Term::NatLit(count), Term::Zero));
    assert!(is_mismatch(&verdict));
}

// 1.10.26c AC1 (D1's asymmetry): executable mode keeps its shortcuts and
// gains no conversion. Also 1.10.26c AC5: the executable judgment the
// existing kernel suites run is unchanged.
#[test]
fn executable_mode_does_not_convert() {
    let verdict = check(
        &Context::new(),
        &right_zero_proof(),
        &right_zero_statement(),
    );
    assert!(is_mismatch(&verdict));
}

fn two_is(admission: GlobalAdmission) -> Context {
    Context::new()
        .with_global("two", Type::Nat, admission)
        .with_term("h", eq_nat(Term::Global("two".into()), Term::NatLit(2)))
}

// 1.10.26c AC3: an admitted, citable global is still not opened.
#[test]
fn no_global_is_unfolded() {
    let verdict = check_proof(
        &two_is(GlobalAdmission::Admitted),
        &var("h"),
        &eq_nat(Term::NatLit(2), Term::NatLit(2)),
    );
    assert!(is_mismatch(&verdict));
}

// 1.10.26c AC3
#[test]
fn an_opaque_global_is_not_unfolded() {
    let verdict = check_proof(
        &two_is(GlobalAdmission::Opaque),
        &var("h"),
        &eq_nat(Term::NatLit(2), Term::NatLit(2)),
    );
    assert!(is_mismatch(&verdict));
}

// 1.10.26c AC3: the `+` rule needs no body, so a global is neutral and
// compared by name.
#[test]
fn globals_compare_by_name() {
    let two = || Term::Global("two".into());
    let ctx = Context::new().with_global("two", Type::Nat, GlobalAdmission::Admitted);
    let statement = eq_nat(plus(two(), Term::Zero), two());
    assert_eq!(check_proof(&ctx, &refl_nat(two()), &statement), Ok(()));
}
