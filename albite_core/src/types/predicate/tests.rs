//! Tests for the predicate forms' type-level helpers (ADR 6.10.26h D4):
//! display, free variables, equality up to α, and substitution's arms.

use crate::terms::Term;
use crate::types::{types_equal_alpha, types_equal_strict, Type};

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

fn binder_over(var: &str) -> Type {
    Type::forall_pred(var, vec![Type::Nat], Type::pred_app(var, vec![Term::Zero]))
}

fn abs_over(param: &str) -> Type {
    Type::pred_abs(
        vec![(param.into(), Type::Nat)],
        eq_nat(Term::var(param), Term::Zero),
    )
}

#[test]
fn each_form_displays_its_source_like_spelling() {
    assert_eq!(
        Type::pred_app("P", vec![Term::Zero, Term::var("n")]).to_string(),
        "P<zero, n>"
    );
    assert_eq!(binder_over("P").to_string(), "∀P: Nat → Prop. P<zero>");
    assert_eq!(abs_over("n").to_string(), "λ(n: Nat). Eq Nat n zero");
    assert!(binder_over("P")
        .display_detailed()
        .starts_with("∀P: Nat → Prop."));
}

#[test]
fn free_type_variables_see_the_predicate_and_respect_its_binder() {
    let app = Type::pred_app("P", vec![Term::Zero]);
    assert!(app.free_type_vars().contains("P"));
    assert!(binder_over("P").free_type_vars().is_empty());
    let open = Type::forall_pred("P", vec![Type::TyVar("a".into())], Type::Unit);
    assert!(open.free_type_vars().contains("a"));
    let abs = Type::pred_abs(vec![("n".into(), Type::TyVar("b".into()))], Type::Unit);
    assert!(abs.free_type_vars().contains("b"));
}

#[test]
fn each_form_is_equal_up_to_renaming_and_no_further() {
    for equal in [types_equal_alpha, types_equal_strict] {
        assert!(equal(&binder_over("P"), &binder_over("Q")));
        assert!(equal(&abs_over("n"), &abs_over("m")));
        assert!(!equal(&binder_over("P"), &abs_over("n")));
        assert!(!equal(
            &Type::pred_app("P", vec![Term::Zero]),
            &Type::pred_app("P", vec![Term::Zero, Term::Zero])
        ));
        assert!(!equal(
            &Type::pred_app("P", vec![Term::Zero]),
            &Type::pred_app("Q", vec![Term::Zero])
        ));
        let other_domain = Type::forall_pred("P", vec![Type::Bool], Type::pred_app("P", vec![]));
        assert!(!equal(&binder_over("P"), &other_domain));
        let other_param = Type::pred_abs(
            vec![("n".into(), Type::Bool)],
            eq_nat(Term::var("n"), Term::Zero),
        );
        assert!(!equal(&abs_over("n"), &other_param));
    }
}

#[test]
fn substitution_renames_shadows_and_poisons() {
    let app = Type::pred_app("P", vec![Term::Zero]);
    assert_eq!(
        app.substitute("P", &Type::TyVar("Q".into())),
        Type::pred_app("Q", vec![Term::Zero])
    );
    assert_eq!(app.substitute("P", &Type::Error), Type::Error);
    assert_eq!(app.substitute("R", &Type::Nat), app);
    // A `ForallPred` shadows its own variable, and substitutes in its domains.
    assert_eq!(
        binder_over("P").substitute("P", &abs_over("n")),
        binder_over("P")
    );
    let open = Type::forall_pred("P", vec![Type::TyVar("a".into())], Type::Unit);
    assert_eq!(
        open.substitute("a", &Type::Nat),
        Type::forall_pred("P", vec![Type::Nat], Type::Unit)
    );
    let abs = Type::pred_abs(
        vec![("n".into(), Type::TyVar("a".into()))],
        Type::TyVar("a".into()),
    );
    assert_eq!(
        abs.substitute("a", &Type::Nat),
        Type::pred_abs(vec![("n".into(), Type::Nat)], Type::Nat)
    );
}

#[test]
#[should_panic(expected = "instantiated at a non-predicate")]
fn substituting_a_data_type_for_a_predicate_variable_panics() {
    let _ = Type::pred_app("P", vec![Term::Zero]).substitute("P", &Type::Nat);
}

// 6.10.26h D3: a predicate applied sorts `Prop`, and a binder sorts as its
// body; the instantiating predicate is no type with inhabitants.
#[test]
fn each_predicate_form_sorts_as_designed() {
    use crate::types::Sort;
    assert_eq!(Type::pred_app("P", vec![]).sort_of(), Some(Sort::Prop));
    assert_eq!(binder_over("P").sort_of(), Some(Sort::Prop));
    assert_eq!(abs_over("n").sort_of(), None);
    assert!(binder_over("P").has_predicate_prefix());
    assert!(Type::forall("a", binder_over("P")).has_predicate_prefix());
    assert!(!Type::arrow(Type::Nat, binder_over("P")).has_predicate_prefix());
}

#[test]
fn mentions_predicate_form_finds_a_nested_form() {
    assert!(Type::arrow(Type::Nat, Type::pred_app("P", vec![])).mentions_predicate_form());
    assert!(!Type::arrow(Type::Nat, Type::Nat).mentions_predicate_form());
}
