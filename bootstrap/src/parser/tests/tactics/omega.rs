//! ADR 6.10.26f AC 2 — `omega` parses alone, takes no argument, and stays an
//! identifier outside a `by` block (D1).

use super::*;
use crate::parser::tactics::EXPECTED_OMEGA_ALONE;

// ADR 6.10.26f AC2 (D1): the name 4.10.26e moved its unknown-tactic example
// off now parses.
#[test]
fn omega_parses_alone() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { omega }";
    let file = parse_ok(source);
    let body = theorem_body(&file, 0);
    let tactics = tactics_of(body);
    let Tactic::Omega(span) = &tactics[0] else {
        panic!("expected an omega, got {:?}", tactics[0]);
    };
    assert_eq!(&source[span.start as usize..span.end as usize], "omega");
    assert_eq!(tactics[0].name(), "omega");
    assert_eq!(tactics[0].span(), *span);
    assert!(body.expressions().is_empty());
    let file = parse_ok("theorem t(n: Nat) -> Eq<Nat, n, n> by { omega; refl }");
    assert_eq!(tactics_of(theorem_body(&file, 0)).len(), 2);
}

// ADR 6.10.26f AC2 (D1)
#[test]
fn omega_with_an_argument_is_refused() {
    for source in [
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { omega n }",
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { omega [h] }",
    ] {
        let (_, errors) = parse(source);
        assert!(has_expected(&errors, EXPECTED_OMEGA_ALONE), "{errors:?}");
    }
    assert!(EXPECTED_OMEGA_ALONE.contains("no arguments"));
    assert!(EXPECTED_A_TACTIC.contains("omega"));
}

// ADR 6.10.26f AC2 (D1): contextual, like every tactic name before it.
#[test]
fn omega_stays_an_identifier_outside_a_by_block() {
    parse_ok("mod omega;");
    parse_ok("fn f(omega: Nat) -> Nat { omega }");
    parse_ok("fn omega(n: Nat) -> Nat { n }");
}
