// Clippy lint policy — ADR 18.5.26h wrote the original triage; ADR 1.10.26j
// (G8 slice 3) cut it to the families below. Each group names the slice that
// owns its names, and `tests/crate_lint_policy.rs` pins the list exactly.
// --- Docs: a docs slice, after G6's dead-definition gate ---
#![allow(clippy::doc_markdown)] // Reason: docs deferred to v2.0
#![allow(clippy::missing_errors_doc)] // Reason: docs deferred to v2.0
#![allow(clippy::missing_panics_doc)] // Reason: docs deferred to v2.0
// --- API surface: its own slice ---
#![allow(clippy::must_use_candidate)]
// Reason: adding #[must_use] raises unused_must_use at callers
// --- Error size: 26a R2's boxed-errors slice ---
#![allow(clippy::result_large_err)]
// Reason: TypeError is intentionally large; boxing changes every Result
// --- Size: governed by code-health, permanent ---
#![allow(clippy::too_many_lines)]
// Reason: function size is code-health's (100 LOC, flat-match ×1.5, allowlisted); permanent (ADR 1.10.26j D4)
// --- Signature ripple: a ripple slice, function by function ---
#![allow(clippy::match_same_arms)] // Reason: intentionally separate arms for clarity
#![allow(clippy::manual_let_else)] // Reason: existing match-based let is idiomatic
#![allow(clippy::needless_pass_by_value)] // Reason: changing to &T would ripple through call sites
#![allow(clippy::unused_self)] // Reason: methods use &self for API consistency
#![allow(clippy::only_used_in_recursion)] // Reason: recursive helper params are intentional
#![allow(clippy::ptr_arg)] // Reason: changing &PathBuf→&Path ripples through call sites
#![allow(clippy::unnecessary_wraps)]
// Reason: Result/Option wrappers for API consistency
// --- Mechanical style: `cargo clippy --fix` slices ---
#![allow(clippy::uninlined_format_args)] // Reason: purely cosmetic, partial auto-fix
#![allow(clippy::elidable_lifetime_names)] // Reason: cosmetic
#![allow(clippy::format_push_string)] // Reason: format! + push_str used for readability
#![allow(clippy::redundant_closure_for_method_calls)] // Reason: cosmetic
#![allow(clippy::redundant_closure)] // Reason: auto-fix applied most; remainder are method calls
#![allow(clippy::stable_sort_primitive)] // Reason: sort() vs sort_unstable() perf is negligible here
#![allow(clippy::wildcard_imports)] // Reason: glob imports used for prelude-style modules
#![allow(clippy::map_unwrap_or)] // Reason: map().unwrap_or() reads better than map_or()
#![allow(clippy::clone_on_copy)] // Reason: partial auto-fix applied
#![allow(clippy::single_match_else)] // Reason: explicit match preferred for clarity
#![allow(clippy::unnecessary_map_or)] // Reason: map_or used for readability
#![allow(clippy::if_not_else)] // Reason: negated conditions sometimes read better
#![allow(clippy::format_collect)] // Reason: format!+collect pattern used in codegen
#![allow(clippy::implicit_clone)] // Reason: .to_string() on &String is idiomatic// --- Casts: a casts slice ---
#![allow(clippy::cast_possible_truncation)] // Reason: handle/index casts checked at call sites
#![allow(clippy::cast_precision_loss)]
// Reason: intentional f64 conversions for stats/profiling
// --- rustc, in progress: with G6 (dead-definition detection) ---
#![allow(unused_imports)] // Reason: modules under active development
#![allow(unused_variables)] // Reason: modules under active development
#![allow(dead_code)] // Reason: modules under active development

//! Bootstrap compiler for the Tungsten proof language.
//!
//! This crate provides lexing, parsing, and elaboration
//! for Tungsten source code. It produces surface AST that can be
//! elaborated into core terms for type checking via `albite_core`.
//!
//! # Architecture
//!
//! ```text
//! .tg source
//!     │
//!     ▼
//! ┌─────────┐
//! │  Lexer  │  → Vec<Token>
//! └─────────┘
//!     │
//!     ▼
//! ┌─────────┐
//! │ Parser  │  → SourceFile (AST)
//! └─────────┘
//!     │
//!     ▼
//! ┌────────────┐
//! │ Elaborator │ → albite_core::Term
//! └────────────┘
//!     │
//!     ▼
//! ┌─────────────┐
//! │ Type Check  │  (albite_core)
//! └─────────────┘
//! ```
//!
//! # Example
//!
//! ```
//! use albite_bootstrap::{parse, elaborate};
//! use albite_core::Context;
//!
//! let source = r#"
//!     fn add(x: Nat, y: Nat) -> Nat {
//!         x + y
//!     }
//! "#;
//!
//! // Parse the source
//! let (ast, parse_errors) = parse(source);
//!
//! if parse_errors.is_empty() {
//!     // Elaborate to Core
//!     let mut ctx = Context::new();
//!     match elaborate(&ast, &mut ctx) {
//!         Ok(defs) => println!("Elaborated {} definitions", defs.len()),
//!         Err(errors) => {
//!             for err in errors {
//!                 eprintln!("Error: {}", err);
//!             }
//!         }
//!     }
//! }
//! ```

pub mod ast;
pub mod cache;
pub mod comparator;
pub mod config;
pub mod doctor;
pub mod driver;
pub mod elaborate;
pub mod error;
pub mod fold_analysis;
pub mod lexer;
pub mod parser;
pub mod scratch;
pub mod sidecar;
pub mod span;
pub mod token;
pub mod utils;
// Re-exports for convenience
pub use ast::{
    AxiomDef, BinOp, Expr, Field, FunctionDef, Ident, Item, LambdaParam, LiteralPattern, MatchArm,
    Param, Pattern, Sorry, SourceFile, Stmt, Tactic, TacticBlock, TheoremBody, TheoremDef,
    TypeAlias, TypeDef, TypeExpr, TypeParam, UnaryOp, Variant,
};
pub use cache::BuildCache;
/// The evaluation and cache seams `fuzz-test`'s properties drive (ADR 5.10.26d
/// D3): a step-bounded `main`, the stuck-residual diagnosis, and a quiet,
/// cache-honouring elaboration with its cold/warm equivalence check.
pub use driver::run_mode::{bounded_eval, head_label, stuck_diagnosis};
pub use driver::{compare_elab_outputs, elaborate_file_quiet, ElabDiff, QuietElabFailure};
pub use elaborate::{
    elaborate, elaborate_with_warnings, elaborate_with_warnings_full, AdtOrigin, CoreDef,
    ElabError, ElabErrorKind, ElabOutput, Elaborator, Env, Note, TraceFrame, TypeProvenance,
};
pub use error::{
    Diagnostic, DiagnosticRenderer, Label, LexError, ParseError, Severity, Suggestion,
};
pub use lexer::Lexer;
pub use parser::{parse, Parser};
pub use span::{LineIndex, Location, Span, Spanned};
pub use token::{keyword_from_str, Token, TokenKind};
