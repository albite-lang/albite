//! 13.10.26n AC2: a non-tail `return` unwinds to its function boundary on the
//! environment stepper, over the closed stepper's table of shapes plus the
//! two boundaries only this stepper has: a nullary global's forcing and an
//! extern call's arguments.

use std::collections::HashMap;

use crate::eval::env::*;
use crate::eval::step::tests::return_unwinds::{
    as_number, curried_returning, holds_a_call_frame, return_shapes, stuck_inside_a_frame,
};
use crate::eval::StepResult;
use crate::terms::Term;
use crate::types::Type;

fn nat(n: u64) -> Term {
    Term::nat_lit(n)
}

#[test]
fn every_shape_evaluates_to_natives_value() {
    let env = EvalEnv::empty();
    for shape in return_shapes() {
        let value = eval_with_env(&shape.call, &env).expect(shape.name);
        assert_eq!(
            as_number(&value),
            as_number(&shape.native),
            "{}",
            shape.name
        );
    }
}

#[test]
fn a_curried_call_frames_only_at_the_saturating_beta() {
    let env = EvalEnv::empty();
    let StepResult::Stepped(partial) = step_with_env(&Term::app(curried_returning(), nat(3)), &env)
    else {
        panic!("β must step");
    };
    assert!(matches!(partial, Term::Lambda(..)), "{partial:?}");
    let StepResult::Stepped(saturated) = step_with_env(&Term::app(partial, nat(4)), &env) else {
        panic!("β must step");
    };
    assert!(matches!(saturated, Term::CallFrame(_)), "{saturated:?}");
}

/// A nullary global has no β: forcing it is its function boundary, and the
/// memo holds the caught value, never the `return` or a frame.
#[test]
fn a_nullary_globals_return_stops_at_its_forcing_and_is_memoized_caught() {
    let returning = Term::let_in(
        "x",
        Type::Nat,
        Term::early_return(nat(7)),
        Term::nat_add(Term::var("x"), nat(1)),
    );
    let globals = HashMap::from([("g".to_string(), returning)]);
    let env = EvalEnv::new(globals);
    let main = Term::nat_add(Term::Global("g".into()), nat(1));
    assert_eq!(
        as_number(&eval_with_env(&main, &env).expect("main runs")),
        8
    );
    assert_eq!(env.lookup("g"), GlobalLookup::Value(nat(7)));
}

/// An argument is not a function body: a `return` inside one exits the
/// function holding the call, and the extern never runs.
#[test]
fn a_return_in_an_extern_argument_exits_the_caller() {
    let assertion = Term::ExternCall(
        "__c_tg_assert_eq_bool".to_string(),
        vec![Term::early_return(Term::var("n")), Term::True],
    );
    let body = Term::let_in("_", Type::Unit, assertion, nat(0));
    let call = Term::app(Term::lambda("n", Type::Nat, body), nat(5));
    let env = EvalEnv::empty();
    assert_eq!(eval_with_env(&call, &env), Ok(nat(5)));
    assert_eq!(env.assertions_executed(), 0);
}

#[test]
fn a_returned_at_the_top_is_the_value_on_every_driver() {
    let top = Term::nat_add(Term::early_return(nat(4)), nat(1));
    let env = EvalEnv::empty();
    assert_eq!(eval_with_env(&top, &env), Ok(nat(4)));
    assert_eq!(eval_with_env_and_limit(&top, &env, 100), Ok(nat(4)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    assert_eq!(eval_with_env_until(&top, &env, deadline), Ok(nat(4)));
}

#[test]
fn a_stuck_run_hands_back_no_call_frame() {
    let env = EvalEnv::empty();
    let stuck = eval_with_env(&stuck_inside_a_frame(), &env).expect("stuck is not a stop");
    assert!(!holds_a_call_frame(&stuck), "{stuck:?}");
    let limited = eval_with_env_and_limit(&stuck_inside_a_frame(), &env, 100)
        .expect("stuck within the limit");
    assert!(!holds_a_call_frame(&limited), "{limited:?}");
    assert_eq!(
        eval_with_env_and_limit(&stuck_inside_a_frame(), &env, 2),
        Err(EvalStopped::StepLimit { limit: 2 })
    );
}
