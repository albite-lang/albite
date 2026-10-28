//! Erasure and term substitution through the predicate forms (ADR 6.10.26h
//! AC 4, D4, D7), beside `capture_tests.rs`'s `Sigma` pins.

use crate::terms::Term;
use crate::types::Type;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

fn p_of(arg: Term) -> Type {
    Type::pred_app("P", vec![arg])
}

/// 6.10.26h AC4: `PredApp(P, _)` erases to `TyVar(P)`, `ForallPred(P, _, B)`
/// to `Forall(P, |B|)` and `PredAbs(_, T)` to `|T|`; each is a dependent
/// former, and no erasure is.
#[test]
fn erase_dependency_maps_each_predicate_form() {
    let app = p_of(Term::var("n"));
    assert_eq!(app.erase_dependency(), Type::TyVar("P".into()));

    let binder = Type::forall_pred("P", vec![Type::Nat], Type::arrow(Type::Nat, app.clone()));
    assert_eq!(
        binder.erase_dependency(),
        Type::forall("P", Type::arrow(Type::Nat, Type::TyVar("P".into())))
    );

    let body = eq_nat(Term::var("n"), Term::Zero);
    let abs = Type::pred_abs(vec![("n".into(), Type::Nat)], body.clone());
    assert_eq!(abs.erase_dependency(), body);

    for form in [app, binder, abs] {
        assert!(form.contains_dependent_former(), "{form}");
        assert!(
            !form.erase_dependency().contains_dependent_former(),
            "{form}"
        );
    }
}

/// 6.10.26h AC4: erasure is unwidened — a `PredApp` is not a proposition to
/// `is_proposition` (4.10.26i D8), though it sorts `Prop`.
#[test]
fn a_pred_app_is_not_an_erasure_proposition() {
    let app = p_of(Term::Zero);
    assert!(!app.is_proposition());
    assert!(!Type::arrow(Type::Nat, app).is_proposition());
}

/// D4: a `PredApp`'s arguments substitute like `Eq`'s, and a `PredAbs`
/// binder shadows its own name and renames to avoid capture.
#[test]
fn term_substitution_reaches_predicate_arguments() {
    let app = p_of(Term::var("x"));
    assert_eq!(app.substitute_term("x", &Term::Zero), p_of(Term::Zero));
    assert!(app.free_term_vars().contains("x"));

    let shadowing = Type::pred_abs(vec![("x".into(), Type::Nat)], p_of(Term::var("x")));
    assert_eq!(shadowing.substitute_term("x", &Term::Zero), shadowing);
    assert!(shadowing.free_term_vars().is_empty());

    // λy. P<x>, [x := y]: the binder `y` must not capture the incoming `y`.
    let capturing = Type::pred_abs(vec![("y".into(), Type::Nat)], p_of(Term::var("x")));
    let got = capturing.substitute_term("x", &Term::var("y"));
    let Type::PredAbs(params, body) = &got else {
        panic!("expected a PredAbs, got {got}");
    };
    assert_ne!(params[0].0, "y");
    assert_eq!(**body, p_of(Term::var("y")));
    assert!(got.free_term_vars().contains("y"));
}
