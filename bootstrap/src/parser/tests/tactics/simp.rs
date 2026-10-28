//! ADR 4.10.26e AC 1 — `simp [r₁, …, rₙ]` parses with `rewrite`'s rules,
//! requires its list, and stays an identifier outside a `by` block (D1).

use super::*;
use crate::parser::tactics::EXPECTED_A_SIMP_LIST;

// ADR 4.10.26e AC1
#[test]
fn simp_parses_a_rule_list_with_reversal() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { \
                  simp [nat_arith::add_succ_left, <- ih, nat_arith::mul_comm(k, n)] }";
    let file = parse_ok(source);
    let tactics = tactics_of(theorem_body(&file, 0));
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    let Tactic::Simp(rules, span) = &tactics[0] else {
        panic!("expected a simp, got {:?}", tactics[0]);
    };
    assert_eq!(
        text(*span),
        "simp [nat_arith::add_succ_left, <- ih, nat_arith::mul_comm(k, n)]"
    );
    assert_eq!(tactics[0].name(), "simp");
    assert_eq!(tactics[0].span(), *span);
    let flags: Vec<bool> = rules.iter().map(|rule| rule.reverse).collect();
    assert_eq!(flags, [false, true, false]);
    assert_eq!(text(rules[1].span), "<- ih");
    assert_eq!(text(rules[2].span), "nat_arith::mul_comm(k, n)");
    // The walkers see each rule's expression, as for `rewrite` (D11).
    assert_eq!(theorem_body(&file, 0).expressions().len(), 3);
}

// ADR 4.10.26e AC1: no global database (Q5), so no list is no `simp`.
#[test]
fn simp_without_a_list_is_refused() {
    for source in [
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { simp }",
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { simp h }",
    ] {
        let (_, errors) = parse(source);
        assert!(has_expected(&errors, EXPECTED_A_SIMP_LIST), "{errors:?}");
    }
    assert!(EXPECTED_A_SIMP_LIST.contains("explicit lemma list"));
}

// ADR 4.10.26e AC1
#[test]
fn simp_with_an_empty_list_is_refused() {
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { simp [] }");
    assert!(has_expected(&errors, EXPECTED_A_SIMP_LIST), "{errors:?}");
    // `rewrite []` is refused with its own list message, not `simp`'s.
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { rewrite [] }");
    let rule_list = crate::parser::tactics::EXPECTED_A_RULE_LIST;
    assert!(has_expected(&errors, rule_list), "{errors:?}");
    assert!(!has_expected(&errors, EXPECTED_A_SIMP_LIST), "{errors:?}");
}

// ADR 4.10.26e AC1
#[test]
fn an_unknown_tactic_lists_simp() {
    assert!(EXPECTED_A_TACTIC.contains("simp"));
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { simplify [h] }");
    assert!(has_expected(&errors, EXPECTED_A_TACTIC), "{errors:?}");
}

// ADR 4.10.26e AC1: contextual, like every tactic name before it.
#[test]
fn simp_stays_an_identifier_outside_a_by_block() {
    parse_ok("mod simp;");
    parse_ok("use a::simp::{f};");
    parse_ok("fn f(simp: Nat) -> Nat { simp }");
    parse_ok("fn simp(n: Nat) -> Nat { n }");
}
