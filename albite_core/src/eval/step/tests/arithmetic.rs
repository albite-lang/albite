//! ADR 13.10.26e: the closed-term evaluator keeps the operator it steps, and
//! stops at overflow.
//!
//! Every nested fixture is chosen so that rebuilding the stepped term with the
//! wrong operator (`NatAdd` for arithmetic, `NatLt` for comparison, `BoolAnd`
//! for Bool, the pre-fix behaviour) gives a different answer. `+`, `<` and
//! `&&` are the control rows: the old rebuild happened to be right for them.

use super::*;
use crate::eval::{eval, term_to_nat};

fn lit(n: u64) -> Term {
    Term::nat_lit(n)
}

fn sum(a: u64, b: u64) -> Term {
    Term::nat_add(lit(a), lit(b))
}

fn eval_nat(term: &Term) -> Option<usize> {
    term_to_nat(&eval(term))
}

// 13.10.26e AC2: each arithmetic operator, left and right operand stepping first.
#[test]
fn a_stepped_arithmetic_operand_keeps_its_operator() {
    let rows: [(&str, Term, usize); 10] = [
        ("(2 + 1) + 1", Term::nat_add(sum(2, 1), lit(1)), 4),
        ("1 + (2 + 1)", Term::nat_add(lit(1), sum(2, 1)), 4),
        ("(2 + 1) - 1", Term::nat_sub(sum(2, 1), lit(1)), 2),
        ("10 - (2 + 1)", Term::nat_sub(lit(10), sum(2, 1)), 7),
        ("(2 + 1) * 2", Term::nat_mul(sum(2, 1), lit(2)), 6),
        ("2 * (2 + 1)", Term::nat_mul(lit(2), sum(2, 1)), 6),
        ("(3 + 3) / 2", Term::nat_div(sum(3, 3), lit(2)), 3),
        ("12 / (1 + 1)", Term::nat_div(lit(12), sum(1, 1)), 6),
        ("(3 + 3) % 4", Term::nat_mod(sum(3, 3), lit(4)), 2),
        ("10 % (1 + 2)", Term::nat_mod(lit(10), sum(1, 2)), 1),
    ];
    for (label, term, expected) in rows {
        assert_eq!(eval_nat(&term), Some(expected), "{label}");
    }
}

// 13.10.26e AC2: each comparison operator, left and right operand stepping first.
#[test]
fn a_stepped_comparison_operand_keeps_its_operator() {
    let rows: [(&str, Term, Term); 11] = [
        ("(1 + 2) == 3", Term::nat_eq(sum(1, 2), lit(3)), Term::True),
        ("3 == (1 + 2)", Term::nat_eq(lit(3), sum(1, 2)), Term::True),
        ("(1 + 1) < 3", Term::nat_lt(sum(1, 1), lit(3)), Term::True),
        ("1 < (1 + 1)", Term::nat_lt(lit(1), sum(1, 1)), Term::True),
        ("(1 + 2) <= 3", Term::nat_le(sum(1, 2), lit(3)), Term::True),
        ("3 <= (1 + 2)", Term::nat_le(lit(3), sum(1, 2)), Term::True),
        ("(2 + 2) > 3", Term::nat_gt(sum(2, 2), lit(3)), Term::True),
        ("4 > (1 + 2)", Term::nat_gt(lit(4), sum(1, 2)), Term::True),
        // Equal operands separate `>` from `>=`.
        ("(1 + 2) > 3", Term::nat_gt(sum(1, 2), lit(3)), Term::False),
        ("(1 + 1) >= 3", Term::nat_ge(sum(1, 1), lit(3)), Term::False),
        ("3 >= (1 + 2)", Term::nat_ge(lit(3), sum(1, 2)), Term::True),
    ];
    for (label, term, expected) in rows {
        assert_eq!(eval(&term), expected, "{label}");
    }
}

// 13.10.26e AC2: `&&` and `||`, left and right operand stepping first.
#[test]
fn a_stepped_bool_operand_keeps_its_operator() {
    let not = Term::bool_not;
    let rows: [(&str, Term, Term); 5] = [
        // Unequal operands separate `&&` from `||`.
        (
            "!true && true",
            Term::bool_and(not(Term::True), Term::True),
            Term::False,
        ),
        (
            "!false && true",
            Term::bool_and(not(Term::False), Term::True),
            Term::True,
        ),
        (
            "true && !false",
            Term::bool_and(Term::True, not(Term::False)),
            Term::True,
        ),
        (
            "!true || true",
            Term::bool_or(not(Term::True), Term::True),
            Term::True,
        ),
        (
            "false || !false",
            Term::bool_or(Term::False, not(Term::False)),
            Term::True,
        ),
    ];
    for (label, term, expected) in rows {
        assert_eq!(eval(&term), expected, "{label}");
    }
}

// 13.10.26e AC3: `+` and `*` past the largest Nat are stuck, not saturated.
#[test]
fn overflow_is_stuck_not_saturated() {
    let max = u64::MAX;
    for term in [
        Term::nat_add(lit(max), lit(1)),
        Term::nat_mul(lit(max), lit(2)),
        // A stuck operand stops the enclosing operator too.
        Term::nat_sub(Term::nat_add(lit(max), lit(1)), lit(1)),
    ] {
        assert_eq!(eval(&term), term);
    }
}

// 13.10.26e AC3: truncated subtraction is the language's `-`, not an overflow.
#[test]
fn truncation_stays_and_the_largest_nat_is_reachable() {
    let max = u64::MAX;
    assert_eq!(eval_nat(&Term::nat_sub(lit(0), lit(1))), Some(0));
    assert_eq!(
        eval_nat(&Term::nat_add(Term::nat_sub(lit(max), lit(1)), lit(1))),
        Some(max as usize)
    );
    assert_eq!(
        eval_nat(&Term::nat_mul(lit(max), lit(1))),
        Some(max as usize)
    );
}
