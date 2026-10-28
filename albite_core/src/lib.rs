// Clippy lint policy — ADR 18.5.26h triaged it; ADR 28.9.26h cut the
// crate-level list from 23 names to these 2. Three more are confined to the
// module root holding every site (`typecheck/`, `ffi/`, `ffi/driver/`), and
// the rest are fixed or allowed per item with a `// Reason:`.
// `tests/crate_lint_policy.rs` pins all three tiers.
#![allow(
    clippy::too_many_lines,           // Reason: permanent — code-health owns function size, with a flat-match exemption clippy lacks
    clippy::cast_possible_truncation, // Reason: mostly u64 → usize, which truncates on the wasm32 playground build; paid down by a later G8 slice
)]

//! # Tungsten Core Calculus
//!
//! This crate implements the Phase 1 core calculus for the Tungsten proof language.
//!
//! ## Overview
//!
//! The core calculus is a simply-typed lambda calculus with:
//! - Base types: `Bool`, `Nat`, `Unit`, `Void`, `Prop`
//! - Type constructors: `→`, `×`, `+`, `∀α`, `Eq τ t t`
//! - Polymorphism: impredicative (∀α. τ; type application accepts any
//!   well-formed type, including a `∀`-type; ADR 28.9.26c)
//! - Propositional equality: `Eq τ t₁ t₂` with `refl` and `subst`
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    TUNGSTEN CORE                            │
//! ├─────────────────────────────────────────────────────────────┤
//! │  types.rs    - Type syntax (τ)                              │
//! │  terms.rs    - Term syntax (t)                              │
//! │  context.rs  - Typing contexts (Γ)                          │
//! │  typecheck.rs - Typing judgment (Γ ⊢ t : τ)                 │
//! │  eval.rs     - Evaluation (t → t')                          │
//! │  conversion/ - δ-free open-term conversion (t ≡ t')         │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! ```rust
//! use albite_core::prelude::*;
//!
//! // Build a term: (λx:Nat. x) zero
//! let id = Term::lambda("x", Type::Nat, Term::var("x"));
//! let term = Term::app(id, Term::Zero);
//!
//! // Type check
//! let ctx = Context::new();
//! let ty = type_of(&ctx, &term).unwrap();
//! assert_eq!(ty, Type::Nat);
//!
//! // Evaluate
//! let result = eval(&term);
//! assert_eq!(result, Term::Zero);
//! ```
//!
//! ## Phase 1 Specification
//!
//! ### Types
//!
//! ```text
//! τ ::= Bool
//!     | Nat
//!     | Unit
//!     | Void
//!     | Prop
//!     | τ → τ
//!     | τ × τ
//!     | τ + τ
//!     | α
//!     | ∀α. τ
//!     | Eq τ t t
//! ```
//!
//! ### Terms
//!
//! ```text
//! t ::= x                                      -- variable
//!     | λx:τ. t                                -- lambda
//!     | t t                                    -- application
//!     | let x : τ = t in t                     -- let binding
//!     | true | false                           -- booleans
//!     | if t then t else t                     -- conditional
//!     | ()                                     -- unit
//!     | absurd [τ] t                           -- void elimination
//!     | zero                                   -- nat zero
//!     | succ t                                 -- nat successor
//!     | natrec [τ] t t t                       -- nat recursion
//!     | natind [P] t t t                       -- nat induction
//!     | (t, t)                                 -- pair
//!     | fst t                                  -- first projection
//!     | snd t                                  -- second projection
//!     | inl [τ + τ] t                          -- left injection
//!     | inr [τ + τ] t                          -- right injection
//!     | case t of inl x => t | inr y => t      -- case analysis
//!     | Λα. t                                  -- type abstraction
//!     | t [τ]                                  -- type application
//!     | refl [τ] t                             -- equality introduction
//!     | subst [τ] [P] t t                      -- equality elimination
//!     | (t : τ)                                -- type annotation
//!     | sorry                                  -- unsafe axiom
//! ```

pub mod builtins;
pub mod context;
pub mod conversion;
pub mod diagnostics;
pub mod eval;
pub mod ffi;
pub mod link_names;
pub mod native_core;
pub mod packages;
pub mod terms;
pub mod typecheck;
pub mod types;

// Force-link the runtime crate so its #[no_mangle] symbols are
// exported from our cdylib (signal handler, diagnostics).
extern crate albite_runtime;

/// Prelude for convenient imports
pub mod prelude {
    pub use crate::context::Context;
    pub use crate::eval::{eval, eval_with_limit, step, StepResult};
    pub use crate::terms::Term;
    pub use crate::terms::{SpannedTerm, TermSpan};
    pub use crate::typecheck::{
        check, check_proof, check_type_wf, type_of, type_of_proof, TypeError, TypeResult,
    };
    pub use crate::types::Type;
}

// Re-export main types at crate root
pub use context::Context;
pub use eval::{eval, step};
pub use terms::Term;
pub use terms::{SpannedTerm, TermSpan};
pub use typecheck::{check_proof, type_of, type_of_proof, types_equal, TypeError};
pub use types::{types_equal_alpha, Type};

#[cfg(test)]
mod integration_tests;
