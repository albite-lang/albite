//! Helper functions for evaluation
//!
//! This module provides utility functions used by both the standard
//! and environment-based evaluators.

use crate::terms::Term;

use super::StepResult;

// ============================================================================
// Nat conversion helpers
// ============================================================================

/// Convert a Nat term (zero/succ chain or `NatLit`) to a usize
///
/// Returns `None` if the term is not a normalized natural number.
#[must_use]
pub fn term_to_nat(term: &Term) -> Option<usize> {
    match term {
        Term::Zero => Some(0),
        Term::Succ(inner) => term_to_nat(inner).map(|n| n + 1),
        Term::NatLit(n) => Some(*n as usize),
        _ => None,
    }
}

/// Convert a usize to a Nat term.
///
/// Delegates to `Term::nat_smart` (ADR 21.7.26e wall 2): computed results —
/// arithmetic, FFI marshalling — must not materialize deep unary `Succ`
/// chains any more than literals may, or a large computed Nat re-opens the
/// stack-guard-page wedge that `nat_smart`'s threshold closes.
#[must_use]
pub fn nat_to_term(n: usize) -> Term {
    Term::nat_smart(n as u64)
}

// ============================================================================
// Call frames (ADR 13.10.26n D3)
// ============================================================================

/// Frame an instantiated function body when its *uninstantiated* `body`
/// holds a `return` outside every nested lambda, so a non-tail `return`
/// unwinds to this call and no further. The walk runs before substitution:
/// it gives the same answer, on a smaller term. A body with no `return` is
/// handed back unframed and costs one walk.
pub(crate) fn frame_if_returning(body: &Term, instantiated: Term) -> Term {
    if body.has_unguarded_return() {
        Term::call_frame(instantiated)
    } else {
        instantiated
    }
}

// ============================================================================
// Binary comparison helper
// ============================================================================

/// Helper for binary Nat comparison operations
///
/// Evaluates both operands and applies the comparison function. A stepped
/// operand is rebuilt with `constructor`, the operator this call was made for
/// (ADR 13.10.26e D1), exactly as `step_binary_nat_env` does.
/// Note: This function uses the environment-free `step()` internally,
/// so it's only suitable for the non-environment evaluator.
pub(crate) fn step_binary_nat_compare<F>(
    t1: &Term,
    t2: &Term,
    cmp: F,
    constructor: fn(Term, Term) -> Term,
) -> StepResult
where
    F: Fn(usize, usize) -> bool,
{
    use super::step::step;

    // Evaluate left operand first
    if !t1.is_value() {
        match step(t1) {
            StepResult::Stepped(t1_new) => {
                return StepResult::Stepped(constructor(t1_new, t2.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // Evaluate right operand
    if !t2.is_value() {
        match step(t2) {
            StepResult::Stepped(t2_new) => {
                return StepResult::Stepped(constructor(t1.clone(), t2_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // Both values: compare
    match (term_to_nat(t1), term_to_nat(t2)) {
        (Some(a), Some(b)) => {
            if cmp(a, b) {
                StepResult::Stepped(Term::True)
            } else {
                StepResult::Stepped(Term::False)
            }
        }
        _ => StepResult::Stuck,
    }
}

// ============================================================================
// Binary Nat arithmetic helper
// ============================================================================

/// Helper for binary Nat arithmetic operations (add, sub, mul, div, mod)
///
/// Evaluates both operands and applies the arithmetic function, rebuilding a
/// stepped operand with `constructor` (ADR 13.10.26e D1). `op` returning
/// `None` means the result is not a Nat (`+`/`*` past the largest one), and
/// the term is `Stuck`: a neutral equal only to itself, rather than a
/// saturated value that would make `MAX + 1` convertible with `MAX` (D2).
pub(crate) fn step_binary_nat<F>(
    t1: &Term,
    t2: &Term,
    op: F,
    constructor: fn(Term, Term) -> Term,
) -> StepResult
where
    F: Fn(usize, usize) -> Option<usize>,
{
    use super::step::step;

    // Evaluate left operand first
    if !t1.is_value() {
        match step(t1) {
            StepResult::Stepped(t1_new) => {
                return StepResult::Stepped(constructor(t1_new, t2.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // Evaluate right operand
    if !t2.is_value() {
        match step(t2) {
            StepResult::Stepped(t2_new) => {
                return StepResult::Stepped(constructor(t1.clone(), t2_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // Both values: compute
    match (term_to_nat(t1), term_to_nat(t2)) {
        (Some(a), Some(b)) => op(a, b).map_or(StepResult::Stuck, |result| {
            StepResult::Stepped(nat_to_term(result))
        }),
        _ => StepResult::Stuck,
    }
}

// ============================================================================
// Binary Bool helper
// ============================================================================

/// Helper for binary Bool operations (and, or)
///
/// Evaluates both operands and applies the boolean function, rebuilding a
/// stepped operand with `constructor` (ADR 13.10.26e D1).
pub(crate) fn step_binary_bool<F>(
    t1: &Term,
    t2: &Term,
    op: F,
    constructor: fn(Term, Term) -> Term,
) -> StepResult
where
    F: Fn(bool, bool) -> bool,
{
    use super::step::step;

    // Helper to convert Term to bool
    fn term_to_bool(t: &Term) -> Option<bool> {
        match t {
            Term::True => Some(true),
            Term::False => Some(false),
            _ => None,
        }
    }

    // Evaluate left operand first
    if !t1.is_value() {
        match step(t1) {
            StepResult::Stepped(t1_new) => {
                return StepResult::Stepped(constructor(t1_new, t2.clone()));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // Evaluate right operand
    if !t2.is_value() {
        match step(t2) {
            StepResult::Stepped(t2_new) => {
                return StepResult::Stepped(constructor(t1.clone(), t2_new));
            }
            stop @ (StepResult::Stuck | StepResult::Returned(_)) => return stop,
            StepResult::Value => {}
        }
    }

    // Both values: compute
    match (term_to_bool(t1), term_to_bool(t2)) {
        (Some(a), Some(b)) => {
            if op(a, b) {
                StepResult::Stepped(Term::True)
            } else {
                StepResult::Stepped(Term::False)
            }
        }
        _ => StepResult::Stuck,
    }
}

// ============================================================================
// Term helper methods
// ============================================================================

/// Helper method on Term to get let binding type
impl Term {
    pub(crate) fn let_type(&self) -> Option<&crate::types::Type> {
        match self {
            Term::Let(_, ty, _, _) => Some(ty),
            _ => None,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_to_nat_zero() {
        assert_eq!(term_to_nat(&Term::Zero), Some(0));
    }

    #[test]
    fn test_term_to_nat_succ() {
        let three = Term::succ(Term::succ(Term::succ(Term::Zero)));
        assert_eq!(term_to_nat(&three), Some(3));
    }

    #[test]
    fn test_term_to_nat_non_nat() {
        assert_eq!(term_to_nat(&Term::Unit), None);
        assert_eq!(term_to_nat(&Term::True), None);
    }

    #[test]
    fn test_nat_to_term_zero() {
        assert_eq!(nat_to_term(0), Term::Zero);
    }

    #[test]
    fn test_nat_to_term_positive() {
        let result = nat_to_term(3);
        let expected = Term::succ(Term::succ(Term::succ(Term::Zero)));
        assert_eq!(result, expected);
    }

    #[test]
    fn test_roundtrip() {
        for n in 0..10 {
            let term = nat_to_term(n);
            assert_eq!(term_to_nat(&term), Some(n));
        }
    }
}
