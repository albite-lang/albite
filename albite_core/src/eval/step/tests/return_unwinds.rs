//! 13.10.26n AC2: a non-tail `return` unwinds to its function boundary on the
//! closed-term stepper. The shapes are shared with the environment stepper's
//! twin (`eval::env::tests::return_unwinds`), so both steppers are held to
//! one table of native's answers.

use super::*;
use crate::eval::{eval, eval_with_limit};
use crate::types::Type;

/// One probed shape: a function applied to its argument, and native's value.
pub(in crate::eval) struct ReturnShape {
    pub name: &'static str,
    pub call: Term,
    pub native: Term,
}

fn nat(n: u64) -> Term {
    Term::nat_lit(n)
}

fn call(param: &str, param_ty: Type, body: Term, arg: Term) -> Term {
    Term::app(Term::lambda(param, param_ty, body), arg)
}

fn opt_ty() -> Type {
    Type::app("Opt", vec![])
}

/// `let x = if c then return 99 else 1 in x + 1`, the return in either branch.
fn let_bound_if(return_in_then: bool, c: bool) -> Term {
    let (then_, else_) = if return_in_then {
        (Term::early_return(nat(99)), nat(1))
    } else {
        (nat(1), Term::early_return(nat(99)))
    };
    let cond = if c { Term::True } else { Term::False };
    call(
        "c",
        Type::Bool,
        Term::let_in(
            "x",
            Type::Nat,
            Term::if_then_else(Term::var("c"), then_, else_),
            Term::nat_add(Term::var("x"), nat(1)),
        ),
        cond,
    )
}

/// A `?` as the elaborator lowers it: `case s of inl a => a | inr e => return e`.
fn question_mark(scrutinee: Term) -> Term {
    let sum = Type::sum(Type::Nat, Type::Nat);
    call(
        "s",
        sum,
        Term::let_in(
            "q",
            Type::Nat,
            Term::case(
                Term::var("s"),
                "a",
                Term::var("a"),
                "e",
                Term::early_return(Term::var("e")),
            ),
            Term::nat_add(Term::var("q"), nat(1000)),
        ),
        scrutinee,
    )
}

/// A statement-position `match` whose first arm returns.
fn statement_match(tag: usize) -> Term {
    let arms = vec![
        (
            0,
            "n".to_string(),
            Box::new(Term::early_return(Term::nat_add(Term::var("n"), nat(100)))),
        ),
        (1, "_u".to_string(), Box::new(Term::Unit)),
    ];
    let payload = if tag == 0 { nat(5) } else { Term::Unit };
    call(
        "o",
        opt_ty(),
        Term::let_in(
            "_",
            Type::Unit,
            Term::adt_match(Term::var("o"), arms),
            nat(7),
        ),
        Term::adt_construct(opt_ty(), tag, payload),
    )
}

/// `let g = λy. (let z = return y in z + 50) in g(n) + 1`: the closure's
/// `return` exits the closure, not the function holding it.
fn return_inside_a_closure() -> Term {
    let closure = Term::lambda(
        "y",
        Type::Nat,
        Term::let_in(
            "z",
            Type::Nat,
            Term::early_return(Term::var("y")),
            Term::nat_add(Term::var("z"), nat(50)),
        ),
    );
    call(
        "n",
        Type::Nat,
        Term::let_in(
            "g",
            Type::arrow(Type::Nat, Type::Nat),
            closure,
            Term::nat_add(Term::app(Term::var("g"), Term::var("n")), nat(1)),
        ),
        nat(5),
    )
}

/// `λa. λb. let x = return a in x + b`, the curried function.
pub(in crate::eval) fn curried_returning() -> Term {
    Term::lambda(
        "a",
        Type::Nat,
        Term::lambda(
            "b",
            Type::Nat,
            Term::let_in(
                "x",
                Type::Nat,
                Term::early_return(Term::var("a")),
                Term::nat_add(Term::var("x"), Term::var("b")),
            ),
        ),
    )
}

/// Every §1.1 shape, as Core, beside native's value.
pub(in crate::eval) fn return_shapes() -> Vec<ReturnShape> {
    let shape = |name, call, native| ReturnShape { name, call, native };
    vec![
        shape(
            "let-bound if, return in then",
            let_bound_if(true, true),
            nat(99),
        ),
        shape(
            "let-bound if, return in else",
            let_bound_if(false, false),
            nat(99),
        ),
        shape(
            "let-bound if, the other branch",
            let_bound_if(true, false),
            nat(2),
        ),
        shape(
            "failing ? under a let",
            question_mark(Term::inr(Type::sum(Type::Nat, Type::Nat), nat(3))),
            nat(3),
        ),
        shape(
            "succeeding ? under a let",
            question_mark(Term::inl(Type::sum(Type::Nat, Type::Nat), nat(3))),
            nat(1003),
        ),
        shape(
            "return in a statement match arm",
            statement_match(0),
            nat(105),
        ),
        shape("statement match, the other arm", statement_match(1), nat(7)),
        shape(
            "return in an application argument",
            call(
                "n",
                Type::Nat,
                Term::app(
                    Term::lambda("y", Type::Nat, Term::nat_add(Term::var("y"), nat(1))),
                    Term::early_return(Term::var("n")),
                ),
                nat(5),
            ),
            nat(5),
        ),
        shape(
            "return under arithmetic",
            call(
                "n",
                Type::Nat,
                Term::nat_add(Term::early_return(Term::var("n")), nat(100)),
                nat(4),
            ),
            nat(4),
        ),
        shape("return inside a closure", return_inside_a_closure(), nat(6)),
        shape(
            "curried, applied in full",
            Term::app(Term::app(curried_returning(), nat(3)), nat(4)),
            nat(3),
        ),
        shape(
            "tail return, the control",
            call(
                "c",
                Type::Bool,
                Term::if_then_else(Term::var("c"), Term::early_return(nat(99)), nat(1)),
                Term::True,
            ),
            nat(99),
        ),
    ]
}

/// A Nat result read as a number: literals and `Succ` chains spell the same
/// value differently, and the shapes assert values, not spellings.
pub(in crate::eval) fn as_number(term: &Term) -> u64 {
    crate::eval::term_to_nat(term).unwrap_or_else(|| panic!("not a Nat: {term:?}")) as u64
}

/// Whether a `CallFrame` survives anywhere in `term`.
pub(in crate::eval) fn holds_a_call_frame(term: &Term) -> bool {
    if matches!(term, Term::CallFrame(_)) {
        return true;
    }
    let mut found = false;
    term.for_each_subterm(|sub| found = found || holds_a_call_frame(sub));
    found
}

/// A function whose body gets stuck on a free variable after its `return`
/// was not taken, so the stuck residual is inside a call frame.
pub(in crate::eval) fn stuck_inside_a_frame() -> Term {
    call(
        "c",
        Type::Bool,
        Term::let_in(
            "x",
            Type::Nat,
            Term::if_then_else(
                Term::var("c"),
                Term::early_return(nat(1)),
                Term::var("free"),
            ),
            Term::nat_add(Term::var("x"), nat(1)),
        ),
        Term::False,
    )
}

#[test]
fn every_shape_evaluates_to_natives_value() {
    for shape in return_shapes() {
        assert_eq!(
            as_number(&eval(&shape.call)),
            as_number(&shape.native),
            "{}",
            shape.name
        );
    }
}

#[test]
fn a_returning_body_is_framed_and_the_frame_catches_the_return() {
    let StepResult::Stepped(framed) = step(&let_bound_if(true, true)) else {
        panic!("β must step");
    };
    assert!(matches!(framed, Term::CallFrame(_)), "{framed:?}");
    // The frame turns the body's `Returned` into a plain step to the value.
    let ready = Term::call_frame(Term::nat_add(Term::early_return(nat(9)), nat(1)));
    assert_eq!(step(&ready), StepResult::Stepped(nat(9)));
    // A bare `return` of a value is abrupt, not stripped where it stands.
    assert_eq!(
        step(&Term::early_return(nat(9))),
        StepResult::Returned(nat(9))
    );
}

#[test]
fn a_return_free_body_is_not_framed() {
    let plain = call(
        "n",
        Type::Nat,
        Term::nat_add(Term::var("n"), nat(1)),
        nat(2),
    );
    assert_eq!(
        step(&plain),
        StepResult::Stepped(Term::nat_add(nat(2), nat(1)))
    );
}

#[test]
fn a_curried_call_frames_only_at_the_saturating_beta() {
    let StepResult::Stepped(partial) = step(&Term::app(curried_returning(), nat(3))) else {
        panic!("β must step");
    };
    assert!(matches!(partial, Term::Lambda(..)), "{partial:?}");
}

#[test]
fn a_returned_at_the_top_is_the_value() {
    let top = Term::let_in(
        "x",
        Type::Nat,
        Term::early_return(nat(4)),
        Term::nat_add(Term::var("x"), nat(1)),
    );
    assert_eq!(eval(&top), nat(4));
    assert_eq!(eval_with_limit(&top, 100), Some(nat(4)));
}

#[test]
fn a_stuck_run_hands_back_no_call_frame() {
    let stuck = eval(&stuck_inside_a_frame());
    assert!(!holds_a_call_frame(&stuck), "{stuck:?}");
    assert!(matches!(stuck, Term::Let(..)), "{stuck:?}");
    let limited = eval_with_limit(&stuck_inside_a_frame(), 100).expect("stuck within the limit");
    assert!(!holds_a_call_frame(&limited), "{limited:?}");
}

/// The helpers the twin stepper's file shares are pinned, so neither the
/// shape table nor the frame detector can go vacuous.
#[test]
fn the_shared_helpers_keep_their_contracts() {
    assert_eq!(return_shapes().len(), 12);
    assert!(holds_a_call_frame(&Term::call_frame(Term::Unit)));
    assert!(holds_a_call_frame(&Term::pair(
        Term::Unit,
        Term::call_frame(Term::Unit)
    )));
    assert!(holds_a_call_frame(&Term::pair(
        Term::call_frame(Term::Unit),
        Term::Unit
    )));
    assert!(!holds_a_call_frame(&Term::pair(Term::Unit, Term::Unit)));
}
