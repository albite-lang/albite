//! ADR 7.10.26l D2 — `left` and `right` parse alone, take no argument, and
//! stay identifiers outside a `by` block.

use super::*;
use crate::parser::tactics::{EXPECTED_LEFT_ALONE, EXPECTED_RIGHT_ALONE};

// ADR 7.10.26l AC1 (D2)
#[test]
fn left_and_right_parse_alone() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> + Eq<Nat, 0, 1> by { left; right }";
    let file = parse_ok(source);
    let body = theorem_body(&file, 0);
    let tactics = tactics_of(body);
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    let Tactic::Left(left) = &tactics[0] else {
        panic!("expected a left, got {:?}", tactics[0]);
    };
    let Tactic::Right(right) = &tactics[1] else {
        panic!("expected a right, got {:?}", tactics[1]);
    };
    assert_eq!(text(*left), "left");
    assert_eq!(text(*right), "right");
    assert_eq!(tactics[0].name(), "left");
    assert_eq!(tactics[1].name(), "right");
    assert_eq!(tactics[0].span(), *left);
    assert_eq!(tactics[1].span(), *right);
    assert!(body.expressions().is_empty());
}

// ADR 7.10.26l AC2 (D2)
#[test]
fn left_or_right_with_an_argument_is_refused() {
    for (source, expected) in [
        (
            "theorem t(n: Nat) -> Eq<Nat, n, n> by { left refl }",
            EXPECTED_LEFT_ALONE,
        ),
        (
            "theorem t(n: Nat) -> Eq<Nat, n, n> by { right(h) }",
            EXPECTED_RIGHT_ALONE,
        ),
    ] {
        let (_, errors) = parse(source);
        assert!(has_expected(&errors, expected), "{errors:?}");
    }
    assert!(EXPECTED_LEFT_ALONE.contains("after `left`"));
    assert!(EXPECTED_RIGHT_ALONE.contains("after `right`"));
    assert!(EXPECTED_A_TACTIC.contains("left, right"));
}

// ADR 7.10.26l AC1 (D2): contextual, like every tactic name before it.
#[test]
fn left_and_right_stay_identifiers_outside_a_by_block() {
    parse_ok("fn f(left: Nat, right: Nat) -> Nat { left + right }");
    parse_ok("fn left(n: Nat) -> Nat { n }");
    parse_ok("type Side = Left | Right");
}
