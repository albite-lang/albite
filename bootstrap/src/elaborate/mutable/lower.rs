//! The rewrite behind [`super::lower_joins`] (ADR 13.10.26r D3).
//!
//! The lowerer walks a block's statements in order, keeping one frame per
//! block it is inside. A frame is an **arm** when its block is an arm of a
//! join-position construct; an assignment to a `mut` binding declared outside
//! the innermost arm is recorded there, under a fresh name, and the construct
//! that owns the arm joins it. Blocks that are not in a join position — a call
//! argument, a lambda body, a region body — are left as they are, and are
//! lowered (and refused) when the elaborator reaches them.

use std::collections::HashMap;

use crate::ast::{Expr, Ident, MatchArm, Path, Pattern, Stmt};
use crate::elaborate::error::MutableErrorKind;
use crate::elaborate::exprs::crossing_exit_word;
use crate::span::{Span, Spanned};

use super::arms::{
    annotation_hint, arm_parts, arm_sources, is_construct, join_targets, let_var, pair_arm,
    pass_through_arm, path, pattern_names, rebuild, var, ArmState, Declared, LoweredArm,
};
use super::{
    BindingFacts, JoinMap, JoinSource, LoweredBlock, MutableRefusal, OuterScope, ValueHint,
};

type Lowering<T> = Result<T, MutableRefusal>;

/// One block being lowered.
struct Frame {
    /// Its binders in order, each with whether it is `mut`.
    names: Vec<(String, bool)>,
    /// When the block is a join's arm, what it assigned outside itself.
    arm: Option<ArmState>,
}

/// What a construct became: unchanged in shape, or joined (with its value's
/// name when the position wants a value).
enum Lowered {
    Plain(Expr),
    Joined(Option<Expr>),
}

enum Resolved {
    Local {
        frame: usize,
        pos: usize,
        mutable: bool,
    },
    Outer(BindingFacts),
    Unknown,
}

pub(super) struct Lowerer<'s> {
    scope: &'s OuterScope<'s>,
    prefix: u32,
    counter: usize,
    frames: Vec<Frame>,
    joins: JoinMap,
}

impl<'s> Lowerer<'s> {
    pub(super) fn new(scope: &'s OuterScope<'s>, prefix: u32) -> Self {
        Self {
            scope,
            prefix,
            counter: 0,
            frames: Vec::new(),
            joins: JoinMap::new(),
        }
    }

    pub(super) fn lower_root(
        mut self,
        stmts: &[Stmt],
        tail: Option<&Expr>,
    ) -> Lowering<LoweredBlock> {
        let root = self.lower_block(stmts, tail, &[], false, Span::empty(self.prefix))?;
        Ok(LoweredBlock {
            stmts: root.stmts,
            tail: root.tail,
            joins: self.joins,
        })
    }

    /// A name no source can spell, unique to this lowering.
    fn fresh(&mut self, base: &str) -> String {
        self.counter += 1;
        format!("{base}#{}.{}", self.prefix, self.counter)
    }

    fn lower_block(
        &mut self,
        stmts: &[Stmt],
        tail: Option<&Expr>,
        bound: &[String],
        is_arm: bool,
        span: Span,
    ) -> Lowering<LoweredArm> {
        self.frames.push(Frame {
            names: bound.iter().map(|name| (name.clone(), false)).collect(),
            arm: is_arm.then(ArmState::default),
        });
        let lowered = self.lower_block_body(stmts, tail);
        let frame = self.frames.pop().expect("pushed above");
        let (stmts, tail) = lowered?;
        Ok(LoweredArm {
            stmts,
            tail,
            span,
            state: frame.arm.unwrap_or_default(),
        })
    }

    fn lower_block_body(
        &mut self,
        stmts: &[Stmt],
        tail: Option<&Expr>,
    ) -> Lowering<(Vec<Stmt>, Option<Expr>)> {
        let mut out = Vec::new();
        for stmt in stmts {
            self.lower_stmt(stmt, &mut out)?;
        }
        // Only the lowered block's own tail has a type the elaborator knows.
        let hint = if self.frames.len() == 1 {
            ValueHint::BlockExpected
        } else {
            ValueHint::Unknown
        };
        let tail = match tail {
            Some(tail) => Some(self.lower_value(tail, hint, &mut out)?),
            None => None,
        };
        Ok((out, tail))
    }

    fn lower_stmt(&mut self, stmt: &Stmt, out: &mut Vec<Stmt>) -> Lowering<()> {
        match stmt {
            Stmt::Let(pattern, ty, value, span) => {
                let value = self.lower_value(value, annotation_hint(ty.as_ref()), out)?;
                out.push(Stmt::Let(pattern.clone(), ty.clone(), value, *span));
                self.bind_pattern(pattern);
            }
            Stmt::LetMut(name, ty, value, span) => {
                let value = self.lower_value(value, annotation_hint(ty.as_ref()), out)?;
                out.push(Stmt::LetMut(name.clone(), ty.clone(), value, *span));
                self.bind(name, true);
            }
            Stmt::Assign(name, value, span) => {
                let hint = ValueHint::TypeOf(name.clone());
                let value = self.lower_value(value, hint, out)?;
                self.assign(name, value, *span, out)?;
            }
            Stmt::Expr(expr, span) if is_construct(expr) => {
                if let Lowered::Plain(expr) = self.lower_construct(expr, None, out)? {
                    out.push(Stmt::Expr(expr, *span));
                }
            }
            Stmt::LetElse(pattern, ..) => {
                out.push(stmt.clone());
                self.bind_pattern(pattern);
            }
            Stmt::Expr(..) | Stmt::Item(_) => out.push(stmt.clone()),
        }
        Ok(())
    }

    /// An expression in a value position: a construct there is a join
    /// position, anything else is kept as it is.
    fn lower_value(&mut self, expr: &Expr, hint: ValueHint, out: &mut Vec<Stmt>) -> Lowering<Expr> {
        if !is_construct(expr) {
            return Ok(expr.clone());
        }
        Ok(match self.lower_construct(expr, Some(hint), out)? {
            Lowered::Plain(expr) => expr,
            Lowered::Joined(value) => value.expect("a value position asks for the value"),
        })
    }

    /// Lower a join-position construct; `value` is `Some` when its position
    /// wants its value, with where that value's type can be read.
    fn lower_construct(
        &mut self,
        expr: &Expr,
        value: Option<ValueHint>,
        out: &mut Vec<Stmt>,
    ) -> Lowering<Lowered> {
        let want_value = value.is_some();
        let sources = arm_sources(expr);
        let mut arms = Vec::with_capacity(sources.len());
        for (body, bound) in &sources {
            let (stmts, tail) = arm_parts(body);
            arms.push(self.lower_block(stmts, tail, bound, true, body.span())?);
        }
        let targets = join_targets(&arms);
        if targets.is_empty() {
            let bodies = arms.into_iter().map(LoweredArm::into_block).collect();
            return Ok(Lowered::Plain(rebuild(expr, bodies, None)));
        }
        // An arm that leaves the construct cannot end by pairing (D3).
        for (body, _) in &sources {
            if let Some((span, exit)) = crossing_exit_word(body) {
                let kind = MutableErrorKind::NotJoined {
                    name: targets[0].1.clone(),
                    exit: Some(exit.to_string()),
                };
                return Err(MutableRefusal { kind, span });
            }
        }
        let span = expr.span();
        let snapshots: Vec<String> = targets
            .iter()
            .map(|(_, name)| {
                let snapshot = self.fresh(name);
                out.push(let_var(&snapshot, path(name, span), span));
                snapshot
            })
            .collect();
        let pair = |arm: LoweredArm| pair_arm(arm, &targets, &snapshots, want_value);
        let bodies = arms.into_iter().map(pair).collect();
        let pass_through = pass_through_arm(&snapshots, want_value, span);
        let construct = rebuild(expr, bodies, Some(pass_through));
        self.unpack(construct, &targets, value, span, out)
    }

    /// Bind the joined construct and hand each binding its arm's value.
    fn unpack(
        &mut self,
        construct: Expr,
        targets: &[(Declared, String)],
        hint: Option<ValueHint>,
        span: Span,
        out: &mut Vec<Stmt>,
    ) -> Lowering<Lowered> {
        let value = hint.map(|hint| (self.fresh(""), hint));
        let unpacked: Vec<String> = targets.iter().map(|(_, name)| self.fresh(name)).collect();
        if let Some((value, hint)) = &value {
            self.joins
                .insert(value.clone(), JoinSource::Value(hint.clone()));
        }
        let value = value.map(|(value, _)| value);
        for ((_, name), fresh) in targets.iter().zip(&unpacked) {
            self.joins
                .insert(fresh.clone(), JoinSource::Binding(name.clone()));
        }
        let mut names: Vec<Pattern> = value
            .iter()
            .chain(&unpacked)
            .map(|n| var(n, span))
            .collect();
        let pattern = if names.len() == 1 {
            names.remove(0)
        } else {
            Pattern::Tuple(names, span)
        };
        out.push(Stmt::Let(pattern, None, construct, span));
        for ((_, name), fresh) in targets.iter().zip(&unpacked) {
            self.assign(name, path(fresh, span), span, out)?;
        }
        Ok(Lowered::Joined(value.map(|value| path(&value, span))))
    }

    /// `name = value;`: a rebinding, recorded in the innermost arm when the
    /// binding was declared outside it.
    fn assign(&mut self, name: &str, value: Expr, span: Span, out: &mut Vec<Stmt>) -> Lowering<()> {
        let refuse = |kind| Err(MutableRefusal { kind, span });
        match self.resolve(name) {
            Resolved::Local { mutable: false, .. } => refuse(MutableErrorKind::NotMut {
                name: name.to_string(),
            }),
            Resolved::Local { frame, pos, .. } => {
                out.push(Stmt::Assign(name.to_string(), value, span));
                let arm = self.frames.iter().rposition(|frame| frame.arm.is_some());
                if let Some(arm) = arm.filter(|arm| *arm > frame) {
                    let fresh = self.fresh(name);
                    out.push(let_var(&fresh, path(name, span), span));
                    let state = self.frames[arm].arm.as_mut().expect("an arm frame");
                    if !state.latest.contains_key(name) {
                        state.assigned.push(((frame, pos), name.to_string()));
                    }
                    state.latest.insert(name.to_string(), fresh);
                }
                Ok(())
            }
            Resolved::Outer(facts) => refuse(self.outer_refusal(name, facts)),
            Resolved::Unknown => {
                out.push(Stmt::Assign(name.to_string(), value, span));
                Ok(())
            }
        }
    }

    /// An assignment to a binding declared outside the lowered block (D4):
    /// a closure's is E0105, a region's E0106, and anything else is not
    /// joined, because whatever could join it has already been lowered.
    fn outer_refusal(&self, name: &str, facts: BindingFacts) -> MutableErrorKind {
        let name = name.to_string();
        if !facts.mutable {
            MutableErrorKind::NotMut { name }
        } else if facts.closure_depth < self.scope.closure_depth {
            MutableErrorKind::Captured { name }
        } else if facts.region_depth < self.scope.region_depth {
            MutableErrorKind::InRegion { name }
        } else {
            MutableErrorKind::NotJoined { name, exit: None }
        }
    }

    fn resolve(&self, name: &str) -> Resolved {
        for (frame, entry) in self.frames.iter().enumerate().rev() {
            if let Some(pos) = entry.names.iter().rposition(|(bound, _)| bound == name) {
                let mutable = entry.names[pos].1;
                return Resolved::Local {
                    frame,
                    pos,
                    mutable,
                };
            }
        }
        match (self.scope.lookup)(name) {
            Some(facts) => Resolved::Outer(facts),
            None => Resolved::Unknown,
        }
    }

    fn bind(&mut self, name: &str, mutable: bool) {
        if let Some(frame) = self.frames.last_mut() {
            frame.names.push((name.to_string(), mutable));
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern) {
        for name in pattern_names(pattern) {
            self.bind(&name, false);
        }
    }
}
