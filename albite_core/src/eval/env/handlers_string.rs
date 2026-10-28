//! String operation handlers for `step_with_env()`.
//!
//! Handles: `StrLen`, `StrConcat`, `StrEq`, `StrCharAt`, `StrSubstring`

use crate::terms::Term;

use super::step_with_env;
use super::EvalEnv;
use crate::eval::helpers::{nat_to_term, term_to_nat};
use crate::eval::{EvalStopped, IntTrapKind, StepResult};

/// Step `StrLen` with environment: evaluate argument, then compute length.
pub(super) fn step_str_len_env(t: &Term, env: &EvalEnv) -> StepResult {
    if !t.is_value() {
        match step_with_env(t, env) {
            StepResult::Stepped(t_new) => return StepResult::Stepped(Term::str_len(t_new)),
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    match t {
        Term::StringLit(s) => StepResult::Stepped(nat_to_term(s.len())),
        _ => StepResult::Stuck,
    }
}

/// Step string concatenation with environment.
pub(super) fn step_str_concat_env(t1: &Term, t2: &Term, env: &EvalEnv) -> StepResult {
    if !t1.is_value() {
        match step_with_env(t1, env) {
            StepResult::Stepped(t1_new) => {
                return StepResult::Stepped(Term::str_concat(t1_new, t2.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    if !t2.is_value() {
        match step_with_env(t2, env) {
            StepResult::Stepped(t2_new) => {
                return StepResult::Stepped(Term::str_concat(t1.clone(), t2_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    match (t1, t2) {
        (Term::StringLit(s1), Term::StringLit(s2)) => {
            StepResult::Stepped(Term::string_lit(format!("{s1}{s2}")))
        }
        _ => StepResult::Stuck,
    }
}

/// Step string equality with environment.
pub(super) fn step_str_eq_env(t1: &Term, t2: &Term, env: &EvalEnv) -> StepResult {
    if !t1.is_value() {
        match step_with_env(t1, env) {
            StepResult::Stepped(t1_new) => {
                return StepResult::Stepped(Term::str_eq(t1_new, t2.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    if !t2.is_value() {
        match step_with_env(t2, env) {
            StepResult::Stepped(t2_new) => {
                return StepResult::Stepped(Term::str_eq(t1.clone(), t2_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    match (t1, t2) {
        (Term::StringLit(s1), Term::StringLit(s2)) => {
            if s1 == s2 {
                StepResult::Stepped(Term::True)
            } else {
                StepResult::Stepped(Term::False)
            }
        }
        _ => StepResult::Stuck,
    }
}

/// Step string character access with environment.
pub(super) fn step_str_char_at_env(s: &Term, n: &Term, env: &EvalEnv) -> StepResult {
    if !s.is_value() {
        match step_with_env(s, env) {
            StepResult::Stepped(s_new) => {
                return StepResult::Stepped(Term::str_char_at(s_new, n.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    if !n.is_value() {
        match step_with_env(n, env) {
            StepResult::Stepped(n_new) => {
                return StepResult::Stepped(Term::str_char_at(s.clone(), n_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // The byte at an index, or 0 at or past the end — the closed stepper's,
    // `tg_string_char_at_internal`'s and native code's guarded read alike
    // (ADR 16.10.26i D1, D3).
    match (s, term_to_nat(n)) {
        (Term::StringLit(str_val), Some(idx)) => {
            StepResult::Stepped(Term::nat_smart(crate::eval::step::byte_at(str_val, idx)))
        }
        _ => StepResult::Stuck,
    }
}

/// Step string substring with environment.
pub(super) fn step_str_substring_env(
    s: &Term,
    start: &Term,
    len: &Term,
    env: &EvalEnv,
) -> StepResult {
    if !s.is_value() {
        match step_with_env(s, env) {
            StepResult::Stepped(s_new) => {
                return StepResult::Stepped(Term::str_substring(s_new, start.clone(), len.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    if !start.is_value() {
        match step_with_env(start, env) {
            StepResult::Stepped(start_new) => {
                return StepResult::Stepped(Term::str_substring(s.clone(), start_new, len.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }
    if !len.is_value() {
        match step_with_env(len, env) {
            StepResult::Stepped(len_new) => {
                return StepResult::Stepped(Term::str_substring(s.clone(), start.clone(), len_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    match (s, term_to_nat(start), term_to_nat(len)) {
        (Term::StringLit(str_val), Some(start_idx), Some(length)) => {
            let outcome = crate::eval::step::byte_substring(str_val, start_idx, length);
            // A refusal is a cut inside a character: a trap on every reader
            // (ADR 16.10.26h D4), so the entry point reports it rather than
            // printing a residual term.
            if matches!(outcome, StepResult::Stuck) {
                env.record_stop(EvalStopped::IntTrap {
                    kind: IntTrapKind::SubstringInsideCharacter,
                });
            }
            outcome
        }
        _ => StepResult::Stuck,
    }
}
