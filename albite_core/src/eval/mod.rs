//! # Evaluation Module
//!
//! This module provides a small-step call-by-value evaluator for the
//! Tungsten core calculus.
//!
//! ## Overview
//!
//! The evaluator implements the standard operational semantics for a
//! dependently-typed lambda calculus with:
//!
//! - Lambda abstraction and application (β-reduction)
//! - Let bindings (substitution)
//! - Booleans with if-then-else
//! - Natural numbers with primitive recursion (natrec)
//! - Products (pairs) and sums (tagged unions)
//! - Polymorphism (type abstraction/application)
//! - Propositional equality (refl and subst)
//! - Strings with concatenation, length, and equality
//! - General recursion (fix combinator)
//! - Recursive types (fold/unfold for μ-types)
//! - Nat comparison operations (lt, le, gt, ge)
//! - String character access (`char_at`, substring)
//! - FFI calls (`extern_call`) - stuck in pure evaluation
//! - Mutable references (`ref_new`, `ref_get`, `ref_set`) - stuck in pure evaluation
//!
//! ## Evaluation Strategies
//!
//! The module provides two evaluation strategies:
//!
//! ### Without Environment (`step`, `eval`)
//!
//! The standard small-step evaluator that works on closed terms.
//! Global references are stuck - this is suitable for evaluating
//! self-contained expressions.
//!
//! ### With Environment (`step_with_env`, `eval_with_env`)
//!
//! Environment-based evaluation with call-by-need semantics. The evaluator
//! maintains an `EvalEnv` that maps global names to their definitions.
//! Lookups are memoized to provide call-by-need (lazy evaluation with sharing).
//!
//! This is the recommended approach for evaluating programs with multiple
//! definitions, as it avoids exponential term blowup from naive substitution.

mod env;
mod helpers;
mod step;

// Re-export main types and functions
pub use env::{
    eval_with_env, eval_with_env_and_limit, eval_with_env_until, extern_registry, step_with_env,
    ComparatorFailure, ComparatorFailureKind, ComparatorSynth, EvalEnv, EvalStopped, GlobalLookup,
    IntTrapKind,
};
// The checked byte buffer's shared surface — its symbols, its ceiling and its
// UTF-8 lead table — which the code generator lowers from (ADR 3.10.26n).
pub use env::handlers::externs::bytes as byte_intrinsics;
pub use helpers::{nat_to_term, term_to_nat};
pub use step::step;

/// The polymorphic structural-comparator intrinsic symbol (ADR 29.6.26f §T11.2a).
///
/// `compare(a, b)` lowers to `App(App(TyApp(Global(COMPARE_INTRINSIC), T), …), …)`.
/// At runtime, `step_tyapp_env` resolves `TyApp(Global(COMPARE_INTRINSIC), T)` for a
/// concrete `T` by invoking the [`EvalEnv`]'s [`ComparatorSynth`] callback (an
/// Bootstrap-era seam: synthesis is Rust in `bootstrap`, which `albite_core` cannot
/// depend on). Kept in sync with `bootstrap::comparator::COMPARE_INTRINSIC`.
pub const COMPARE_INTRINSIC: &str = "__cmp";

use crate::terms::Term;

// ============================================================================
// StepResult
// ============================================================================

/// Result of one step of evaluation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    /// Term reduced to a new term
    Stepped(Term),
    /// Term is already a value (cannot step further)
    Value,
    /// Term is stuck (e.g., open term, sorry)
    Stuck,
    /// A `return` of this value is unwinding to the nearest call frame (ADR
    /// 13.10.26n D2). Every congruence rule passes it up unchanged; a
    /// `Term::CallFrame` or a driver loop's top turns it into the value.
    Returned(Term),
}

// ============================================================================
// Public evaluation functions
// ============================================================================

/// Evaluate a term to a value (or stuck state)
///
/// Uses the environment-free evaluator. Global references will be stuck.
/// For programs with global definitions, use [`eval_with_env`] instead.
#[must_use]
pub fn eval(term: &Term) -> Term {
    let mut current = term.strip_spans();
    loop {
        match step(&current) {
            StepResult::Stepped(next) => current = next,
            finished => return finished_term(current, finished),
        }
    }
}

/// Evaluate with step limit (to detect non-termination)
///
/// Returns `None` if the step limit is exceeded, `Some(value)` otherwise.
/// Uses the environment-free evaluator.
#[must_use]
pub fn eval_with_limit(term: &Term, limit: usize) -> Option<Term> {
    let mut current = term.strip_spans();
    for _ in 0..limit {
        match step(&current) {
            StepResult::Stepped(next) => current = next,
            finished => return Some(finished_term(current, finished)),
        }
    }
    None // Step limit exceeded
}

/// What a driver loop hands back once stepping ends (ADR 13.10.26n D3).
///
/// A `Returned(v)` at the top is the run's value: a `main` or `test_*` body
/// is evaluated without an application, so the loop is its function
/// boundary. A stuck term is handed back with its call frames stripped, so
/// no caller that writes an evaluated term into Core or a type sees one.
/// `current` is the term the final `step` was asked about.
pub(crate) fn finished_term(current: Term, finished: StepResult) -> Term {
    match finished {
        StepResult::Returned(value) => value,
        StepResult::Stuck => current.strip_call_frames(),
        StepResult::Value | StepResult::Stepped(_) => current,
    }
}
