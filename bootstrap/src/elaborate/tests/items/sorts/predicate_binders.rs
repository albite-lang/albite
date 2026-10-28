//! A definition can quantify over a predicate (ADR 6.10.26h AC 2, D1–D5):
//! the signature and Core the elaborator builds, and D9's elaboration twins,
//! each asserted by its own code.

use super::{def, elab_ok, first_code};
use crate::elaborate::error::{ElabErrorKind, KindRefusal, SortErrorKind};
use crate::elaborate::tests::elab_err;
use albite_core::{Term, Type};

const TRANSPORT: &str = "\
theorem pred_transport<P: Nat -> Prop>(a: Nat, b: Nat, e: Eq<Nat, a, b>, h: P<a>) -> P<b> {
    subst(e, |x: Nat| P<x>, h)
}";

const IS_ZERO: &str = "type IsZero(n: Nat): Prop = n == 0";

fn kind_refusal(source: &str) -> KindRefusal {
    match &elab_err(source)[0].kind {
        ElabErrorKind::Sort(SortErrorKind::PredicateKindRefused { why, .. }) => why.clone(),
        other => panic!("expected E0069 for `{source}`, got {other:?}"),
    }
}

// 6.10.26h AC2 (D4, D5): the signature folds a `ForallPred`, and the body is
// introduced by annotation, `Annot(ΛP. …, ∀P : Nat → Prop. …)`.
#[test]
fn a_kinded_parameter_folds_a_predicate_binder() {
    let defs = elab_ok(TRANSPORT);
    let transport = def(&defs, "pred_transport");
    let Type::ForallPred(pred, domains, matrix) = &transport.ty else {
        panic!("expected a ForallPred, got {}", transport.ty);
    };
    assert_eq!((pred.as_str(), domains.as_slice()), ("P", &[Type::Nat][..]));
    assert!(matrix.to_string().contains("P<a>"), "{matrix}");
    let mut term = &transport.term.term;
    while let Term::Spanned(inner, _) = term {
        term = inner;
    }
    let Term::Annot(body, annotation) = term else {
        panic!("expected an Annot, got {term:?}");
    };
    assert_eq!(annotation, &transport.ty);
    assert!(matches!(**body, Term::TyAbs(ref p, _) if p == "P"));
}

// 6.10.26h AC2 (D5): a citation at a family instantiates with its body.
#[test]
fn a_family_instantiates_a_predicate_parameter() {
    let source = format!(
        "{IS_ZERO}\n{TRANSPORT}\n\
         theorem z() -> IsZero<0> {{ pred_transport::<IsZero>(0, 0, refl, refl) }}"
    );
    let defs = elab_ok(&source);
    assert_eq!(
        def(&defs, "z").ty,
        Type::eq(Type::Nat, Term::Zero, Term::Zero)
    );
}

// 6.10.26h AC2 (D5): a predicate parameter in scope instantiates another.
#[test]
fn a_predicate_parameter_in_scope_instantiates_another() {
    let source = format!(
        "{TRANSPORT}\n\
         theorem via<Q: Nat -> Prop>(a: Nat, h: Q<a>) -> Q<a> {{ pred_transport::<Q>(a, a, refl, h) }}"
    );
    elab_ok(&source);
}

// Twin: `P<n>` with `P` unkinded stays 6.10.26b's refusal, `own_params: None`.
#[test]
fn twin_an_unkinded_parameter_applied_is_type_parameter_applied() {
    let errors = elab_err("fn f<P>(n: Nat, h: P<n>) -> Nat { n }");
    assert!(matches!(
        &errors[0].kind,
        ElabErrorKind::Sort(SortErrorKind::TypeParameterApplied {
            own_params: None,
            ..
        })
    ));
    assert_eq!(errors[0].kind.code(), "E0068");
}

// Twins: each kind D2 refuses, at E0069, each for its own reason.
#[test]
fn twin_each_refused_kind_is_e0069_for_its_reason() {
    assert!(matches!(
        kind_refusal("fn f<P: Prop>(n: Nat) -> Nat { n }"),
        KindRefusal::RangesOverPropositions
    ));
    assert!(matches!(
        kind_refusal("fn f<P: Eq<Nat, 0, 0> -> Prop>(n: Nat) -> Nat { n }"),
        KindRefusal::DomainIsProposition { .. }
    ));
    assert!(matches!(
        kind_refusal("fn f<T, P: T -> Prop>(n: Nat) -> Nat { n }"),
        KindRefusal::DomainNamesTypeParameter { ref domain } if domain == "T"
    ));
    assert!(matches!(
        kind_refusal("fn f<F: Nat -> Nat>(n: Nat) -> Nat { n }"),
        KindRefusal::CodomainNotProp
    ));
    assert!(matches!(
        kind_refusal("fn f<T: Nat>(n: Nat) -> Nat { n }"),
        KindRefusal::NotAKind
    ));
    assert!(matches!(
        kind_refusal("type Foo<P: Nat -> Prop> = Mk(Nat)"),
        KindRefusal::OnTypeDefinition { ref type_name } if type_name == "Foo"
    ));
    assert!(matches!(
        kind_refusal("type Al<P: Nat -> Prop> = Nat"),
        KindRefusal::OnTypeDefinition { .. }
    ));
    for source in [
        "fn f<P: Prop>(n: Nat) -> Nat { n }",
        "fn f<F: Nat -> Nat>(n: Nat) -> Nat { n }",
    ] {
        assert_eq!(first_code(source), "E0069");
    }
}

// D2: one message per case, each naming the parameter.
#[test]
fn each_kind_refusal_has_its_own_message() {
    let cases = [
        KindRefusal::RangesOverPropositions,
        KindRefusal::DomainIsProposition {
            domain: Type::eq(Type::Nat, Term::Zero, Term::Zero),
        },
        KindRefusal::DomainNamesTypeParameter { domain: "T".into() },
        KindRefusal::CodomainNotProp,
        KindRefusal::NotAKind,
        KindRefusal::OnTypeDefinition {
            type_name: "Foo".into(),
        },
    ];
    let messages: Vec<String> = cases
        .into_iter()
        .map(|why| {
            SortErrorKind::PredicateKindRefused {
                param: "Pq".into(),
                why,
            }
            .default_message()
        })
        .collect();
    for (index, message) in messages.iter().enumerate() {
        assert!(message.contains("`Pq"), "{message}");
        assert!(!messages[..index].contains(message), "duplicate: {message}");
    }
    assert!(messages[0].contains("impredicative"));
    assert!(messages[5].contains("`Foo`"));
}

// Twin: a predicate parameter cited without `::<…>` is never inferred, E0015.
#[test]
fn twin_an_uninstantiated_predicate_is_e0015() {
    let source =
        format!("{TRANSPORT}\ntheorem z() -> Eq<Nat, 0, 0> {{ pred_transport(0, 0, refl, refl) }}");
    assert_eq!(first_code(&source), "E0015");
}

// Twin: a family of the wrong arity instantiates nothing, E0012.
#[test]
fn twin_a_family_of_the_wrong_arity_is_e0012() {
    let source = format!(
        "type Same(n: Nat, m: Nat): Prop = n == m\n{TRANSPORT}\n\
         theorem z() -> Eq<Nat, 0, 0> {{ pred_transport::<Same>(0, 0, refl, refl) }}"
    );
    assert_eq!(first_code(&source), "E0012");
}

// D3: a predicate parameter used bare is missing its arguments, E0012; one
// applied to the wrong count is E0012 too.
#[test]
fn a_predicate_parameter_used_bare_or_misapplied_is_e0012() {
    assert_eq!(
        first_code("fn f<P: Nat -> Prop>(h: P) -> Nat { 0 }"),
        "E0012"
    );
    assert_eq!(
        first_code("fn f<P: Nat -> Prop>(h: P<0, 1>) -> Nat { 0 }"),
        "E0012"
    );
}

// D5: a data type is not a predicate.
#[test]
fn a_data_type_is_not_a_predicate_argument() {
    let source =
        format!("{TRANSPORT}\ntheorem z() -> Nat {{ pred_transport::<Nat>(0, 0, refl, refl) }}");
    assert!(matches!(
        elab_err(&source)[0].kind,
        ElabErrorKind::ExpectedType { .. }
    ));
}
