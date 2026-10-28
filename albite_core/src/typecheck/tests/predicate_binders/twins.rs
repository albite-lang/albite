//! D9's twelve kernel twins (ADR 6.10.26h AC 1): each shape Q6 rules out,
//! built in Core and refused with its own `TypeError`.

use super::*;

fn nat_kind() -> Vec<Type> {
    vec![Type::Nat]
}

/// `∀P : Nat → Prop. P<0>`, a minimal well-formed predicate binder.
fn small_binder() -> Type {
    Type::forall_pred("P", nat_kind(), pred("P", vec![Term::Zero]))
}

fn identity_forall() -> Type {
    Type::forall(
        "a",
        Type::arrow(Type::TyVar("a".into()), Type::TyVar("a".into())),
    )
}

// Twin 1: a `ForallPred` under an arrow is not prenex.
#[test]
fn twin_1_a_predicate_binder_under_an_arrow_is_refused() {
    let statement = Type::arrow(Type::Nat, small_binder());
    assert!(matches!(
        check_signature_wf(&Context::new(), &statement),
        Err(TypeError::PredicateBinderNotPrenex { .. })
    ));
}

// Twin 2: an empty kind — a variable ranging over propositions (Q5's fourth).
#[test]
fn twin_2_an_empty_kind_is_refused() {
    let statement = Type::forall_pred("P", vec![], Type::Unit);
    assert_eq!(
        check_signature_wf(&Context::new(), &statement),
        Err(TypeError::PredicateKindEmpty { var: "P".into() })
    );
}

// Twin 3: a domain that sorts `Prop`.
#[test]
fn twin_3_a_prop_sorted_domain_is_refused() {
    let domain = eq_nat(Term::Zero, Term::Zero);
    let statement = Type::forall_pred("P", vec![domain.clone()], Type::Unit);
    assert_eq!(
        check_signature_wf(&Context::new(), &statement),
        Err(TypeError::PredicateDomainNotData {
            var: "P".into(),
            domain
        })
    );
}

// Twin 4: a domain naming an earlier `Forall`'s variable, which `α := Eq<…>`
// would make `Prop`-sorted.
#[test]
fn twin_4_a_domain_naming_an_earlier_type_variable_is_refused() {
    let alpha = Type::TyVar("a".into());
    let statement = Type::forall("a", Type::forall_pred("P", vec![alpha.clone()], Type::Unit));
    assert_eq!(
        check_signature_wf(&Context::new(), &statement),
        Err(TypeError::PredicateDomainOpen {
            var: "P".into(),
            domain: alpha
        })
    );
}

// Twin 5: a stray `PredAbs` as an annotation's type.
#[test]
fn twin_5_a_stray_predicate_as_an_annotation_is_refused() {
    let term = Term::annot(Term::Unit, is_zero());
    assert!(matches!(
        type_of(&Context::new(), &term),
        Err(TypeError::StrayPredicate { .. })
    ));
}

// Twin 6: `λh: ∀P. …. h` — a predicate binder as a λ's annotation.
#[test]
fn twin_6_a_predicate_binder_as_a_lambda_annotation_is_refused() {
    let term = Term::lambda("h", small_binder(), Term::var("h"));
    assert!(matches!(
        type_of(&Context::new(), &term),
        Err(TypeError::PredicateBinderNotPrenex { .. })
    ));
}

// Twin 7: `id [∀P. …]` — an impredicative instantiation at a kind.
#[test]
fn twin_7_instantiating_a_forall_at_a_predicate_binder_is_refused() {
    let ctx = Context::new().with_term("id", identity_forall());
    let term = Term::ty_app(Term::var("id"), small_binder());
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::PredicateBinderNotPrenex { .. })
    ));
}

// Twin 8: a `PredAbs` instantiating a plain `Forall`.
#[test]
fn twin_8_a_predicate_instantiating_a_plain_forall_is_refused() {
    let ctx = Context::new().with_term("id", identity_forall());
    let term = Term::ty_app(Term::var("id"), is_zero());
    assert_eq!(
        type_of(&ctx, &term),
        Err(TypeError::PredicateInstantiatesForall { got: is_zero() })
    );
}

// Twin 9: `Nat` instantiating a `ForallPred`.
#[test]
fn twin_9_a_data_type_instantiating_a_predicate_binder_is_refused() {
    let term = Term::ty_app(Term::global("pred_transport"), Type::Nat);
    assert_eq!(
        type_of(&with_transport(), &term),
        Err(TypeError::NotAPredicate { got: Type::Nat })
    );
}

// Twin 10: `P<0>` with `P` bound nowhere.
#[test]
fn twin_10_an_unbound_predicate_variable_is_refused() {
    assert_eq!(
        check_type_wf(&Context::new(), &pred("P", vec![Term::Zero])),
        Err(TypeError::UnboundPredicateVar("P".into()))
    );
}

// Twin 11: `P<0>` with `P` bound kind-less by `with_type_var` — distinct from
// twin 10, so a kind-less binding is never mistaken for a kind.
#[test]
fn twin_11_a_type_variable_applied_is_refused() {
    let ctx = Context::new().with_type_var("P");
    assert_eq!(
        check_type_wf(&ctx, &pred("P", vec![Term::Zero])),
        Err(TypeError::PredicateVarIsTypeVar("P".into()))
    );
}

// Twin 12: `P<0, 0>` against a one-domain kind.
#[test]
fn twin_12_an_arity_mismatch_is_refused() {
    let ctx = Context::new().with_pred_var("P", nat_kind());
    assert_eq!(
        check_type_wf(&ctx, &pred("P", vec![Term::Zero, Term::Zero])),
        Err(TypeError::PredicateArity {
            var: "P".into(),
            expected: 1,
            got: 2
        })
    );
}

// The arguments are typed against the domains in every mode (D6).
#[test]
fn an_ill_typed_predicate_argument_is_refused_outside_proof_mode() {
    let ctx = Context::new().with_pred_var("P", nat_kind());
    assert!(!ctx.is_proof());
    assert!(check_type_wf(&ctx, &pred("P", vec![Term::True])).is_err());
    assert_eq!(check_type_wf(&ctx, &pred("P", vec![Term::Zero])), Ok(()));
}

// A predicate whose parameters are not the kind's domains, or whose body is
// data, does not instantiate.
#[test]
fn a_predicate_of_the_wrong_kind_is_refused() {
    let wrong_domain = Type::pred_abs(
        vec![("n".into(), Type::Bool)],
        eq_nat(Term::Zero, Term::Zero),
    );
    let data_body = Type::pred_abs(vec![("n".into(), Type::Nat)], Type::Nat);
    for predicate in [wrong_domain, data_body] {
        let term = Term::ty_app(Term::global("pred_transport"), predicate);
        assert!(matches!(
            type_of(&with_transport(), &term),
            Err(TypeError::PredicateKindMismatch { .. })
        ));
    }
}

// An annotation with a predicate prefix needs a `TyAbs` per binder.
#[test]
fn a_predicate_prefix_without_its_abstraction_is_refused() {
    let term = Term::annot(Term::Unit, small_binder());
    assert!(matches!(
        type_of(&Context::new(), &term),
        Err(TypeError::PredicateIntroShape { .. })
    ));
}

// Each refusal renders its own message, naming what it refused (D6).
#[test]
fn each_predicate_refusal_renders_its_own_message() {
    let messages = [
        TypeError::PredicateBinderNotPrenex { ty: Type::Nat },
        TypeError::StrayPredicate { ty: Type::Nat },
        TypeError::PredicateKindEmpty { var: "Pq".into() },
        TypeError::PredicateDomainNotData {
            var: "Pq".into(),
            domain: Type::Nat,
        },
        TypeError::PredicateDomainOpen {
            var: "Pq".into(),
            domain: Type::Nat,
        },
        TypeError::PredicateInstantiatesForall { got: Type::Nat },
        TypeError::NotAPredicate { got: Type::Nat },
        TypeError::PredicateKindMismatch {
            expected: vec![Type::Nat],
            got: Type::Bool,
        },
        TypeError::UnboundPredicateVar("Pq".into()),
        TypeError::PredicateVarIsTypeVar("Pq".into()),
        TypeError::PredicateArity {
            var: "Pq".into(),
            expected: 1,
            got: 2,
        },
        TypeError::PredicateIntroShape { ty: Type::Nat },
    ]
    .map(|refusal| refusal.to_string());
    for (index, message) in messages.iter().enumerate() {
        assert!(!messages[..index].contains(message), "duplicate: {message}");
    }
    assert!(messages[2].contains("`Pq`") && messages[2].contains("propositions"));
    assert!(messages[10].contains("takes 1 argument(s), got 2"));
}
