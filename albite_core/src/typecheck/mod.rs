//! Type Checker
//!
//! Implements the typing judgment Γ ⊢ t : τ for the Tungsten core calculus.
//!
//! The type checker synthesizes types from terms (bidirectional type checking).
//!
//! ## Overview
//!
//! This module provides:
//!
//! - [`type_of`]: Synthesize the type of a term given a context
//! - [`check`]: Check a term against an expected type
//! - [`types_equal`]: Check if two types are definitionally equal (α-equivalent)
//! - [`check_type_wf`]: Verify a type is well-formed under a context
//! - [`type_of_proof`] / [`check_proof`]: the proof judgment (ADR 29.9.26e)
//!
//! ## The proof judgment
//!
//! `type_of` is the executable judgment: `#[partial]` code lives there, so its
//! `fix` is unrestricted and its `return` types at `Void`. The proof judgment
//! additionally refuses a `fix` the termination engine does not certify, a
//! `return`, a `sorry`, a poisoned type and any global not bound
//! [`GlobalAdmission::Admitted`], and it compares types strictly, up to
//! conversion of `Eq` witnesses, which it types (ADR 1.10.26c). Conversion
//! unfolds only the bodies the kernel admitted itself (ADR 2.10.26l,
//! `delta.rs`): `Total` in the caller's report, effect-free, and typed by
//! `check_proof`, a self-recursive one as a `fix` it re-checks for
//! termination and unrolls only on a constructor. The mode is
//! a flag on the [`Context`](crate::context::Context), so every rule that
//! extends the context carries it. Strict positivity runs at every
//! `fold`/`unfold` in **both** modes. The gates live in `gates/`, each a
//! pure function reusing the elaborator's engine.
//!
//! ## Typing Rules
//!
//! The type checker implements standard typing rules for:
//!
//! - Lambda calculus (λ, application, let)
//! - Booleans and conditionals
//! - Natural numbers with recursion (natrec, natind)
//! - Products (pairs) and sums (tagged unions)
//! - Polymorphism (∀, type application)
//! - Propositional equality (refl, subst)
//! - Strings (Phase 2A)
//! - General recursion via fix (Phase 2A)
//! - Recursive types μα.τ (Phase 2A)
//! - Nat comparisons and string operations (Phase 3-Prep)
//! - Mutable references (Phase 3-Prep)

// Every `result_large_err` site in the crate is under this root (ADR 28.9.26h).
#![allow(clippy::result_large_err)] // Reason: TypeError is large by design; boxing hurts ergonomics

mod convert;
mod delta;
mod eq_witnesses;
mod error;
mod gates;
mod rules;
mod rules_core;
mod rules_ext;
mod rules_sum;

// Re-export public API
pub use crate::context::GlobalAdmission;
pub use delta::{
    mentioned_names, present_definition, AdmittedBody, AdmittedUnfoldings, DeltaAdmission,
    DeltaCandidate,
};
pub use error::TypeError;
pub use rules::{check, check_proof, check_type_wf, type_of, type_of_proof, types_equal};

/// Result type for type checking
pub type TypeResult<T> = Result<T, TypeError>;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests;
