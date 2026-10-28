//! Tests for `natind` and `natrec` surface syntax (ADR 22.5.26a).

use albite_core::{Term, Type};

use crate::elaborate::exprs::natind_motive::{
    motive_instance, motive_is_dependent, reverse_capture, successor_instance,
};
use crate::elaborate::tests::{elab_err, elab_ok};

// ============================================================================
// natrec — positive tests
// ============================================================================

#[test]
fn natrec_basic() {
    elab_ok("fn f(n: Nat) -> Nat { natrec(Nat, 0, fn(k: Nat, acc: Nat) => acc, n) }");
}

// ============================================================================
// natrec — negative tests
// ============================================================================

#[test]
fn natrec_target_not_nat() {
    let errors =
        elab_err("fn f(b: Bool) -> Nat { natrec(Nat, 0, fn(k: Nat, acc: Nat) => acc, b) }");
    assert!(!errors.is_empty());
}

#[test]
fn natrec_base_type_mismatch() {
    let errors =
        elab_err("fn f(n: Nat) -> Nat { natrec(Nat, true, fn(k: Nat, acc: Nat) => acc, n) }");
    assert!(!errors.is_empty());
}

// ============================================================================
// natind — positive tests
// ============================================================================

#[test]
fn natind_basic() {
    elab_ok("fn f(n: Nat) -> Nat { natind(|k: Nat| Nat, 0, fn(k: Nat, ih: Nat) => ih, n) }");
}

// ============================================================================
// natind — negative tests
// ============================================================================

#[test]
fn natind_motive_not_lambda() {
    // Use a numeric literal as motive — it should parse as MotiveExpr then fail in elaboration
    let errors = elab_err("fn f(n: Nat) -> Nat { natind(0, 0, fn(k: Nat, ih: Nat) => ih, n) }");
    assert!(!errors.is_empty());
}

#[test]
fn natind_motive_domain_not_nat() {
    let errors =
        elab_err("fn f(n: Nat) -> Nat { natind(|k: Bool| Nat, 0, fn(k: Nat, ih: Nat) => ih, n) }");
    assert!(!errors.is_empty());
}

#[test]
fn natind_target_not_nat() {
    let errors =
        elab_err("fn f(b: Bool) -> Nat { natind(|k: Nat| Nat, 0, fn(k: Nat, ih: Nat) => ih, b) }");
    assert!(!errors.is_empty());
}

#[test]
fn natind_base_type_mismatch() {
    let errors = elab_err(
        "fn f(n: Nat) -> Nat { natind(|k: Nat| Nat, true, fn(k: Nat, ih: Nat) => ih, n) }",
    );
    assert!(!errors.is_empty());
}

// ============================================================================
// Dependent motive — the pure helpers (ADR 29.9.26d AC 6)
// ============================================================================

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// 29.9.26d AC6: a motive is dependent when its binder is free in the body.
#[test]
fn a_motive_that_mentions_its_binder_is_dependent() {
    assert!(motive_is_dependent(
        "k",
        &eq_nat(Term::var("k"), Term::var("k"))
    ));
    assert!(!motive_is_dependent("k", &eq_nat(Term::Zero, Term::Zero)));
    assert!(!motive_is_dependent("k", &Type::Nat));
}

/// 29.9.26d AC6: a `k` bound by an inner `natind`'s own motive does not count.
#[test]
fn a_binder_rebound_by_an_inner_natind_does_not_make_a_motive_dependent() {
    let inner = Term::natind(
        "k",
        Type::arrow(Type::Nat, eq_nat(Term::var("k"), Term::var("k"))),
        Term::Zero,
        Term::Zero,
        Term::Zero,
    );
    assert!(!motive_is_dependent(
        "k",
        &eq_nat(inner.clone(), Term::Zero)
    ));
    assert!(motive_is_dependent("k", &eq_nat(inner, Term::var("k"))));
}

/// 29.9.26d AC6: the three instances — zero, succ(j), and the target.
#[test]
fn the_motive_is_instantiated_at_zero_the_successor_and_the_target() {
    let body = eq_nat(Term::nat_add(Term::Zero, Term::var("k")), Term::var("k"));
    assert_eq!(
        motive_instance("k", &body, &Term::Zero),
        eq_nat(Term::nat_add(Term::Zero, Term::Zero), Term::Zero)
    );
    let successor = Term::succ(Term::var("j"));
    assert_eq!(
        successor_instance("k", &body, "j"),
        eq_nat(Term::nat_add(Term::Zero, successor.clone()), successor)
    );
    assert_eq!(
        motive_instance("k", &body, &Term::var("n")),
        eq_nat(Term::nat_add(Term::Zero, Term::var("n")), Term::var("n"))
    );
}

/// 29.9.26d AC6: a step name the motive mentions other than as `k` is caught.
#[test]
fn a_step_name_the_motive_mentions_is_a_reverse_capture() {
    let body = eq_nat(
        Term::nat_add(Term::var("m"), Term::var("k")),
        Term::var("k"),
    );
    assert_eq!(
        reverse_capture("k", &body, &["m", "ih"]),
        Some("m".to_owned())
    );
    assert_eq!(
        reverse_capture("k", &body, &["ih", "m"]),
        Some("m".to_owned())
    );
    assert_eq!(reverse_capture("k", &body, &["k", "ih"]), None);
    assert_eq!(reverse_capture("k", &body, &["j", "ih"]), None);
}

// ============================================================================
// Dependent motive — the step-shape decision and the refusals (AC 6)
// ============================================================================

fn first_code(source: &str) -> &'static str {
    elab_err(source)
        .first()
        .map(|error| error.kind.code())
        .expect("at least one error")
}

/// 29.9.26d AC6: a literal two-parameter step under a dependent motive checks.
#[test]
fn a_dependent_motive_accepts_a_two_parameter_step() {
    elab_ok("fn f(n: Nat) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, refl, fn(k: Nat, ih) => refl, n); 0 }");
}

/// 29.9.26d AC6: a non-dependent motive keeps accepting a step that is not a lambda.
#[test]
fn a_non_dependent_motive_accepts_a_step_that_is_not_a_lambda() {
    elab_ok("fn f(n: Nat, s: Nat -> Nat -> Nat) -> Nat { natind(|k: Nat| Nat, 0, s, n) }");
}

/// 29.9.26d AC6: a dependent motive refuses a step that is not a lambda.
#[test]
fn a_dependent_motive_refuses_a_step_that_is_not_a_lambda() {
    assert_eq!(
        first_code("fn f(n: Nat, s: Nat) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, refl, s, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: the step shape is exactly two parameters. One or three,
/// under a dependent motive, is not a step `natind` can bind an IH for.
#[test]
fn a_dependent_motive_refuses_a_step_of_the_wrong_arity() {
    for step in ["fn(k: Nat) => refl", "fn(k: Nat, ih, extra: Nat) => refl"] {
        let source = format!(
            "fn f(n: Nat) -> Nat {{ let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, refl, {step}, n); 0 }}"
        );
        assert!(!elab_err(&source).is_empty(), "{step} was accepted");
    }
}

/// 29.9.26d AC6: a step returning its IH proves P(j), not P(succ j).
#[test]
fn a_step_that_returns_its_ih_is_e0010() {
    assert_eq!(
        first_code("fn f(n: Nat) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, refl, fn(k: Nat, ih) => ih, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: the base is checked at zero, where `0 == 1` fails.
#[test]
fn a_base_that_fails_at_zero_is_e0071() {
    assert_eq!(
        first_code("fn f(n: Nat) -> Nat { let _p: Eq<Nat, n, 1> = natind(|k: Nat| Eq<Nat, k, 1>, refl, fn(k: Nat, ih) => refl, n); 0 }"),
        "E0071"
    );
}

/// 29.9.26d AC6: the result is the motive at the target, not at zero.
#[test]
fn binding_the_result_at_zero_is_e0010() {
    assert_eq!(
        first_code("fn f(n: Nat) -> Nat { let _p: Eq<Nat, 0, 0> = natind(|k: Nat| Eq<Nat, k, k>, refl, fn(k: Nat, ih) => refl, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: a step parameter shadowing the motive's outer `m` is refused.
#[test]
fn a_step_parameter_shadowing_the_motive_is_e0010() {
    assert_eq!(
        first_code("fn f(m: Nat, n: Nat) -> Nat { let _p: Eq<Nat, (m + 1) + n, (m + n) + 1> = natind(|k: Nat| Eq<Nat, (m + 1) + k, (m + k) + 1>, refl, fn(m: Nat, ih) => cong(|x: Nat| x + 1, ih), n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: a written IH annotation must be the motive at the step's index.
#[test]
fn a_wrong_ih_annotation_is_e0010() {
    assert_eq!(
        first_code("fn f(n: Nat) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, refl, fn(k: Nat, ih: Eq<Nat, 0, 0>) => refl, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: the step's index annotation must be `Nat`.
#[test]
fn a_step_index_that_is_not_nat_is_e0010() {
    assert_eq!(
        first_code("fn f(n: Nat) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, refl, fn(k: Bool, ih) => refl, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: a case proof that is not an equality is refused against an `Eq` goal.
#[test]
fn a_case_proof_that_is_not_an_equality_is_e0010() {
    assert_eq!(
        first_code("fn f(n: Nat) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, 0, fn(k: Nat, ih) => refl, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: a case proof over another base type is refused.
#[test]
fn a_case_proof_over_another_base_type_is_e0010() {
    assert_eq!(
        first_code("fn f(n: Nat, b: Eq<Bool, true, true>) -> Nat { let _p: Eq<Nat, n, n> = natind(|k: Nat| Eq<Nat, k, k>, b, fn(k: Nat, ih) => refl, n); 0 }"),
        "E0010"
    );
}

/// 29.9.26d AC6: `zero_add` is accepted through `cong`, by conversion.
#[test]
fn zero_add_is_accepted_with_a_cong_step() {
    elab_ok("fn f(n: Nat) -> Nat { let _p: Eq<Nat, 0 + n, n> = natind(|k: Nat| Eq<Nat, 0 + k, k>, refl, fn(k: Nat, ih) => cong(|x: Nat| x + 1, ih), n); 0 }");
}

/// 29.9.26d AC6: the elaborated Core carries the motive's binder.
#[test]
fn the_elaborated_natind_carries_its_binder() {
    let defs =
        elab_ok("fn f(n: Nat) -> Nat { natind(|q: Nat| Nat, 0, fn(k: Nat, ih: Nat) => ih, n) }");
    let rendered = format!("{}", defs[0].term.term);
    assert!(rendered.contains("natind [q. "), "got {rendered}");
}
