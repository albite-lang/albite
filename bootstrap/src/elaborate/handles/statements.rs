//! The use walk's statements (ADR 7.10.26t D4), split from `uses.rs`, and how
//! it stays correct over `let mut` and assignment (ADR 13.10.26r D6).
//!
//! Each block is lowered first, by the same `lower_joins` the elaborator runs,
//! so the walk sees rebindings and joins as `let`s. Two things keep it exact
//! over them:
//!
//! - **A name the lowering introduced is an alias, never a use.** A `let`
//!   whose binder or bare-variable definition is such a name hands the handle
//!   id across without consuming it, and a path to one (an arm's pairing) is
//!   recorded instead of consumed.
//! - **Handle-ness crosses a join through its `JoinMap`.** A joined binding is
//!   a handle exactly when its source was one in some arm, and is consumed
//!   when it was consumed at the end of some live arm — so two consuming uses
//!   after a join are still E0092.

use crate::ast::{Expr, Ident, Pattern, Stmt};
use crate::elaborate::mutable::{
    is_synthesised, lower_joins, needs_lowering, JoinSource, OuterScope,
};
use crate::span::Span;

use super::UseWalk;
use crate::elaborate::handles::policy::{bare_variable, yields_handle};

/// One pairing an arm made: the source binding's name, the handle id it
/// handed back, and whether that id was consumed on the arm's path.
pub(super) type Pairing = (String, usize, bool);

impl UseWalk<'_> {
    /// A block's statements in order, then its tail. A statement after a
    /// `return` is on no path through the block, so the walk stops there.
    pub(super) fn block(&mut self, stmts: &[Stmt], tail: Option<&Expr>, span: Span) {
        let lowered = needs_lowering(stmts, tail)
            .then(|| lower_joins(stmts, tail, &OuterScope::EMPTY, span.start).ok())
            .flatten();
        let (stmts, tail) = match &lowered {
            Some(block) => {
                self.joins.extend(block.joins.clone());
                (block.stmts.as_slice(), block.tail.as_ref())
            }
            None => (stmts, tail),
        };
        for stmt in stmts {
            if !self.live {
                return;
            }
            self.stmt(stmt);
        }
        if self.live {
            self.exprs(tail);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(Pattern::Var(ident), _, definition, _) => {
                self.bind_name(&ident.name, definition);
            }
            Stmt::Let(Pattern::Tuple(items, _), _, definition, _) if self.is_join(items) => {
                self.expr(definition);
                for item in items {
                    if let Pattern::Var(ident) = item {
                        self.bind_joined(&ident.name, definition);
                    }
                }
            }
            Stmt::Let(pattern, _, definition, _) => {
                self.expr(definition);
                self.bind_let(pattern, definition);
            }
            Stmt::LetMut(name, _, definition, _) | Stmt::Assign(name, definition, _) => {
                self.bind_name(name, definition);
            }
            Stmt::LetElse(pattern, _, definition, otherwise, _) => {
                self.let_else(definition, otherwise);
                self.bind_shadows(pattern);
            }
            Stmt::Expr(expr, _) => self.expr(expr),
            Stmt::Item(_) => {}
        }
    }

    /// `name = definition`, from a `let`, a `let mut` or an assignment.
    fn bind_name(&mut self, name: &str, definition: &Expr) {
        if let Some((source, _)) = bare_variable(definition) {
            if is_synthesised(name) || is_synthesised(source) {
                let id = self.handle_id(source);
                self.scope.push((name.to_string(), id));
                return;
            }
        }
        self.expr(definition);
        if self.joins.contains_key(name) {
            self.bind_joined(name, definition);
        } else {
            self.bind_let(&Pattern::Var(Ident::new(name, Span::empty(0))), definition);
        }
    }

    /// Bind a name a join unpacks (D6).
    fn bind_joined(&mut self, name: &str, construct: &Expr) {
        let (is_handle, consumed) = match self.joins.get(name).cloned() {
            Some(JoinSource::Binding(source)) => self.take_pairings(&source),
            Some(JoinSource::Value(_)) | None => (self.join_value_yields_handle(construct), false),
        };
        self.bind(name, is_handle);
        if consumed {
            let id = self.handle_id(name);
            self.consumed.insert(id);
        }
    }

    /// The pairings recorded for `source` since its join's arms began:
    /// whether any handed back a handle, and whether any of those was
    /// consumed on its own arm's path.
    fn take_pairings(&mut self, source: &str) -> (bool, bool) {
        let (mine, rest): (Vec<Pairing>, Vec<Pairing>) = std::mem::take(&mut self.pairings)
            .into_iter()
            .partition(|(name, _, _)| name == source);
        self.pairings = rest;
        let is_handle = mine.iter().any(|(_, id, _)| *id != 0);
        let consumed = mine.iter().any(|(_, id, consumed)| *id != 0 && *consumed);
        (is_handle, consumed)
    }

    /// Whether a joined construct's value is a handle: B4.1's rule, applied to
    /// the value each arm pairs first.
    fn join_value_yields_handle(&self, construct: &Expr) -> bool {
        let is_handle_variable = |name: &str| self.handle_id(name) != 0;
        arm_values(construct)
            .into_iter()
            .any(|value| yields_handle(value, &is_handle_variable, self.declarations))
    }

    fn is_join(&self, items: &[Pattern]) -> bool {
        items.iter().any(|item| match item {
            Pattern::Var(ident) => self.joins.contains_key(&ident.name),
            _ => false,
        })
    }

    /// Bind a `let`'s pattern: a variable bound to a handle-yielding
    /// definition is a handle variable (D4); anything else is not.
    pub(super) fn bind_let(&mut self, pattern: &Pattern, definition: &Expr) {
        match pattern {
            Pattern::Var(ident) => {
                let is_handle_variable = |name: &str| self.handle_id(name) != 0;
                let is_handle = yields_handle(definition, &is_handle_variable, self.declarations);
                self.bind(&ident.name, is_handle);
            }
            other => self.bind_shadows(other),
        }
    }

    /// A path to a name the lowering introduced: an arm's pairing, recorded
    /// with whether its handle was consumed on this path, never a use.
    pub(super) fn record_pairing(&mut self, name: &str) {
        let id = self.handle_id(name);
        let source = name.split('#').next().unwrap_or_default().to_string();
        let consumed = self.consumed.contains(&id);
        self.pairings.push((source, id, consumed));
    }
}

/// The value each arm of a joined construct pairs first.
fn arm_values(construct: &Expr) -> Vec<&Expr> {
    match construct {
        Expr::If(_, then, otherwise, _) => [&**then, &**otherwise]
            .into_iter()
            .filter_map(first)
            .collect(),
        Expr::IfLet(_, _, then, otherwise, _) => std::iter::once(&**then)
            .chain(otherwise.as_deref())
            .filter_map(first)
            .collect(),
        Expr::Match(_, arms, _) => arms.iter().filter_map(|arm| first(&arm.body)).collect(),
        block => first(block).into_iter().collect(),
    }
}

/// The first item of an arm's pairing tuple.
fn first(arm: &Expr) -> Option<&Expr> {
    let Expr::Block(_, Some(tail), _) = arm else {
        return None;
    };
    let Expr::Tuple(items, _) = &**tail else {
        return None;
    };
    items.first()
}
