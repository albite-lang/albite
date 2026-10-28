//! ADR 29.9.26h AC 1: the nullary predicates are the census's definition.
//! ADR 1.10.26g AC 1: the tag path sees exactly what the predicate sees.

use crate::terms::analysis::{
    empty_payload_path, injects_into_mu, is_empty_payload, is_nullary_construction,
    nullary_tag_path,
};
use crate::terms::{Term, TermSpan};
use crate::types::Type;

fn unit() -> Box<Term> {
    Box::new(Term::Unit)
}

fn inl(payload: Term) -> Term {
    Term::Inl(Type::Unit, Box::new(payload))
}

fn inr(payload: Term) -> Term {
    Term::Inr(Type::Unit, Box::new(payload))
}

fn construct(payload: Term) -> Term {
    construct_at(1, payload)
}

fn construct_at(index: usize, payload: Term) -> Term {
    Term::AdtConstruct(Type::Unit, index, Box::new(payload))
}

fn spanned(term: Term) -> Term {
    Term::spanned(term, TermSpan::new(0, 4))
}

fn annot(term: Term) -> Term {
    Term::Annot(Box::new(term), Type::Unit)
}

/// The three shapes a nullary construction reaches codegen as.
fn nullary_shapes() -> Vec<Term> {
    vec![construct(Term::Unit), inl(Term::Unit), inr(inl(Term::Unit))]
}

/// Every nullary shape, bare and under each wrapper the predicate peels.
fn nullary_cases() -> Vec<Term> {
    let mut cases = Vec::new();
    for shape in nullary_shapes() {
        cases.push(spanned(shape.clone()));
        cases.push(annot(shape.clone()));
        cases.push(spanned(annot(shape.clone())));
        cases.push(shape);
    }
    cases
}

/// Constructions carrying a value, a bare payload, a variable, and a
/// construction hidden under a `let` (29.9.26h D3): none is nullary.
fn non_nullary_cases() -> Vec<Term> {
    let pair = Term::Pair(Box::new(Term::NatLit(1)), unit());
    let hidden = Term::Let(
        "x".into(),
        Type::Unit,
        Box::new(construct(Term::Unit)),
        Box::new(Term::Var("x".into())),
    );
    vec![
        construct(Term::NatLit(3)),
        construct(pair.clone()),
        inl(Term::NatLit(3)),
        inr(pair),
        inr(inl(Term::NatLit(3))),
        inr(inl(Term::Var("n".into()))),
        Term::Unit,
        spanned(Term::Unit),
        Term::Var("nil".into()),
        hidden,
    ]
}

/// The one table the predicate and the tag path are both judged against.
fn all_cases() -> Vec<Term> {
    let mut cases = nullary_cases();
    cases.extend(non_nullary_cases());
    cases
}

#[test]
fn nullary_constructions_are_recognised_bare_and_wrapped() {
    let cases = nullary_cases();
    // An emptied fixture would make the loop below assert nothing.
    assert_eq!(cases.len(), 12);
    for case in cases {
        assert!(is_nullary_construction(&case), "{case:?}");
    }
}

#[test]
fn constructions_carrying_a_value_are_not_nullary() {
    let cases = non_nullary_cases();
    assert_eq!(cases.len(), 10);
    for case in cases {
        assert!(!is_nullary_construction(&case), "{case:?}");
    }
}

#[test]
fn a_bare_unit_or_a_variable_is_not_a_construction() {
    assert!(!is_nullary_construction(&Term::Unit));
    assert!(!is_nullary_construction(&spanned(Term::Unit)));
    assert!(!is_nullary_construction(&Term::Var("nil".into())));
}

#[test]
fn empty_payload_is_unit_or_nested_empty_injections() {
    assert!(is_empty_payload(&Term::Unit));
    assert!(is_empty_payload(&spanned(Term::Unit)));
    assert!(is_empty_payload(&inl(Term::Unit)));
    assert!(is_empty_payload(&inr(annot(inl(Term::Unit)))));
    assert!(!is_empty_payload(&inr(inl(Term::NatLit(0)))));
    assert!(!is_empty_payload(&Term::NatLit(0)));
    assert!(!is_empty_payload(&Term::Var("u".into())));
    // A flat ADT construction is a value, not a payload shape of the census.
    assert!(!is_empty_payload(&construct(Term::Unit)));
    assert!(!is_empty_payload(&Term::Pair(unit(), unit())));
}

/// ADR 1.10.26g AC 1.
#[test]
fn nullary_tag_path_names_every_tag() {
    assert_eq!(
        nullary_tag_path(&construct_at(7, Term::Unit)),
        Some(vec![7])
    );
    assert_eq!(nullary_tag_path(&inl(Term::Unit)), Some(vec![0]));
    assert_eq!(nullary_tag_path(&inr(Term::Unit)), Some(vec![1]));
    assert_eq!(nullary_tag_path(&inr(inl(Term::Unit))), Some(vec![1, 0]));
    assert_eq!(
        nullary_tag_path(&inl(inr(annot(inr(Term::Unit))))),
        Some(vec![0, 1, 1])
    );
    assert_eq!(
        nullary_tag_path(&construct_at(2, inr(Term::Unit))),
        Some(vec![2, 1])
    );
    // The wrappers carry no tag.
    assert_eq!(
        nullary_tag_path(&spanned(annot(inr(inl(Term::Unit))))),
        Some(vec![1, 0])
    );
    for refused in [
        construct(Term::NatLit(3)),
        construct(Term::Pair(unit(), unit())),
        inr(inl(Term::NatLit(3))),
        Term::Unit,
        Term::Var("nil".into()),
    ] {
        assert_eq!(nullary_tag_path(&refused), None, "{refused:?}");
    }
}

/// ADR 1.10.26g AC 1: an index no `i32` tag can hold is declined, not
/// truncated to a different constructor.
#[test]
fn nullary_tag_path_refuses_an_index_past_u32() {
    let index = usize::try_from(u64::from(u32::MAX) + 1).unwrap_or(usize::MAX);
    assert_eq!(nullary_tag_path(&construct_at(index, Term::Unit)), None);
    let last = u32::MAX as usize;
    assert_eq!(
        nullary_tag_path(&construct_at(last, Term::Unit)),
        Some(vec![u32::MAX])
    );
}

/// ADR 1.10.26g AC 1: the population optimised is the population counted.
#[test]
fn tag_path_agrees_with_the_nullary_predicate() {
    let cases = all_cases();
    assert_eq!(cases.len(), 22);
    for case in cases {
        assert_eq!(
            nullary_tag_path(&case).is_some(),
            is_nullary_construction(&case),
            "{case:?}"
        );
        // The payload half agrees with its own predicate the same way.
        assert_eq!(
            empty_payload_path(&case).is_some(),
            is_empty_payload(&case),
            "{case:?}"
        );
    }
}

#[test]
fn empty_payload_path_lists_the_nested_tags() {
    assert_eq!(empty_payload_path(&Term::Unit), Some(vec![]));
    assert_eq!(empty_payload_path(&inl(Term::Unit)), Some(vec![0]));
    assert_eq!(
        empty_payload_path(&inr(spanned(inl(Term::Unit)))),
        Some(vec![1, 0])
    );
    assert_eq!(empty_payload_path(&inl(Term::NatLit(0))), None);
}

/// ADR 1.10.26g D1: a μ-typed injection lowers to a pointer, so a chain
/// carrying one is flagged for codegen to decline.
#[test]
fn injects_into_mu_finds_a_mu_typed_injection_at_any_depth() {
    let mu = Type::Mu("α_L".into(), Box::new(Type::Unit));
    let mu_inl = Term::Inl(mu.clone(), unit());
    assert!(injects_into_mu(&mu_inl));
    assert!(injects_into_mu(&inr(mu_inl.clone())));
    assert!(injects_into_mu(&construct(annot(mu_inl))));
    assert!(injects_into_mu(&Term::Inr(mu, Box::new(inl(Term::Unit)))));
    for plain in nullary_shapes() {
        assert!(!injects_into_mu(&plain), "{plain:?}");
    }
    assert!(!injects_into_mu(&Term::Unit));
    assert!(!injects_into_mu(&Term::Var("x".into())));
}
