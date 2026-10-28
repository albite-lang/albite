//! Expression elaboration: Surface Expr → Core Term
//!
//! Implements bidirectional type checking:
//! - `check(expr, expected)` - check expression against known type
//! - `infer(expr)` - synthesize type from expression
//!
//! This approach gives better error messages and requires fewer annotations
//! than full Hindley-Milner inference.
//!
//! ## Module Structure
//!
//! The elaboration logic is split across several modules:
//! - [`check_infer`] - Main bidirectional check/infer entry points
//! - [`application`] - Function application and type inference
//! - [`blocks`] - Let, block, and statement elaboration
//! - [`match_expr`] - Match expression entry point
//! - [`adt_match`] - ADT/constructor pattern matching
//! - [`patterns`] - Nested pattern elaboration
//! - [`constructors`] - Constructor elaboration and injection
//! - [`helpers`] - Type utilities and helper functions
//! - [`errors`] - Error helper methods
//! - [`forms`] - Expression form elaborators (lambda, operators, builtins,
//!   proofs, records, tuples, type_args)

mod adt_match;
pub(in crate::elaborate) use adt_match::ownership;
mod application;
mod blocks;
mod blocks_tuples;
mod check_infer;
mod compare;
mod constructors;
mod errors;
mod forms;
mod helpers;
mod if_let;
mod int_match;
mod let_else;
mod match_expr;
mod patterns;
mod try_block;

/// The pure motive helpers, for the elaborator's tests (ADR 29.9.26d AC 6)
/// and `induction`'s goals (ADR 3.10.26e D2).
pub(crate) use check_infer::natind as natind_motive;
// `rewrite` builds the combinators' motive and `sym` (ADR 2.10.26e D4).
pub(crate) use check_infer::combinators;
// `cases` and `left`/`right` read the same disjunction test (ADR 7.10.26l D3).
pub(crate) use forms::{is_disjunction, Side};
pub(crate) use try_block::{crossing_exit_word, find_crossing_exit};
