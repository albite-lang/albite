//! A theorem's body: a term, or a `by` block of tactics (ADR 2.10.26c D11).
//!
//! The tactic names are contextual, not keywords (D1): `intro`, `exact`,
//! `apply`, `rewrite` (ADR 2.10.26e D1), `induction` (ADR 3.10.26e D1),
//! `cases` (ADR 3.10.26f D1), `simp` (ADR 4.10.26e D1), `ring` (ADR
//! 4.10.26g D1), `left` and `right` (ADR 7.10.26l D2) are ordinary
//! identifiers everywhere outside a `by` block, and `refl` is the already-reserved `Refl` token,
//! as `exists` (ADR 6.10.26e D4) is the reserved `Exists` token.

use serde::{Deserialize, Serialize};

use super::{Expr, Ident};
use crate::span::{Span, Spanned};

/// What follows a theorem's statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TheoremBody {
    /// `{ term }` or `= term` — a proof written as one expression.
    Term(Expr),
    /// `by { tactic; … }` — a proof built step by step.
    Tactics(TacticBlock),
}

impl Spanned for TheoremBody {
    fn span(&self) -> Span {
        match self {
            TheoremBody::Term(expr) => expr.span(),
            TheoremBody::Tactics(block) => block.span,
        }
    }
}

impl TheoremBody {
    /// Every expression a user wrote in this body, in source order: the term
    /// itself, or each `exact` and `apply` argument, each `exists` witness,
    /// each `rewrite` and `simp` rule, and
    /// each `cases` scrutinee and its arms' expressions. The AST walkers visit
    /// these and nothing else (D11).
    #[must_use]
    pub fn expressions(&self) -> Vec<&Expr> {
        match self {
            TheoremBody::Term(expr) => vec![expr],
            TheoremBody::Tactics(block) => {
                let mut expressions = Vec::new();
                collect_tactic_expressions(&block.tactics, &mut expressions);
                expressions
            }
        }
    }
}

/// Every expression `tactics` holds, in source order, arms included.
fn collect_tactic_expressions<'a>(tactics: &'a [Tactic], expressions: &mut Vec<&'a Expr>) {
    for tactic in tactics {
        match tactic {
            Tactic::Exact(expr, _) | Tactic::Apply(expr, _) | Tactic::Exists(expr, _) => {
                expressions.push(expr);
            }
            Tactic::Rewrite(rules, _) | Tactic::Simp(rules, _) => {
                expressions.extend(rules.iter().map(|rule| &rule.rule));
            }
            Tactic::Cases(scrutinee, arms, _) => {
                expressions.push(scrutinee);
                for arm in arms {
                    collect_tactic_expressions(&arm.body, expressions);
                }
            }
            Tactic::CasesPair(scrutinee, _, _, body, _) => {
                expressions.push(scrutinee);
                collect_tactic_expressions(body, expressions);
            }
            Tactic::Intro(..)
            | Tactic::Refl(_)
            | Tactic::Induction(..)
            | Tactic::Ring(_)
            | Tactic::Omega(_)
            | Tactic::Left(_)
            | Tactic::Right(_) => {}
        }
    }
}

/// `by { tactic; … }`, with the span from `by` to the closing brace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TacticBlock {
    pub tactics: Vec<Tactic>,
    pub span: Span,
}

/// One step of a `by` block. Each carries the span of the whole step, which
/// is what a kernel refusal names (D8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Tactic {
    /// `intro x` — move an arrow's or a Π's domain into the context.
    Intro(Ident, Span),
    /// `exact e` — close the goal with an expression.
    Exact(Expr, Span),
    /// `apply e` — close the goal's conclusion with a lemma, leaving its
    /// premises as new goals.
    Apply(Expr, Span),
    /// `refl` — close an equality goal by reflexivity.
    Refl(Span),
    /// `rewrite [r₁, …, rₙ]` — replace one side of each rule's equation by the
    /// other in the goal, left to right (ADR 2.10.26e D1).
    Rewrite(Vec<RewriteRule>, Span),
    /// `induction x` — split the goal on the `Nat` local `x` into its zero
    /// case and its successor step (ADR 3.10.26e D1).
    Induction(Ident, Span),
    /// `cases x { C₁ => tac, C₂(a, b) => { tac; tac } }` — split the goal on
    /// the local `x`, one arm per constructor (ADR 3.10.26f D1). The
    /// scrutinee is an expression so that a compound one is refused by the
    /// elaborator, with E0082, rather than as a parse error.
    Cases(Box<Expr>, Vec<CasesArm>, Span),
    /// `simp [r₁, …, rₙ]` — rewrite by the first rule that occurs, again and
    /// again, under a step bound, then close by `refl` if it can (ADR
    /// 4.10.26e D1, D2). The list is explicit and non-empty: there is no
    /// global lemma database (tactic_framework.md Q5).
    Simp(Vec<RewriteRule>, Span),
    /// `ring` — normalise both sides of an `Eq<Nat, a, b>` goal as
    /// commutative-semiring polynomials, one library-lemma rewrite per step,
    /// then close by `refl` (ADR 4.10.26g D1). It takes no arguments.
    Ring(Span),
    /// `exists w` — give the witness of an `exists` goal; the body at `w`
    /// becomes the goal (ADR 6.10.26e D4).
    Exists(Expr, Span),
    /// `cases h { (k, e) => tactics }` — unpack the `exists` local `h` into
    /// its witness and its evidence, one arm (ADR 6.10.26e D6). `None` is a
    /// `_`.
    CasesPair(Box<Expr>, Option<Ident>, Option<Ident>, Vec<Tactic>, Span),
    /// `omega` — prove a `Le`-shaped or `Eq<Nat, …>` goal from the `Le` and
    /// `Eq` hypotheses in scope: solve for the witness, then close by `ring`
    /// (ADR 6.10.26f D1). It takes no arguments.
    Omega(Span),
    /// `left` — prove a disjunction `A + B` by its left side: `A` becomes the
    /// goal (ADR 7.10.26l D2). It takes no arguments.
    Left(Span),
    /// `right` — prove a disjunction `A + B` by its right side: `B` becomes
    /// the goal (ADR 7.10.26l D2). It takes no arguments.
    Right(Span),
}

/// One arm of `cases`: a constructor, one binder per declared field (`None`
/// for a `_`), and the tactics that close the arm's goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasesArm {
    pub ctor: Ident,
    pub binders: Vec<Option<Ident>>,
    pub body: Vec<Tactic>,
    /// From the constructor to the arm's end: what a refusal names.
    pub span: Span,
}

/// One rule of a `rewrite` or `simp` list: `e`, or `<- e` to rewrite right to left.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteRule {
    pub reverse: bool,
    pub rule: Expr,
    /// From the `<-` when there is one, to the rule's end: what a refusal names.
    pub span: Span,
}

impl Tactic {
    /// The tactic's name as written, for messages and the trace.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Tactic::Intro(..) => "intro",
            Tactic::Exact(..) => "exact",
            Tactic::Apply(..) => "apply",
            Tactic::Refl(_) => "refl",
            Tactic::Rewrite(..) => "rewrite",
            Tactic::Induction(..) => "induction",
            Tactic::Cases(..) | Tactic::CasesPair(..) => "cases",
            Tactic::Simp(..) => "simp",
            Tactic::Ring(_) => "ring",
            Tactic::Exists(..) => "exists",
            Tactic::Omega(_) => "omega",
            Tactic::Left(_) => "left",
            Tactic::Right(_) => "right",
        }
    }
}

impl Spanned for Tactic {
    fn span(&self) -> Span {
        match self {
            Tactic::Intro(_, span)
            | Tactic::Exact(_, span)
            | Tactic::Apply(_, span)
            | Tactic::Rewrite(_, span)
            | Tactic::Simp(_, span)
            | Tactic::Induction(_, span)
            | Tactic::Exists(_, span)
            | Tactic::Cases(_, _, span)
            | Tactic::CasesPair(_, _, _, _, span)
            | Tactic::Refl(span)
            | Tactic::Ring(span)
            | Tactic::Omega(span)
            | Tactic::Left(span)
            | Tactic::Right(span) => *span,
        }
    }
}
