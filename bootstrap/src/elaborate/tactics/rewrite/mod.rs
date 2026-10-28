//! `rewrite [r₁, …, rₙ]` (ADR 2.10.26e): replace one side of an equation by
//! the other in the focused goal, one rule at a time.
//!
//! Each rule becomes one motive-carrying `Subst` around a hole for the new
//! goal, plus a `sym` for a forward rule (D4) — shapes the kernel already
//! types, so the tactic adds no kernel rule (D6).
//!
//! The step has two halves (ADR 4.10.26g D4): [`Elaborator::run_rewrite_rule`]
//! elaborates a written rule into an [`Equation`], and
//! [`Elaborator::rewrite_by_equation`] rewrites the goal by any equation
//! already in Core — which is how `ring` cites the library.
//!
//! - `abstraction.rs`: the occurrence search and the motive, pure (D3, D4)

pub(crate) mod abstraction;
#[cfg(test)]
mod tests;

use albite_core::{Term, Type};

use crate::ast::{Expr, RewriteRule};
use crate::elaborate::error::{ElabError, ElabErrorKind, TacticErrorKind};
use crate::elaborate::exprs::combinators::extract_eq;
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::Span;

use super::matching::instantiate_term;
use super::state::{hole_name, MetaId, Partial, ProofState};
use abstraction::{first_match, goal_endpoints, motive_and_new_goal, occurs};

/// One rule as a tactic hands it to [`Elaborator::run_rewrite_rule`]: the
/// tactic's name, which its refusals carry (`rewrite`, or `simp` — ADR
/// 4.10.26e D8), the rule, and its 1-based position in its list.
#[derive(Clone, Copy)]
pub(super) struct RuleStep<'a> {
    pub(super) tactic: &'static str,
    pub(super) rule: &'a RewriteRule,
    pub(super) position: usize,
}

/// Where a rewrite step stands, for its refusals: the tactic, the step's
/// 1-based position, the span a refusal names, and whether it runs right to
/// left (`<-`).
#[derive(Clone, Copy)]
pub(super) struct EquationSite {
    pub(super) tactic: &'static str,
    pub(super) position: usize,
    pub(super) span: Span,
    pub(super) reverse: bool,
}

impl RuleStep<'_> {
    fn site(&self) -> EquationSite {
        EquationSite {
            tactic: self.tactic,
            position: self.position,
            span: self.rule.span,
            reverse: self.rule.reverse,
        }
    }
}

/// One step at work: where it stands, and the focused goal `Eq σ left right`
/// it rewrites.
struct RuleSite<'a> {
    at: EquationSite,
    target: &'a Type,
    left: &'a Term,
    right: &'a Term,
}

impl RuleSite<'_> {
    /// E0082, at the rule, naming its position.
    fn not_applicable(&self, why: impl std::fmt::Display) -> ElabError {
        rule_not_applicable(self.at, self.target, why)
    }

    /// E0085: `pattern`, the side `equation` was to rewrite, does not occur.
    fn no_occurrence(
        &self,
        equation: &Equation,
        pattern: &Term,
        other_side_occurs: bool,
    ) -> ElabError {
        ElabError::new(
            self.at.span,
            ElabErrorKind::Tactic(TacticErrorKind::RewriteNoOccurrence {
                position: self.at.position,
                rule: Type::eq(
                    equation.base.clone(),
                    equation.left.clone(),
                    equation.right.clone(),
                ),
                pattern: pattern.clone(),
                goal: self.target.clone(),
                reversed: self.at.reverse,
                other_side_occurs,
            }),
        )
    }

    /// Whether `side` occurs syntactically in the goal, outside binders.
    fn occurs(&self, side: &Term) -> bool {
        occurs(side, self.left) || occurs(side, self.right)
    }
}

/// A rule elaborated to an equation: its proof term and its type
/// `Eq σ a b`, instantiated when the rule was a bare lemma.
#[derive(Debug, Clone)]
pub(super) struct Equation {
    pub(super) proof: Term,
    pub(super) base: Type,
    pub(super) left: Term,
    pub(super) right: Term,
}

impl Equation {
    /// The side rewritten and the side it becomes: `a` to `b`, or with `<-`
    /// `b` to `a`.
    fn direction(&self, reverse: bool) -> (&Term, &Term) {
        if reverse {
            (&self.right, &self.left)
        } else {
            (&self.left, &self.right)
        }
    }
}

impl Elaborator<'_> {
    /// Rewrite the focused goal `goal` by the step's rule: elaborate it to an
    /// equation, then [`Self::rewrite_by_equation`]. A refusal leaves `state`
    /// untouched: the new goal's meta is the last thing allocated.
    pub(super) fn run_rewrite_rule(
        &mut self,
        step: RuleStep<'_>,
        state: &mut ProofState,
        goal: MetaId,
    ) -> ElabResult<Partial> {
        let rule = step.rule;
        let locals = state.metas[goal].locals.clone();
        let target = state.metas[goal].target.clone();
        let Some((_, left, right)) = goal_endpoints(&target) else {
            return Err(rule_not_applicable(
                step.site(),
                &target,
                "the goal is not an equation",
            ));
        };
        let site = RuleSite {
            at: step.site(),
            target: &target,
            left,
            right,
        };
        let (term, ty) = self.with_goal_locals(&locals, |elab| elab.infer(&rule.rule))?;
        let equation = match (&rule.rule, &ty) {
            (Expr::Path(_), Type::Pi(..) | Type::Arrow(..) | Type::Forall(..)) => {
                instantiate_bare_lemma(&site, term, &ty)?
            }
            _ => {
                let (base, left, right) = extract_eq(&ty, rule.span)?;
                Equation {
                    proof: term,
                    base,
                    left,
                    right,
                }
            }
        };
        self.rewrite_by_equation(state, goal, equation, step.site())
    }

    /// Rewrite the focused goal `goal` by `equation`, already in Core: refuse
    /// a step that rewrites nothing, abstract every occurrence of the side
    /// rewritten, and emit the `Subst` around a fresh hole for the new goal
    /// (ADR 2.10.26e D3, D4). A refusal leaves `state` untouched.
    pub(super) fn rewrite_by_equation(
        &mut self,
        state: &mut ProofState,
        goal: MetaId,
        equation: Equation,
        at: EquationSite,
    ) -> ElabResult<Partial> {
        let locals = state.metas[goal].locals.clone();
        let target = state.metas[goal].target.clone();
        let Some(endpoints) = goal_endpoints(&target) else {
            return Err(rule_not_applicable(
                at,
                &target,
                "the goal is not an equation",
            ));
        };
        let (_, left, right) = endpoints;
        let site = RuleSite {
            at,
            target: &target,
            left,
            right,
        };
        let (from, to) = equation.direction(at.reverse);
        if equation.left.strip_spans() == equation.right.strip_spans() || !site.occurs(from) {
            return Err(site.no_occurrence(&equation, from, site.occurs(to)));
        }
        let (motive, new_goal) = motive_and_new_goal(endpoints, &equation.base, from, to);
        self.validate_synthesised_motive(&motive, &equation.base, at.span)?;
        // Forward, the kernel transports along `sym(e) : Eq σ b a` from the new
        // goal `P[b]` back to `P[a]`; reversed, along `e` itself (D4's table).
        let eq_proof = if at.reverse {
            equation.proof
        } else {
            self.sym_term(&equation.base, &equation.left, equation.proof, at.span)?
        };
        let hole = state.fresh_meta(locals, new_goal);
        Ok(Partial {
            template: Term::subst(equation.base, motive, eq_proof, Term::var(hole_name(hole))),
            holes: vec![(hole_name(hole), hole)],
        })
    }
}

/// E0082 for the step at `at` against `target` (D7).
fn rule_not_applicable(at: EquationSite, target: &Type, why: impl std::fmt::Display) -> ElabError {
    ElabError::new(
        at.span,
        ElabErrorKind::Tactic(TacticErrorKind::TacticNotApplicable {
            tactic: at.tactic.to_string(),
            goal: target.clone(),
            why: format!("rule {}: {why}", at.position),
        }),
    )
}

/// D2(b): a bare lemma name whose type is a `Π` chain ending in an equation.
/// Each `Π` variable is fixed by the first match of the rule's chosen side in
/// the goal (D3); the rule is then the lemma applied to the matched terms.
fn instantiate_bare_lemma(site: &RuleSite, head: Term, lemma_ty: &Type) -> ElabResult<Equation> {
    if matches!(lemma_ty, Type::Forall(..)) {
        return Err(
            site.not_applicable("a generic lemma cannot be rewritten by; pass explicit arguments")
        );
    }
    let mut vars = Vec::new();
    let mut conclusion = lemma_ty;
    loop {
        match conclusion {
            Type::Pi(var, _, body) => {
                vars.push(var.clone());
                conclusion = body;
            }
            // A non-dependent binder or a hypothesis: matching the pattern
            // can never fix it (D3's undetermined argument).
            Type::Arrow(premise, _) => {
                return Err(site.not_applicable(format!(
                    "cannot infer an argument of type `{premise}`; pass explicit arguments"
                )))
            }
            _ => break,
        }
    }
    let (base, left, right) = extract_eq(conclusion, site.at.span)?;
    let lemma = Equation {
        proof: head,
        base,
        left,
        right,
    };
    let (pattern, other) = lemma.direction(site.at.reverse);
    if let Term::Var(var) = pattern.strip_spans() {
        if vars.contains(&var) {
            return Err(site.not_applicable(format!(
                "the pattern is the bare argument `{var}`, which matches anything; \
                 pass explicit arguments"
            )));
        }
    }
    let free = pattern.free_vars();
    if let Some(var) = vars.iter().find(|var| !free.contains(*var)) {
        return Err(site.not_applicable(format!(
            "cannot infer `{var}` from the pattern `{}`; pass explicit arguments",
            pattern.strip_spans()
        )));
    }
    let Some(bindings) = first_match(pattern, &vars, site.left, site.right) else {
        let other_side_occurs = first_match(other, &vars, site.left, site.right).is_some();
        return Err(site.no_occurrence(&lemma, pattern, other_side_occurs));
    };
    let proof = vars.iter().fold(lemma.proof, |applied, var| {
        Term::app(applied, bindings[var].clone())
    });
    Ok(Equation {
        proof,
        base: lemma.base,
        left: instantiate_term(&lemma.left, &bindings),
        right: instantiate_term(&lemma.right, &bindings),
    })
}
