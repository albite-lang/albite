//! The use rule (ADR 7.10.26t D4): each handle variable has at most one
//! consuming use on every path, and no observer after it.
//!
//! The walk follows evaluation order over the parsed body, carrying the set of
//! handle variables consumed so far. Sequence adds; the arms of an `if` or a
//! `match` start from the same set and join by union, so a handle consumed in
//! each arm is consumed once. A `return` ends its path, and an arm that
//! returns does not join. A closure may not mention a handle bound outside it.
//!
//! [`UsePolicy`] makes the two choices that set this rule apart from the native
//! core's `BufferReused` (6.10.26m D3) explicit: whether observers are free,
//! and whether arms join (`Max`) or add (`Sum`). `B4.1` runs [`UsePolicy::AFFINE`];
//! the other policy exists so a test can show the arm choice is live.

use std::collections::BTreeSet;

use crate::ast::{Expr, FunctionDef, IfLetCondition, MatchArm, Motive, Pattern};
use crate::elaborate::mutable::{is_synthesised, JoinMap};
use crate::span::Span;

use super::policy::{
    bare_variable, callee_name, single_segment, ArmPolicy, HandleFinding, HandleFindingKind,
    UsePolicy,
};
use super::{is_listed_handle, HandleDeclarations, HandleFunctionKind};

#[path = "regions.rs"]
mod regions;
#[path = "statements.rs"]
mod statements;

/// Every refusal in `function`'s body, in evaluation order.
#[must_use]
pub fn check_function(
    function: &FunctionDef,
    declarations: &HandleDeclarations,
    policy: UsePolicy,
) -> Vec<HandleFinding> {
    let mut walk = UseWalk::new(declarations, policy);
    for param in &function.params {
        match &param.pattern {
            Pattern::Var(ident) => walk.bind(&ident.name, is_listed_handle(&param.ty)),
            other => walk.bind_shadows(other),
        }
    }
    walk.expr(&function.body);
    walk.findings
}

/// The walk's state. A handle variable is a scope entry with a non-zero id;
/// id 0 is any other binder, which shadows a handle of its name.
/// One arm's walk, run by [`UseWalk::arms`] from the shared start state.
type ArmWalk<'f, W> = Box<dyn FnOnce(&mut W) + 'f>;

struct UseWalk<'a> {
    declarations: &'a HandleDeclarations,
    policy: UsePolicy,
    scope: Vec<(String, usize)>,
    consumed: BTreeSet<usize>,
    live: bool,
    next_id: usize,
    /// Inside a closure, every handle id below this was bound outside it.
    closure_floor: usize,
    /// Inside a region, every handle id below this was bound outside it.
    region_floor: Option<usize>,
    /// Every join the lowered blocks hold (ADR 13.10.26r D6).
    joins: JoinMap,
    /// Arm pairings not yet taken by their join.
    pairings: Vec<statements::Pairing>,
    findings: Vec<HandleFinding>,
}

impl<'a> UseWalk<'a> {
    fn new(declarations: &'a HandleDeclarations, policy: UsePolicy) -> Self {
        Self {
            declarations,
            policy,
            scope: Vec::new(),
            consumed: BTreeSet::new(),
            live: true,
            next_id: 1,
            closure_floor: 0,
            region_floor: None,
            joins: JoinMap::new(),
            pairings: Vec::new(),
            findings: Vec::new(),
        }
    }

    fn bind(&mut self, name: &str, is_handle: bool) {
        let id = if is_handle {
            self.next_id += 1;
            self.next_id - 1
        } else {
            0
        };
        self.scope.push((name.to_string(), id));
    }

    /// Bind every variable of `pattern` as a non-handle: tracking stops at a
    /// pattern (D4).
    fn bind_shadows(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Var(ident) => self.bind(&ident.name, false),
            Pattern::Tuple(items, _) | Pattern::Constructor(_, items, _) => {
                for item in items {
                    self.bind_shadows(item);
                }
            }
            Pattern::Or(left, right, _) => {
                self.bind_shadows(left);
                self.bind_shadows(right);
            }
            Pattern::Wildcard(_) | Pattern::Literal(_) | Pattern::Error(_) => {}
        }
    }

    fn handle_id(&self, name: &str) -> usize {
        self.scope
            .iter()
            .rev()
            .find(|(bound, _)| bound == name)
            .map_or(0, |(_, id)| *id)
    }

    /// Run `body` and drop the binders it introduced.
    fn scoped(&mut self, body: impl FnOnce(&mut Self)) {
        let depth = self.scope.len();
        body(self);
        self.scope.truncate(depth);
    }

    fn report(&mut self, kind: HandleFindingKind, handle: &str, span: Span) {
        self.findings.push(HandleFinding {
            kind,
            handle: handle.to_string(),
            span,
        });
    }

    /// A use of `name` at `span`; `consuming` is false for an observer's.
    fn use_variable(&mut self, name: &str, span: Span, consuming: bool) {
        if is_synthesised(name) {
            self.record_pairing(name);
            return;
        }
        let id = self.handle_id(name);
        if id == 0 {
            return;
        }
        if id < self.closure_floor {
            self.report(HandleFindingKind::CapturedByClosure, name, span);
        } else if self.consumed.contains(&id) {
            self.report(HandleFindingKind::UsedAfterConsumed, name, span);
        } else if consuming || !self.policy.observers_free {
            self.consumed.insert(id);
        }
    }

    /// Run each arm from the same state and join them (D4: arms take their
    /// maximum). An arm that ends its path does not join.
    fn arms<'f>(&mut self, arms: Vec<ArmWalk<'f, Self>>) {
        let start = self.consumed.clone();
        let start_live = self.live;
        let mut joined: Option<BTreeSet<usize>> = None;
        for arm in arms {
            if self.policy.arms == ArmPolicy::Max {
                self.consumed.clone_from(&start);
            }
            self.live = start_live;
            arm(self);
            if self.live {
                joined
                    .get_or_insert_with(BTreeSet::new)
                    .extend(self.consumed.iter().copied());
            }
        }
        match joined {
            Some(consumed) => {
                self.consumed = consumed;
                self.live = true;
            }
            None => {
                self.consumed = start;
                self.live = false;
            }
        }
    }

    fn exprs<'e>(&mut self, exprs: impl IntoIterator<Item = &'e Expr>) {
        for expr in exprs {
            self.expr(expr);
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Path(path) => {
                self.region_reference(path);
                if let Some(name) = single_segment(path) {
                    self.use_variable(name, path.span, true);
                }
            }
            Expr::Paren(inner, _)
            | Expr::Annot(inner, _, _)
            | Expr::TypeApp(inner, _, _)
            | Expr::Try(inner, _)
            | Expr::TryBlock(inner, _)
            | Expr::Unary(_, inner, _)
            | Expr::Field(inner, _, _)
            | Expr::Show(_, inner, _)
            | Expr::Sym(inner, _) => self.expr(inner),
            Expr::Region(body, _) => self.region(body),
            Expr::Binary(left, _, right, _)
            | Expr::Trans(left, right, _)
            | Expr::Cong(left, right, _) => self.exprs([&**left, &**right]),
            Expr::App(callee, args, _) => self.call(callee, args),
            Expr::Tuple(items, _) => self.exprs(items),
            Expr::RecordLit { spread, fields, .. } | Expr::NamedRecord { spread, fields, .. } => {
                self.exprs(spread.as_deref());
                self.exprs(fields.iter().map(|(_, value)| value));
            }
            Expr::Lambda(params, body, _) => self.closure(params.iter().map(|p| &p.pattern), body),
            Expr::Return(value, _) => {
                self.exprs(value.as_deref());
                self.live = false;
            }
            Expr::Let(pattern, _, definition, body, _) => {
                self.expr(definition);
                self.scoped(|walk| {
                    walk.bind_let(pattern, definition);
                    walk.expr(body);
                });
            }
            Expr::LetElse(pattern, _, definition, otherwise, body, _) => {
                self.let_else(definition, otherwise);
                self.scoped(|walk| {
                    walk.bind_shadows(pattern);
                    walk.expr(body);
                });
            }
            Expr::Block(stmts, tail, span) => {
                self.scoped(|walk| walk.block(stmts, tail.as_deref(), *span));
            }
            Expr::If(condition, then, otherwise, _) => {
                self.expr(condition);
                self.arms(vec![
                    Box::new(|walk| walk.expr(then)),
                    Box::new(|walk| walk.expr(otherwise)),
                ]);
            }
            Expr::Match(scrutinee, arms, _) => {
                self.expr(scrutinee);
                self.arms(arms.iter().map(|arm| Self::match_arm(arm)).collect());
            }
            Expr::IfLet(pattern, scrutinee, then, otherwise, _) => {
                self.expr(scrutinee);
                self.arms(vec![
                    Box::new(|walk| {
                        walk.scoped(|walk| {
                            walk.bind_shadows(pattern);
                            walk.expr(then);
                        });
                    }),
                    Box::new(|walk| walk.exprs(otherwise.as_deref())),
                ]);
            }
            Expr::IfLetChain(conditions, then, otherwise, _) => {
                self.if_let_chain(conditions, then, otherwise.as_deref());
            }
            Expr::Have(name, _, proof, body, _) => {
                self.expr(proof);
                self.scoped(|walk| {
                    walk.bind(&name.name, false);
                    walk.expr(body);
                });
            }
            Expr::Assume(name, _, body, _) => self.scoped(|walk| {
                walk.bind(&name.name, false);
                walk.expr(body);
            }),
            Expr::Subst(proof, motive, witness, _) => {
                self.expr(proof);
                self.motive(motive);
                self.expr(witness);
            }
            Expr::NatInd(motive, base, step, n, _) => {
                self.motive(motive);
                self.exprs([&**base, &**step, &**n]);
            }
            Expr::NatRec(_, base, step, n, _) => self.exprs([&**base, &**step, &**n]),
            Expr::IntLiteral(..)
            | Expr::BoolLiteral(..)
            | Expr::StringLiteral(..)
            | Expr::Unit(_)
            | Expr::Refl(_)
            | Expr::Sorry(_)
            | Expr::Error(_) => {}
        }
    }

    fn motive(&mut self, motive: &Motive) {
        if let Motive::Expr(inner) = motive {
            self.expr(inner);
        }
    }

    /// A call: the callee, then each argument left to right (P0.6). A bare
    /// variable passed to an observer is read, not consumed.
    fn call(&mut self, callee: &Expr, args: &[Expr]) {
        self.expr(callee);
        let observer = callee_name(callee).is_some_and(|name| {
            self.declarations.kind_of(name) == Some(HandleFunctionKind::Observer)
        });
        for arg in args {
            match bare_variable(arg) {
                Some((name, span)) if observer => self.use_variable(name, span, false),
                _ => self.expr(arg),
            }
        }
    }

    /// A closure's body, where every handle bound outside it is off limits.
    fn closure<'p>(&mut self, params: impl Iterator<Item = &'p Pattern>, body: &Expr) {
        let (consumed, live, floor) = (self.consumed.clone(), self.live, self.closure_floor);
        self.closure_floor = self.next_id;
        self.scoped(|walk| {
            params.for_each(|param| walk.bind_shadows(param));
            walk.expr(body);
        });
        (self.consumed, self.live, self.closure_floor) = (consumed, live, floor);
    }

    fn let_else(&mut self, definition: &Expr, otherwise: &Expr) {
        self.expr(definition);
        // The `else` diverges: it is walked for its own uses, and its path
        // never rejoins.
        let (consumed, live) = (self.consumed.clone(), self.live);
        self.expr(otherwise);
        (self.consumed, self.live) = (consumed, live);
    }

    fn match_arm<'m>(arm: &'m MatchArm) -> Box<dyn FnOnce(&mut Self) + 'm> {
        Box::new(move |walk: &mut Self| {
            walk.scoped(|walk| {
                walk.bind_shadows(&arm.pattern);
                walk.exprs(arm.guard.as_ref());
                walk.expr(&arm.body);
            });
        })
    }

    fn if_let_chain(
        &mut self,
        conditions: &[IfLetCondition],
        then: &Expr,
        otherwise: Option<&Expr>,
    ) {
        let depth = self.scope.len();
        for condition in conditions {
            match condition {
                IfLetCondition::Bind(pattern, scrutinee) => {
                    self.expr(scrutinee);
                    self.bind_shadows(pattern);
                }
                IfLetCondition::Guard(guard) => self.expr(guard),
            }
        }
        let bound = self.scope.split_off(depth);
        self.arms(vec![
            Box::new(move |walk: &mut Self| {
                walk.scoped(|walk| {
                    walk.scope.extend(bound);
                    walk.expr(then);
                });
            }),
            Box::new(|walk: &mut Self| walk.exprs(otherwise)),
        ]);
    }
}
