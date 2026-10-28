//! ADR 4.10.26g AC 1 — `ring` parses alone, takes no argument, and stays an
//! identifier outside a `by` block (D1).

use super::*;
use crate::parser::tactics::EXPECTED_RING_ALONE;

// ADR 4.10.26g AC1
#[test]
fn ring_parses_alone() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { ring }";
    let file = parse_ok(source);
    let body = theorem_body(&file, 0);
    let tactics = tactics_of(body);
    let Tactic::Ring(span) = &tactics[0] else {
        panic!("expected a ring, got {:?}", tactics[0]);
    };
    assert_eq!(&source[span.start as usize..span.end as usize], "ring");
    assert_eq!(tactics[0].name(), "ring");
    assert_eq!(tactics[0].span(), *span);
    // No expression for the walkers to visit (D11).
    assert!(body.expressions().is_empty());
    // A step after it, with a separator.
    let file = parse_ok("theorem t(n: Nat) -> Eq<Nat, n, n> by { ring; refl }");
    assert_eq!(tactics_of(theorem_body(&file, 0)).len(), 2);
}

// ADR 4.10.26g AC1
#[test]
fn ring_with_an_argument_is_refused() {
    for source in [
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { ring n }",
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { ring [nat_arith::add_comm] }",
    ] {
        let (_, errors) = parse(source);
        assert!(has_expected(&errors, EXPECTED_RING_ALONE), "{errors:?}");
    }
    assert!(EXPECTED_RING_ALONE.contains("no arguments"));
}

// ADR 4.10.26g AC1
#[test]
fn an_unknown_tactic_lists_ring() {
    assert!(EXPECTED_A_TACTIC.contains("ring"));
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { rings }");
    assert!(has_expected(&errors, EXPECTED_A_TACTIC), "{errors:?}");
}

// ADR 4.10.26g AC1: contextual, like every tactic name before it.
#[test]
fn ring_stays_an_identifier_outside_a_by_block() {
    parse_ok("mod ring;");
    parse_ok("use a::ring::{f};");
    parse_ok("fn f(ring: Nat) -> Nat { ring }");
    parse_ok("fn ring(n: Nat) -> Nat { n }");
}
