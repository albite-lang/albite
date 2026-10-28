//! 13.10.26n AC2, D2: every congruence rule passes `Returned` up from every
//! operand position, on both steppers. One row per operand slot: a `return 7`
//! there must make the whole term report `Returned(7)` — never a step that
//! rebuilds the node around a value, which was the stripped-in-place fault.

use super::*;
use crate::eval::step_with_env;
use crate::eval::EvalEnv;
use crate::terms::IntBinOp;
use crate::types::Type;

fn return_seven() -> Term {
    Term::early_return(Term::nat_lit(7))
}

fn nat(n: u64) -> Term {
    Term::nat_lit(n)
}

fn text() -> Term {
    Term::string_lit("ab")
}

fn sum_ty() -> Type {
    Type::sum(Type::Nat, Type::Nat)
}

/// Arithmetic, comparison, boolean and signed-integer operand slots.
fn arithmetic_slots() -> Vec<(&'static str, Term)> {
    vec![
        ("nat_add left", Term::nat_add(return_seven(), nat(1))),
        ("nat_add right", Term::nat_add(nat(1), return_seven())),
        ("nat_sub right", Term::nat_sub(nat(1), return_seven())),
        ("nat_lt left", Term::nat_lt(return_seven(), nat(1))),
        ("nat_lt right", Term::nat_lt(nat(1), return_seven())),
        ("nat_eq left", Term::nat_eq(return_seven(), nat(1))),
        ("nat_eq right", Term::nat_eq(nat(1), return_seven())),
        ("bool_and left", Term::bool_and(return_seven(), Term::True)),
        ("bool_and right", Term::bool_and(Term::True, return_seven())),
        ("bool_not", Term::bool_not(return_seven())),
        (
            "int_bin left",
            Term::int_bin(IntBinOp::Add, return_seven(), Term::IntLit(1)),
        ),
        (
            "int_bin right",
            Term::int_bin(IntBinOp::Add, Term::IntLit(1), return_seven()),
        ),
        ("int_neg", Term::int_neg(return_seven())),
        ("nat_to_int", Term::nat_to_int(return_seven())),
    ]
}

/// String operand slots.
fn string_slots() -> Vec<(&'static str, Term)> {
    vec![
        ("str_concat left", Term::str_concat(return_seven(), text())),
        ("str_concat right", Term::str_concat(text(), return_seven())),
        ("str_len", Term::str_len(return_seven())),
        ("str_eq left", Term::str_eq(return_seven(), text())),
        ("str_eq right", Term::str_eq(text(), return_seven())),
        (
            "str_char_at string",
            Term::str_char_at(return_seven(), nat(0)),
        ),
        (
            "str_char_at index",
            Term::str_char_at(text(), return_seven()),
        ),
        (
            "str_substring string",
            Term::str_substring(return_seven(), nat(0), nat(1)),
        ),
        (
            "str_substring start",
            Term::str_substring(text(), return_seven(), nat(1)),
        ),
        (
            "str_substring length",
            Term::str_substring(text(), nat(0), return_seven()),
        ),
    ]
}

/// Core-calculus, product, sum, recursion and data operand slots.
fn core_slots() -> Vec<(&'static str, Term)> {
    let identity = Term::lambda("y", Type::Nat, Term::var("y"));
    let step_fn = Term::lambda(
        "k",
        Type::Nat,
        Term::lambda("acc", Type::Nat, Term::var("acc")),
    );
    vec![
        ("app function", Term::app(return_seven(), nat(1))),
        ("app argument", Term::app(identity, return_seven())),
        (
            "let definition",
            Term::let_in("x", Type::Nat, return_seven(), Term::var("x")),
        ),
        (
            "if condition",
            Term::if_then_else(return_seven(), nat(1), nat(2)),
        ),
        ("pair left", Term::pair(return_seven(), nat(1))),
        ("pair right", Term::pair(nat(1), return_seven())),
        ("fst", Term::fst(return_seven())),
        ("snd", Term::snd(return_seven())),
        (
            "case scrutinee",
            Term::case(return_seven(), "a", Term::var("a"), "b", Term::var("b")),
        ),
        (
            "natrec count",
            Term::natrec(Type::Nat, nat(0), step_fn.clone(), return_seven()),
        ),
        (
            "natind count",
            Term::natind("m", Type::Nat, nat(0), step_fn, return_seven()),
        ),
        (
            "subst proof of equality",
            Term::subst(Type::Nat, Type::Nat, return_seven(), Term::Unit),
        ),
        (
            "subst proof",
            Term::subst(
                Type::Nat,
                Type::Nat,
                Term::refl(Type::Nat, nat(1)),
                return_seven(),
            ),
        ),
        ("ty_app", Term::ty_app(return_seven(), Type::Nat)),
        ("unfold", Term::unfold(Type::Nat, return_seven())),
        (
            "adt_match scrutinee",
            Term::adt_match(return_seven(), vec![(0, "p".to_string(), Box::new(nat(1)))]),
        ),
        ("succ", Term::succ(return_seven())),
        ("inl", Term::inl(sum_ty(), return_seven())),
        (
            "adt_construct",
            Term::adt_construct(Type::Nat, 0, return_seven()),
        ),
        ("absurd", Term::absurd(Type::Nat, return_seven())),
        ("annot", Term::annot(return_seven(), Type::Nat)),
        (
            "region body",
            Term::region(
                crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                return_seven(),
            ),
        ),
        ("return of a return", Term::early_return(return_seven())),
    ]
}

fn every_slot() -> Vec<(&'static str, Term)> {
    let mut slots = arithmetic_slots();
    slots.extend(string_slots());
    slots.extend(core_slots());
    slots
}

/// Step until something other than `Stepped` comes back: a `Region` or an
/// `Annot` steps once to its body before the `return` inside is reached.
fn first_stop(term: &Term, stepper: impl Fn(&Term) -> StepResult) -> StepResult {
    let mut current = term.clone();
    for _ in 0..4 {
        match stepper(&current) {
            StepResult::Stepped(next) => current = next,
            other => return other,
        }
    }
    StepResult::Stepped(current)
}

#[test]
fn a_return_in_any_operand_slot_is_returned_by_the_closed_stepper() {
    for (slot, term) in every_slot() {
        assert_eq!(
            first_stop(&term, step),
            StepResult::Returned(nat(7)),
            "{slot}"
        );
    }
}

#[test]
fn a_return_in_any_operand_slot_is_returned_by_the_environment_stepper() {
    let env = EvalEnv::empty();
    for (slot, term) in every_slot() {
        let stepped = first_stop(&term, |t| step_with_env(t, &env));
        assert_eq!(stepped, StepResult::Returned(nat(7)), "{slot}");
    }
}

/// A stuck operand still reports `Stuck`, never `Returned`: the shared arm
/// passes each up as it came.
#[test]
fn a_stuck_operand_stays_stuck_beside_the_returned_arm() {
    let stuck = Term::nat_add(Term::var("free"), nat(1));
    assert_eq!(step(&stuck), StepResult::Stuck);
    assert_eq!(step_with_env(&stuck, &EvalEnv::empty()), StepResult::Stuck);
}

/// A frame around a value is that value; a frame around a stuck body is stuck.
#[test]
fn a_frame_around_a_value_or_a_stuck_body() {
    let env = EvalEnv::empty();
    let done = Term::call_frame(nat(3));
    assert_eq!(step(&done), StepResult::Stepped(nat(3)));
    assert_eq!(step_with_env(&done, &env), StepResult::Stepped(nat(3)));
    let stuck = Term::call_frame(Term::var("free"));
    assert_eq!(step(&stuck), StepResult::Stuck);
    assert_eq!(step_with_env(&stuck, &env), StepResult::Stuck);
}

/// The tables are the test: an emptied one would make both loops above pass
/// vacuously, so their sizes are pinned.
#[test]
fn the_operand_slot_tables_are_complete() {
    assert_eq!(arithmetic_slots().len(), 14);
    assert_eq!(string_slots().len(), 10);
    assert_eq!(core_slots().len(), 23);
    assert_eq!(every_slot().len(), 47);
}
