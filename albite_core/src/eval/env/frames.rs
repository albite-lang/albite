//! `Return` and the call frame it unwinds to, environment stepper (ADR
//! 13.10.26n D2, D3).
//!
//! A `return` of a value is abrupt: it reports `Returned(v)`, every
//! congruence rule passes that up unchanged, and the nearest
//! `Term::CallFrame` — made by β-reduction around a body that returns —
//! turns it into `v`. Split from `mod.rs`, which sits at its size limit.

use crate::eval::env::{step_with_env, EvalEnv};
use crate::eval::StepResult;
use crate::terms::Term;

/// Step a `Return` or a `CallFrame`; any other term is not this module's.
pub(super) fn step_frame_env(term: &Term, env: &EvalEnv) -> StepResult {
    match term {
        Term::Return(inner) => step_return_env(inner, env),
        Term::CallFrame(body) => step_call_frame_env(body, env),
        _ => unreachable!("step_frame_env called with neither a Return nor a CallFrame"),
    }
}

/// Evaluate an extern call's argument to a value, in the *caller's* frame.
///
/// An argument is not a function body, so a `return` inside it exits the
/// function that holds the call: `Err(Returned(v))` hands that up. A stuck
/// argument is handed to the dispatch as it stands, frames stripped — the
/// residual-comparison guard there must see it (ADR 1.8.26b) — and a stop
/// the env recorded beneath it (a black hole, ADR 22.7.26a) is
/// `Err(Stuck)`: the call never runs.
pub(in crate::eval::env) fn eval_argument_env(
    arg: &Term,
    env: &EvalEnv,
) -> Result<Term, Box<StepResult>> {
    let mut current = arg.strip_spans();
    loop {
        match step_with_env(&current, env) {
            StepResult::Stepped(next) => current = next,
            StepResult::Returned(value) => return Err(Box::new(StepResult::Returned(value))),
            finished => {
                current = crate::eval::finished_term(current, finished);
                break;
            }
        }
    }
    super::finished_or_black_hole(current, env).map_err(|_| Box::new(StepResult::Stuck))
}

/// Step a Return term: once the inner value is ready, report `Returned`
/// rather than stripping the wrapper where it stands.
fn step_return_env(inner: &Term, env: &EvalEnv) -> StepResult {
    if inner.is_value() {
        return StepResult::Returned(inner.clone());
    }
    match step_with_env(inner, env) {
        StepResult::Stepped(next) => StepResult::Stepped(Term::early_return(next)),
        StepResult::Value => StepResult::Returned(inner.clone()),
        stop @ (StepResult::Stuck | StepResult::Returned(_)) => stop,
    }
}

/// Step a call frame: a `Returned(v)` from the body is caught as `v`, a
/// finished body is the frame's value, and any other step stays inside.
fn step_call_frame_env(body: &Term, env: &EvalEnv) -> StepResult {
    if body.is_value() {
        return StepResult::Stepped(body.clone());
    }
    match step_with_env(body, env) {
        StepResult::Stepped(next) => StepResult::Stepped(Term::call_frame(next)),
        StepResult::Returned(value) => StepResult::Stepped(value),
        StepResult::Value => StepResult::Stepped(body.clone()),
        StepResult::Stuck => StepResult::Stuck,
    }
}
