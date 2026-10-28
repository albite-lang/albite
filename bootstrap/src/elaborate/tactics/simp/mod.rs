//! `simp [r₁, …, rₙ]` (ADR 4.10.26e): S2's rewrite step, repeated under a
//! bound, then S1's `refl`.
//!
//! Each round applies the first rule in list order that occurs in the goal,
//! through `rewrite`'s own [`Elaborator::run_rewrite_rule`]; the next round
//! starts again from the first rule. When no rule occurs, `refl` closes the
//! goal if it can (D2). Every step is `rewrite`'s `Subst` and the close is
//! `refl`'s term, so the tactic adds no kernel rule (D4); the trace records
//! one step per applied rule, at that rule's span, and one for the `refl`.
//!
//! - `bound.rs`: the rounds under `SIMP_STEP_BOUND` and the permutative
//!   test, pure (D3)

pub(crate) mod bound;
#[cfg(test)]
mod tests;

use albite_core::Type;

use crate::ast::{Expr, RewriteRule, Tactic};
use crate::elaborate::error::{ElabError, ElabErrorKind, TacticErrorKind, NO_GOAL_LEFT};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::Spanned;

use super::not_applicable;
use super::rewrite::RuleStep;
use super::state::{MetaId, ProofState};
use super::TacticStep;
use bound::{is_permutative, run_rounds, RoundsOutcome, SIMP_STEP_BOUND};

/// The name every step and refusal of this tactic carries.
const SIMP: &str = "simp";

/// D2's no-progress refusal.
pub(crate) const NO_RULE_OCCURS: &str = "no rule in the list occurs in the goal";

/// Whether a rewrite step's refusal means "this rule does not apply here",
/// which `simp` skips (D2): no occurrence, or both sides the same term.
fn is_skip(error: &ElabError) -> bool {
    matches!(
        error.kind,
        ElabErrorKind::Tactic(TacticErrorKind::RewriteNoOccurrence { .. })
    )
}

/// `a::b::c` for a path rule, as D3's refusal names it.
fn rule_name(rule: &RewriteRule) -> Option<String> {
    let Expr::Path(path) = &rule.rule else {
        return None;
    };
    let segments: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    Some(segments.join("::"))
}

impl Elaborator<'_> {
    /// Run `simp` on the focused goal, recording its steps (D2, D4).
    pub(super) fn run_simp(
        &mut self,
        tactic: &Tactic,
        rules: &[RewriteRule],
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let Some(start) = state.focus() else {
            return Err(not_applicable(tactic, &Type::Unit, NO_GOAL_LEFT));
        };
        for rule in rules {
            self.refuse_permutative(tactic, rule, state, start)?;
        }
        let outcome = run_rounds(SIMP_STEP_BOUND, || self.simp_round(rules, state, steps))?;
        let goal = state.focus().unwrap_or(start);
        let target = state.metas[goal].target.clone();
        match outcome {
            RoundsOutcome::Stable { steps: 0 } => {
                Err(not_applicable(tactic, &target, NO_RULE_OCCURS))
            }
            RoundsOutcome::BoundReached { last_rules } => Err(ElabError::new(
                tactic.span(),
                ElabErrorKind::Tactic(TacticErrorKind::SimpStepBound {
                    bound: SIMP_STEP_BOUND,
                    last_rules,
                    goal: target,
                }),
            )),
            RoundsOutcome::Stable { .. } => {
                self.simp_close(tactic, state, steps, goal);
                Ok(())
            }
        }
    }

    /// D2's close: `refl` on the simplified goal. Closed now or deferred, the
    /// goal is solved and the `refl` is a trace step at the `simp`; refused,
    /// the goal stays focused and the refusal is dropped. `check_refl` fails
    /// only with E0070 (not an equation) or E0071 (unequal sides), both the
    /// goal's, so every refusal is dropped and none is told apart.
    fn simp_close(
        &mut self,
        tactic: &Tactic,
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
        goal: MetaId,
    ) {
        let locals = state.metas[goal].locals.clone();
        let target = state.metas[goal].target.clone();
        if let Ok(partial) = self.run_refl(tactic.span(), &locals, &target) {
            state.refine(goal, partial);
            steps.push(simp_step(tactic.span(), goal));
        }
    }

    /// One round (D2): apply the first rule that occurs in the focused goal
    /// and return its 1-based position, or `None` when none does.
    fn simp_round(
        &mut self,
        rules: &[RewriteRule],
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<Option<usize>> {
        let Some(goal) = state.focus() else {
            return Ok(None);
        };
        for (index, rule) in rules.iter().enumerate() {
            let step = RuleStep {
                tactic: SIMP,
                rule,
                position: index + 1,
            };
            match self.run_rewrite_rule(step, state, goal) {
                Ok(partial) => {
                    state.refine(goal, partial);
                    steps.push(simp_step(rule.span, goal));
                    return Ok(Some(index + 1));
                }
                Err(error) if is_skip(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    /// D3: a bare lemma whose sides only rename its variables is refused
    /// before any step — it would rewrite forever.
    fn refuse_permutative(
        &mut self,
        tactic: &Tactic,
        rule: &RewriteRule,
        state: &ProofState,
        goal: MetaId,
    ) -> ElabResult<()> {
        let Some(name) = rule_name(rule) else {
            return Ok(());
        };
        let locals = state.metas[goal].locals.clone();
        let (_, mut lemma_ty) = self.with_goal_locals(&locals, |elab| elab.infer(&rule.rule))?;
        let mut vars = Vec::new();
        while let Type::Pi(var, _, body) = lemma_ty {
            vars.push(var);
            lemma_ty = *body;
        }
        match &lemma_ty {
            Type::Eq(_, left, right) if is_permutative(&vars, left, right) => {
                let target = &state.metas[goal].target;
                Err(not_applicable(
                    tactic,
                    target,
                    format!(
                        "`{name}` swaps its arguments and would rewrite forever; \
                         give its arguments, or use `rewrite`"
                    ),
                ))
            }
            _ => Ok(()),
        }
    }
}

/// One trace step of `simp`, at `span`, solving `goal`.
fn simp_step(span: crate::span::Span, goal: MetaId) -> TacticStep {
    TacticStep {
        tactic: SIMP,
        span,
        goal,
        exact_sorry: false,
        citation: None,
    }
}
