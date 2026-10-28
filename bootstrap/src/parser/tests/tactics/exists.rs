//! ADR 6.10.26e AC 2 — `exists w` and the `cases` pair arm parse (D4, D6, D8).

use super::*;
use crate::parser::tactics::{EXPECTED_A_CASES_BINDER, EXPECTED_A_PAIR_ARM};

/// The source text `span` covers.
fn text_of(source: &str, span: crate::span::Span) -> &str {
    &source[span.start as usize..span.end as usize]
}

// ADR 6.10.26e AC2
#[test]
fn exists_parses_its_witness() {
    let source = "theorem t(n: Nat) -> Le<n, n + 2> by { exists 1 + 1; refl }";
    let file = parse_ok(source);
    let body = theorem_body(&file, 0);
    let tactics = tactics_of(body);
    assert_eq!(tactics.len(), 2);
    let Tactic::Exists(witness, span) = &tactics[0] else {
        panic!("expected an exists, got {:?}", tactics[0]);
    };
    assert_eq!(text_of(source, *span), "exists 1 + 1");
    assert_eq!(text_of(source, witness.span()), "1 + 1");
    assert_eq!(tactics[0].name(), "exists");
    assert_eq!(tactics[0].span(), *span);
    // The witness is an expression the walkers visit (D9).
    assert_eq!(body.expressions().len(), 1);
}

// ADR 6.10.26e AC2: `exists` is the reserved token, never an identifier.
#[test]
fn exists_without_a_witness_is_refused() {
    let (_, errors) = parse("theorem t(n: Nat) -> Le<n, n> by { exists }");
    assert!(!errors.is_empty());
    assert!(EXPECTED_A_TACTIC.contains("exists"));
}

// ADR 6.10.26e AC2
#[test]
fn a_pair_arm_parses_its_two_names_and_body() {
    let source =
        "theorem t(h: Le<a, b>) -> Le<a, b> by { cases h { (k, e) => { exists k; exact e } } }";
    let file = parse_ok(source);
    let body = theorem_body(&file, 0);
    let tactics = tactics_of(body);
    let Tactic::CasesPair(scrutinee, witness, evidence, arm, span) = &tactics[0] else {
        panic!("expected a pair arm, got {:?}", tactics[0]);
    };
    assert_eq!(text_of(source, scrutinee.span()), "h");
    assert_eq!(witness.as_ref().map(|i| i.name.as_str()), Some("k"));
    assert_eq!(evidence.as_ref().map(|i| i.name.as_str()), Some("e"));
    assert_eq!(arm.len(), 2);
    assert_eq!(
        text_of(source, *span),
        "cases h { (k, e) => { exists k; exact e } }"
    );
    assert_eq!(tactics[0].name(), "cases");
    // The scrutinee, then the arm's `exists` witness and `exact` term (D9).
    assert_eq!(body.expressions().len(), 3);
}

// ADR 6.10.26e AC2: `_` takes a synthesised name, and a trailing comma and a
// single-tactic body both parse.
#[test]
fn a_pair_arm_takes_underscores_and_a_trailing_comma() {
    let file = parse_ok("theorem t(h: Le<a, b>) -> Le<a, b> by { cases h { (_, _) => exact h, } }");
    let tactics = tactics_of(theorem_body(&file, 0));
    let Tactic::CasesPair(_, witness, evidence, arm, _) = &tactics[0] else {
        panic!("expected a pair arm, got {:?}", tactics[0]);
    };
    assert!(witness.is_none() && evidence.is_none());
    assert_eq!(arm.len(), 1);
}

// ADR 6.10.26e AC2: a constructor arm is still a constructor arm.
#[test]
fn a_constructor_arm_is_not_a_pair_arm() {
    let file = parse_ok(
        "theorem t(x: Opt) -> Eq<Opt, x, x> by { cases x { None => refl, Some(v) => refl } }",
    );
    let tactics = tactics_of(theorem_body(&file, 0));
    assert!(matches!(&tactics[0], Tactic::Cases(_, arms, _) if arms.len() == 2));
}

// ADR 6.10.26e AC2: an `exists` has exactly two parts.
#[test]
fn a_pair_arm_with_other_than_two_names_is_refused() {
    let (_, errors) =
        parse("theorem t(h: Le<a, b>) -> Le<a, b> by { cases h { (k, e, z) => exact h } }");
    assert!(has_expected(&errors, EXPECTED_A_PAIR_ARM), "{errors:?}");
    let (_, errors) = parse("theorem t(h: Le<a, b>) -> Le<a, b> by { cases h { (k) => exact h } }");
    assert!(has_expected(&errors, EXPECTED_A_PAIR_ARM), "{errors:?}");
    let (_, errors) =
        parse("theorem t(h: Le<a, b>) -> Le<a, b> by { cases h { (0, e) => exact h } }");
    assert!(has_expected(&errors, EXPECTED_A_CASES_BINDER), "{errors:?}");
}
