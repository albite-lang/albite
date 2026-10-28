//! `let mut` and assignment as rebinding, `B4.2` (ADR 13.10.26r D3).
//!
//! An assignment `x = e;` is a shadowing `let`, and an `if`, `match`, `if let`
//! or block whose arms assign a `mut` binding declared outside it is a
//! **join**: each arm ends by pairing its value with the current values of the
//! bindings the construct assigns, and the result is bound and unpacked after
//! it. So no Core term is added — the kernel, the evaluator, CIR and both code
//! generators see only `let`, `if`, `match` and pairs.
//!
//! ```text
//!  let mut x = 1;                   let mut x = 1;
//!  if c { x = 2; } else { };   →    let x#in = x;
//!                                   let x#j = if c { let mut x = x#in; x = 2;
//!                                                    let x#1 = x; x#1 }
//!                                             else { x#in };
//!                                   x = x#j;
//! ```
//!
//! Every value a join pairs is read through a name no source can spell (it
//! holds a `#`), never through the source name, so a shadowing `let x` inside
//! an arm cannot change what the arm hands back (the hygiene rule).
//!
//! [`lower_joins`] is pure over the parsed block and an [`OuterScope`]: at
//! elaboration the scope is read from the environment (D5), and the handle
//! walk (D6) lowers each block over an empty one. The three refusals it can
//! raise are E0104 (not `mut`, or not joined), E0105 (a closure assigns an
//! outer `mut`) and E0106 (a region body assigns an outer `mut`).

use std::collections::HashMap;

use crate::ast::{Expr, Stmt, TypeExpr};
use crate::elaborate::error::MutableErrorKind;
use crate::span::Span;

pub(crate) use arms::pattern_names;
pub(crate) use elab::mutable_error;

mod arms;
mod elab;
mod lower;
#[cfg(test)]
mod tests;

/// What the scope outside a lowered block knows about one name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BindingFacts {
    /// Declared with `let mut` (or rebound by an assignment).
    pub mutable: bool,
    /// How many `region`s enclosed the declaration.
    pub region_depth: usize,
    /// How many lambdas enclosed the declaration.
    pub closure_depth: usize,
}

/// The scope a lowering starts from: what is bound outside the block, and how
/// deep in regions and lambdas the block itself sits.
pub(crate) struct OuterScope<'a> {
    pub lookup: &'a dyn Fn(&str) -> Option<BindingFacts>,
    pub region_depth: usize,
    pub closure_depth: usize,
}

impl OuterScope<'_> {
    /// A scope that knows no outer name: an assignment to one is left for the
    /// elaborator to refuse. The handle walk lowers over this (D6).
    pub(crate) const EMPTY: OuterScope<'static> = OuterScope {
        lookup: &|_| None,
        region_depth: 0,
        closure_depth: 0,
    };
}

/// Where a name a join unpacks came from (D6): the construct's value, or the
/// `mut` binding of that name the construct assigned.
#[derive(Debug, Clone)]
pub(crate) enum JoinSource {
    Value(ValueHint),
    Binding(String),
}

/// Where the elaborator can read a joined value's type, so the join is
/// checked rather than inferred and a `Nil` arm cannot fix it too early (D3).
#[derive(Debug, Clone)]
pub(crate) enum ValueHint {
    /// The `let`'s annotation.
    Annotation(TypeExpr),
    /// The lowered block's expected type: the construct is its tail.
    BlockExpected,
    /// The type of the binding the construct's value is assigned to.
    TypeOf(String),
    Unknown,
}

/// For each name a join unpacks, its source. Keyed by the unpacked name, which
/// is unique within the lowered block.
pub(crate) type JoinMap = HashMap<String, JoinSource>;

/// A refusal raised while lowering, at the assignment (or exit) it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutableRefusal {
    pub kind: MutableErrorKind,
    pub span: Span,
}

/// A block after lowering: its statements, its tail, and the joins it holds.
#[derive(Debug, Clone)]
pub(crate) struct LoweredBlock {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Expr>,
    pub joins: JoinMap,
}

/// Rewrite every assignment in a block, and every join-position construct
/// whose arms assign, into `let`s (D3). `block_start` makes the introduced
/// names unique to this block, so a nested block lowered later cannot reuse
/// one of them.
pub(crate) fn lower_joins(
    stmts: &[Stmt],
    tail: Option<&Expr>,
    scope: &OuterScope<'_>,
    block_start: u32,
) -> Result<LoweredBlock, MutableRefusal> {
    lower::Lowerer::new(scope, block_start).lower_root(stmts, tail)
}

/// Whether the block, or a construct in one of its join positions, holds an
/// assignment — the only case [`lower_joins`] changes anything.
pub(crate) fn needs_lowering(stmts: &[Stmt], tail: Option<&Expr>) -> bool {
    stmts.iter().any(stmt_needs_lowering) || tail.is_some_and(construct_assigns)
}

fn stmt_needs_lowering(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Assign(..) => true,
        Stmt::Let(_, _, value, _) | Stmt::LetMut(_, _, value, _) | Stmt::Expr(value, _) => {
            construct_assigns(value)
        }
        Stmt::LetElse(..) | Stmt::Item(_) => false,
    }
}

/// Whether `expr` is a join-position construct one of whose arms assigns.
fn construct_assigns(expr: &Expr) -> bool {
    let block_assigns = |arm: &Expr| match arm {
        Expr::Block(stmts, tail, _) => needs_lowering(stmts, tail.as_deref()),
        other => construct_assigns(other),
    };
    match expr {
        Expr::Block(stmts, tail, _) => needs_lowering(stmts, tail.as_deref()),
        Expr::If(_, then, otherwise, _) => block_assigns(then) || block_assigns(otherwise),
        Expr::IfLet(_, _, then, otherwise, _) => {
            block_assigns(then) || otherwise.as_deref().is_some_and(block_assigns)
        }
        Expr::Match(_, arms, _) => arms.iter().any(|arm| block_assigns(&arm.body)),
        _ => false,
    }
}

/// Whether `name` was introduced by a lowering: no source can spell a `#`.
pub(crate) fn is_synthesised(name: &str) -> bool {
    name.contains('#')
}
