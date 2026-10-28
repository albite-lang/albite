// Clippy lint policy — ADR 18.5.26h triaged it; ADR 2.10.26k retired 19 of
// its 21 crate-level names (fixed, or allowed per item with a reason) and
// albite_codegen/tests/crate_lint_policy.rs pins the two that remain.
#![allow(clippy::too_many_lines)] // Reason: permanent — function length is code-health's (`.code-health/thresholds.toml`)
#![allow(clippy::cast_possible_truncation)]
// Reason: usize/u64 → u32 at LLVM's index and arity parameters (11 sites); owned by G8's casts slice

//! LLVM Code Generation for Tungsten
//!
//! This crate generates LLVM IR from Tungsten Core terms, enabling
//! compilation to native executables.
//!
//! # Architecture
//!
//! ```text
//! Core Term
//!     │
//!     ▼
//! ┌────────────────┐
//! │ Free Variable  │  → Collect free vars for closures
//! │   Analysis     │
//! └────────────────┘
//!     │
//!     ▼
//! ┌────────────────┐
//! │ Type Lowering  │  → Map Core types to LLVM types
//! └────────────────┘
//!     │
//!     ▼
//! ┌────────────────┐
//! │   Codegen      │  → Generate LLVM IR
//! └────────────────┘
//!     │
//!     ▼
//! ┌────────────────┐
//! │ LLVM Backend   │  → Object file / Executable
//! └────────────────┘
//! ```
//!
//! # Type Mapping
//!
//! | Tungsten Type | LLVM Type                    |
//! |---------------|------------------------------|
//! | Bool          | i1                           |
//! | Nat           | i64                          |
//! | Unit          | {}                           |
//! | String        | { i8*, i64 }                 |
//! | τ₁ → τ₂       | { fn_ptr, env_ptr }          |
//! | τ₁ × τ₂       | { τ₁_llvm, τ₂_llvm }         |
//! | τ₁ + τ₂       | { i8 tag, union { τ₁, τ₂ } } |
//! | ∀α. τ         | (erased, same as τ)          |
//! | μα. τ         | opaque pointer               |

mod analysis;
mod codegen;
pub mod escape_analysis;
mod types;

pub use codegen::musttail_report::{
    self, Blocker, BlockerPosition, Decision, MusttailDecision, MusttailReport, ParamAbiKind,
    ReasonCode,
};
pub use codegen::native_core::{
    is_native_core_export, native_core_symbol_name, NativeCoreExport, NativeCoreSwitch,
    NATIVE_CORE_EXPORTS, NATIVE_CORE_PIPELINE, NATIVE_CORE_PREFIX,
};
pub use codegen::{CodeGen, CodeGenError, ComparatorSynth, SymbolEntry};
pub use types::{AdtDef, CodegenConstructor, Route, TypeLowering};

// Re-export inkwell for consumers
pub use inkwell;
