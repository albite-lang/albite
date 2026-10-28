//! The pure half of [`super::lower_joins`] (ADR 13.10.26r D3): a join's
//! arms, how an arm ends by pairing, and how the construct is rebuilt around
//! the paired arms. Split from `lower.rs`, which keeps the walk and its state.

use std::collections::HashMap;

use crate::ast::{Expr, Ident, MatchArm, Path, Pattern, Stmt};
use crate::span::{Span, Spanned};

use super::ValueHint;

/// Where a binding was declared: its frame, then its position in the frame.
/// Sorting by it gives a join's bindings in declaration order.
pub(super) type Declared = (usize, usize);

/// The outer bindings an arm assigned, and the fresh name each was last
/// assigned through.
#[derive(Default)]
pub(super) struct ArmState {
    pub(super) assigned: Vec<(Declared, String)>,
    pub(super) latest: HashMap<String, String>,
}

/// A block after lowering, with what it assigned when it was an arm.
pub(super) struct LoweredArm {
    pub(super) stmts: Vec<Stmt>,
    pub(super) tail: Option<Expr>,
    pub(super) span: Span,
    pub(super) state: ArmState,
}

impl LoweredArm {
    pub(super) fn into_block(self) -> Expr {
        Expr::Block(self.stmts, self.tail.map(Box::new), self.span)
    }
}

/// End an arm by pairing its value (when wanted) with each target's latest
/// fresh name, or its snapshot when the arm left it alone.
pub(super) fn pair_arm(
    arm: LoweredArm,
    targets: &[(Declared, String)],
    snapshots: &[String],
    want_value: bool,
) -> Expr {
    let span = arm.span;
    let mut stmts = Vec::with_capacity(arm.stmts.len() + targets.len() + 1);
    for ((_, name), snapshot) in targets.iter().zip(snapshots) {
        if arm.state.latest.contains_key(name) {
            stmts.push(Stmt::LetMut(name.clone(), None, path(snapshot, span), span));
        }
    }
    stmts.extend(arm.stmts);
    let mut items = Vec::with_capacity(targets.len() + 1);
    match arm.tail {
        Some(tail) if want_value => items.push(tail),
        Some(tail) => stmts.push(Stmt::Expr(tail, span)),
        None if want_value => items.push(Expr::Unit(span)),
        None => {}
    }
    for ((_, name), snapshot) in targets.iter().zip(snapshots) {
        let current = arm.state.latest.get(name).unwrap_or(snapshot);
        items.push(path(current, span));
    }
    Expr::Block(stmts, Some(Box::new(tuple_or_single(items, span))), span)
}

/// The arm a missing `else` stands for: it pairs the snapshots.
pub(super) fn pass_through_arm(snapshots: &[String], want_value: bool, span: Span) -> Expr {
    let value = want_value.then_some(Expr::Unit(span));
    let items = value
        .into_iter()
        .chain(snapshots.iter().map(|s| path(s, span)))
        .collect();
    Expr::Block(
        Vec::new(),
        Some(Box::new(tuple_or_single(items, span))),
        span,
    )
}

fn tuple_or_single(mut items: Vec<Expr>, span: Span) -> Expr {
    if items.len() == 1 {
        items.remove(0)
    } else {
        Expr::Tuple(items, span)
    }
}

/// The outer bindings any arm assigned, once each, in declaration order.
pub(super) fn join_targets(arms: &[LoweredArm]) -> Vec<(Declared, String)> {
    let mut targets: Vec<(Declared, String)> = Vec::new();
    for (declared, name) in arms.iter().flat_map(|arm| &arm.state.assigned) {
        if !targets.iter().any(|(_, seen)| seen == name) {
            targets.push((*declared, name.clone()));
        }
    }
    targets.sort();
    targets
}

/// A join position's constructs: a block, `if`, `if let` and `match`.
pub(super) fn is_construct(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Block(..) | Expr::If(..) | Expr::IfLet(..) | Expr::Match(..)
    )
}

/// Each arm's body, with the names its pattern binds.
pub(super) fn arm_sources(expr: &Expr) -> Vec<(&Expr, Vec<String>)> {
    match expr {
        Expr::If(_, then, otherwise, _) => vec![(&**then, Vec::new()), (&**otherwise, Vec::new())],
        Expr::IfLet(pattern, _, then, otherwise, _) => {
            let mut arms = vec![(&**then, pattern_names(pattern))];
            arms.extend(otherwise.as_deref().map(|other| (other, Vec::new())));
            arms
        }
        Expr::Match(_, arms, _) => arms
            .iter()
            .map(|arm| (&arm.body, pattern_names(&arm.pattern)))
            .collect(),
        block => vec![(block, Vec::new())],
    }
}

/// An arm as a block: its statements and tail.
pub(super) fn arm_parts(body: &Expr) -> (&[Stmt], Option<&Expr>) {
    match body {
        Expr::Block(stmts, tail, _) => (stmts, tail.as_deref()),
        other => (&[], Some(other)),
    }
}

/// The construct with its arms replaced; `pass_through` fills a missing
/// `if let` `else` when the construct joins.
pub(super) fn rebuild(expr: &Expr, mut bodies: Vec<Expr>, pass_through: Option<Expr>) -> Expr {
    match expr {
        Expr::If(condition, _, _, span) => {
            let otherwise = bodies.pop().expect("an else arm");
            let then = bodies.pop().expect("a then arm");
            Expr::If(
                condition.clone(),
                Box::new(then),
                Box::new(otherwise),
                *span,
            )
        }
        Expr::IfLet(pattern, scrutinee, _, otherwise, span) => {
            let fallback = if otherwise.is_some() {
                bodies.pop()
            } else {
                pass_through
            };
            let then = bodies.pop().expect("a then arm");
            Expr::IfLet(
                pattern.clone(),
                scrutinee.clone(),
                Box::new(then),
                fallback.map(Box::new),
                *span,
            )
        }
        Expr::Match(scrutinee, arms, span) => {
            let arms = arms
                .iter()
                .zip(bodies)
                .map(|(arm, body)| MatchArm {
                    body,
                    ..arm.clone()
                })
                .collect();
            Expr::Match(scrutinee.clone(), arms, *span)
        }
        _ => bodies.pop().expect("a block's one arm"),
    }
}

/// Every name a pattern binds.
pub(crate) fn pattern_names(pattern: &Pattern) -> Vec<String> {
    match pattern {
        Pattern::Var(ident) => vec![ident.name.clone()],
        Pattern::Tuple(items, _) | Pattern::Constructor(_, items, _) => {
            items.iter().flat_map(pattern_names).collect()
        }
        Pattern::Or(left, _, _) => pattern_names(left),
        Pattern::Wildcard(_) | Pattern::Literal(_) | Pattern::Error(_) => Vec::new(),
    }
}

pub(super) fn annotation_hint(ty: Option<&crate::ast::TypeExpr>) -> ValueHint {
    ty.map_or(ValueHint::Unknown, |ty| ValueHint::Annotation(ty.clone()))
}

pub(super) fn path(name: &str, span: Span) -> Expr {
    Expr::Path(Path::simple(Ident::new(name, span)))
}

pub(super) fn var(name: &str, span: Span) -> Pattern {
    Pattern::Var(Ident::new(name, span))
}

pub(super) fn let_var(name: &str, value: Expr, span: Span) -> Stmt {
    Stmt::Let(var(name, span), None, value, span)
}
