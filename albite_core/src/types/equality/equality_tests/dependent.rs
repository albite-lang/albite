//! Equality of dependent function types (ADR 30.9.26d AC 5, D4): the binder
//! is compared up to renaming in both equalities, and a `Pi` equals an
//! `Arrow` exactly when it is non-dependent. Dependent pairs follow, with the
//! same binder rule and no product twin (ADR 4.10.26h).

use super::*;
use crate::terms::termination::globals_in_type;
use crate::terms::Term;

fn reflexive_pi(binder: &str) -> Type {
    Type::pi(
        binder,
        Type::Nat,
        Type::eq(Type::Nat, Term::var(binder), Term::var(binder)),
    )
}

fn both_equal(left: &Type, right: &Type) -> (bool, bool) {
    (
        types_equal_alpha(left, right),
        types_equal_strict(left, right),
    )
}

// 30.9.26d AC5: two `Pi`s differing only in binder name are equal in both.
#[test]
fn pis_differing_only_in_binder_name_are_equal() {
    assert_eq!(
        both_equal(&reflexive_pi("n"), &reflexive_pi("m")),
        (true, true)
    );
    assert_eq!(
        both_equal(&reflexive_pi("n"), &reflexive_pi("n")),
        (true, true)
    );
}

// 30.9.26d AC5: different codomains, or different domains, are unequal.
#[test]
fn pis_with_different_codomains_or_domains_are_unequal() {
    let other_cod = Type::pi(
        "n",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("n"), Term::Zero),
    );
    assert_eq!(both_equal(&reflexive_pi("n"), &other_cod), (false, false));
    let other_dom = Type::pi(
        "n",
        Type::Bool,
        Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
    );
    assert_eq!(both_equal(&reflexive_pi("n"), &other_dom), (false, false));
}

// 30.9.26d AC5: a binder renamed on one side only is not the same binder: a
// free `m` in one codomain is not the other's bound `n`.
#[test]
fn a_free_variable_is_not_the_other_sides_binder() {
    let free_m = Type::pi(
        "n",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("m"), Term::var("n")),
    );
    let bound_m = Type::pi(
        "m",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("m"), Term::var("m")),
    );
    assert_eq!(both_equal(&free_m, &bound_m), (false, false));
}

// 30.9.26d AC5: a non-dependent `Pi` equals its `Arrow`, in either order; a
// dependent one does not.
#[test]
fn a_non_dependent_pi_equals_its_arrow_only() {
    let closed = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let non_dependent = Type::pi("n", Type::Nat, closed.clone());
    let arrow = Type::arrow(Type::Nat, closed);
    assert_eq!(both_equal(&non_dependent, &arrow), (true, true));
    assert_eq!(both_equal(&arrow, &non_dependent), (true, true));
    let dependent_arrow = Type::arrow(
        Type::Nat,
        Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
    );
    assert_eq!(
        both_equal(&reflexive_pi("n"), &dependent_arrow),
        (false, false)
    );
    assert_eq!(
        both_equal(&dependent_arrow, &reflexive_pi("n")),
        (false, false)
    );
}

// 30.9.26d AC5: admission finds a global named only inside a `Pi`'s codomain.
#[test]
fn globals_in_type_reaches_a_pi_codomain() {
    let cites = Type::pi(
        "n",
        Type::Nat,
        Type::eq(
            Type::Nat,
            Term::app(Term::global("spin"), Term::var("n")),
            Term::var("n"),
        ),
    );
    assert!(globals_in_type(&cites).contains("spin"));
    let in_domain = Type::pi(
        "n",
        Type::eq(Type::Nat, Term::global("dom"), Term::Zero),
        Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
    );
    assert!(globals_in_type(&in_domain).contains("dom"));
}

// ── Dependent pairs (ADR 4.10.26h AC 2, D1): the binder is compared up to
// renaming in both equalities, and a `Sigma` never equals a `Product` — not
// even a non-dependent one.

fn reflexive_sigma(binder: &str) -> Type {
    Type::sigma(
        binder,
        Type::Nat,
        Type::eq(Type::Nat, Term::var(binder), Term::var(binder)),
    )
}

// 4.10.26h AC2: two `Sigma`s differing only in binder name are equal in both.
#[test]
fn sigmas_differing_only_in_binder_name_are_equal() {
    assert_eq!(
        both_equal(&reflexive_sigma("k"), &reflexive_sigma("j")),
        (true, true)
    );
    let other_body = Type::sigma(
        "k",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("k"), Term::Zero),
    );
    assert_eq!(
        both_equal(&reflexive_sigma("k"), &other_body),
        (false, false)
    );
    let other_binder_type = Type::sigma(
        "k",
        Type::Bool,
        Type::eq(Type::Nat, Term::var("k"), Term::var("k")),
    );
    assert_eq!(
        both_equal(&reflexive_sigma("k"), &other_binder_type),
        (false, false)
    );
}

// 4.10.26h AC2 (D1): a `Sigma` and a `Pi` over the same parts are unequal.
#[test]
fn a_sigma_is_not_a_pi() {
    assert_eq!(
        both_equal(&reflexive_sigma("n"), &reflexive_pi("n")),
        (false, false)
    );
}

// 4.10.26h AC1/AC2 (D1): a non-dependent `Sigma` is not its `Product`, in
// either mode and in either argument order.
#[test]
fn a_non_dependent_sigma_is_not_a_product() {
    let closed = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let sigma = Type::sigma("k", Type::Nat, closed.clone());
    let product = Type::product(Type::Nat, closed);
    assert_eq!(both_equal(&sigma, &product), (false, false));
    assert_eq!(both_equal(&product, &sigma), (false, false));
}

// 6.10.26h D6: a `PredApp`'s arguments decide its equality, modulo source
// spans; a `ForallPred`'s domains decide too, even when the bodies agree.
#[test]
fn predicate_forms_compare_their_arguments_and_domains() {
    use crate::terms::TermSpan;
    let app = |arg: Term| Type::pred_app("P", vec![arg]);
    let spanned = Term::spanned(Term::Zero, TermSpan::new(3, 4));
    for equal in [types_equal_alpha, crate::types::types_equal_strict] {
        assert!(equal(&app(Term::Zero), &app(Term::Zero)));
        assert!(!equal(&app(Term::Zero), &app(Term::succ(Term::Zero))));
        assert!(equal(&app(Term::Zero), &app(spanned.clone())));
        let over = |domain: Type| Type::forall_pred("P", vec![domain], Type::Unit);
        assert!(equal(&over(Type::Nat), &over(Type::Nat)));
        assert!(!equal(&over(Type::Nat), &over(Type::Bool)));
        let abs = |domain: Type| Type::pred_abs(vec![("n".into(), domain)], Type::Unit);
        assert!(!equal(&abs(Type::Nat), &abs(Type::Bool)));
    }
}
