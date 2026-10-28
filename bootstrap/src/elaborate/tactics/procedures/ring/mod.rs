//! `ring` (ADR 4.10.26g): prove an `Eq<Nat, a, b>` goal that holds in every
//! commutative semiring, by rewriting both sides into one normal form.
//!
//! Each step cites a `nat_arith` lemma with all its arguments and goes
//! through S2's own back end, [`Elaborator::rewrite_by_equation`]; S1's `refl`
//! closes the result. So the proof is S2's `Subst`s and S1's `refl`, and the
//! tactic adds no kernel rule (D4). The trace records one `ring` step per
//! citation, naming it, so a kernel refusal names the lemma too.
//!
//! - `normal_form.rs`: atoms, polynomials and D2's spelling, pure
//! - `plan.rs`: the steps that reach the normal form, pure (D3)
//! - `power.rs`: `pow` at a numeral exponent unfolded first, pure (9.10.26j D4)
//! - `lemma.rs`: the lemmas cited, and each one's library

pub(crate) mod lemma;
pub(crate) mod normal_form;
pub(crate) mod plan;
pub(crate) mod power;
#[cfg(test)]
mod tests;

use albite_core::{Term, Type};

use crate::ast::{Expr, Ident, Path, Tactic};
use crate::elaborate::error::{ElabError, ElabErrorKind, TacticErrorKind, NO_GOAL_LEFT};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};

use super::super::not_applicable;
use super::super::rewrite::abstraction::goal_endpoints;
use super::super::rewrite::{Equation, EquationSite};
use super::super::state::{MetaId, ProofState};
use super::super::TacticStep;
use lemma::Lemma;
use normal_form::{polynomial, render, AtomOrder};
use plan::{plan, PlanFault, PlannedStep, RING_STEP_CAP};
use power::{unfold_powers, unfolded_head, Unfolded, POW_UNFOLD_CAP};

/// The name every step and refusal of this tactic carries.
const RING: &str = "ring";

/// D4's refusal when a citation does not resolve, naming the step's library.
pub(crate) fn ring_needs_the_library(library: &str) -> String {
    format!("`ring` cites the `{library}` library; add `mod {library};`")
}

/// D1's refusal for an equation over another type.
pub(crate) const RING_NEEDS_NAT: &str = "`ring` proves equations over `Nat` only";

impl Elaborator<'_> {
    /// Run `ring` on the focused goal, recording its steps (D1–D4).
    pub(in crate::elaborate::tactics) fn run_ring(
        &mut self,
        tactic: &Tactic,
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let Some(goal) = state.focus() else {
            return Err(not_applicable(tactic, &Type::Unit, NO_GOAL_LEFT));
        };
        let target = state.metas[goal].target.clone();
        let Some((base, left, right)) = goal_endpoints(&target) else {
            return Err(not_applicable(
                tactic,
                &target,
                "the goal is not an equation",
            ));
        };
        if *base != Type::Nat {
            return Err(not_applicable(tactic, &target, RING_NEEDS_NAT));
        }
        // D4's order: the cap, the unfolding, then E0087 and the plan on the
        // UNFOLDED sides, so the plan's atoms are read from what it rewrites.
        let locals = state.metas[goal].locals.clone();
        let unfolded = match self.library_pow_head(&locals, tactic.span()) {
            Some(head) => unfold_powers(left, right, &head, POW_UNFOLD_CAP)
                .map_err(|past| not_applicable(tactic, &target, past.describe()))?,
            None => Unfolded {
                steps: Vec::new(),
                left: left.clone(),
                right: right.clone(),
                largest_exponent: None,
            },
        };
        refuse_unequal(tactic.span(), &unfolded.left, &unfolded.right)?;
        let remaining = RING_STEP_CAP.saturating_sub(unfolded.steps.len());
        let planned = plan(&unfolded.left, &unfolded.right, remaining).map_err(|fault| {
            plan_fault_error(tactic, &target, &fault, unfolded.largest_exponent)
        })?;
        for step in unfolded.steps.iter().chain(&planned.steps) {
            let goal = state.focus().unwrap_or(goal);
            let locals = state.metas[goal].locals.clone();
            let (head, lemma_ty) = self
                .with_goal_locals(&locals, |elab| {
                    elab.infer(&library_path(step.lemma, tactic.span()))
                })
                .map_err(|_| {
                    let library = step.lemma.library();
                    not_applicable(tactic, &target, ring_needs_the_library(library))
                })?;
            let Some(equation) = equation_for(step, head, &lemma_ty) else {
                return Err(ElabError::internal(
                    tactic.span(),
                    format!(
                        "`ring` cited `{}`, whose type `{lemma_ty}` is not the lemma's",
                        step.citation()
                    ),
                ));
            };
            self.apply_ring_step(state, step, equation, tactic.span(), steps)?;
        }
        self.close_ring(tactic.span(), state, steps)
    }

    /// The head `nat_pow::pow_succ` unfolds, when that lemma resolves in the
    /// goal's scope; otherwise no `pow` is the library's (D4, P0.3).
    fn library_pow_head(&mut self, locals: &[(String, Type)], span: Span) -> Option<String> {
        let path = library_path(Lemma::PowSucc, span);
        let (_, pow_succ_type) = self
            .with_goal_locals(locals, |elab| elab.infer(&path))
            .ok()?;
        unfolded_head(&pow_succ_type)
    }

    /// One planned step through S2's back end, recorded with its citation.
    pub(in crate::elaborate::tactics) fn apply_ring_step(
        &mut self,
        state: &mut ProofState,
        step: &PlannedStep,
        equation: Equation,
        span: Span,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let Some(goal) = state.focus() else {
            return Err(ElabError::internal(span, "`ring` lost its goal mid-plan"));
        };
        let at = EquationSite {
            tactic: RING,
            position: steps.len() + 1,
            span,
            reverse: false,
        };
        let partial = self.rewrite_by_equation(state, goal, equation, at)?;
        state.refine(goal, partial);
        steps.push(ring_step(span, goal, Some(step.citation())));
        Ok(())
    }

    /// S1's `refl` on the normalised goal. The polynomials were equal, so a
    /// refusal is the planner's fault (E9998), never the user's.
    fn close_ring(
        &mut self,
        span: Span,
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let Some(goal) = state.focus() else {
            return Err(ElabError::internal(
                span,
                "`ring` lost its goal before `refl`",
            ));
        };
        let locals = state.metas[goal].locals.clone();
        let target = state.metas[goal].target.clone();
        let partial = self.run_refl(span, &locals, &target).map_err(|refusal| {
            ElabError::internal(
                span,
                format!(
                    "`ring` normalised the goal to `{target}`, which `refl` refuses: {}",
                    refusal.kind.default_message()
                ),
            )
        })?;
        state.refine(goal, partial);
        steps.push(ring_step(span, goal, None));
        Ok(())
    }
}

/// D6: E0087 when the two sides are different polynomials over their atoms.
fn refuse_unequal(span: Span, left: &Term, right: &Term) -> ElabResult<()> {
    let mut atoms = AtomOrder::default();
    atoms.extend_from(left);
    atoms.extend_from(right);
    let (Some(left_p), Some(right_p)) = (polynomial(left, &atoms), polynomial(right, &atoms))
    else {
        return Err(ElabError::internal(
            span,
            "`ring` read an atom it had not indexed",
        ));
    };
    if left_p == right_p {
        return Ok(());
    }
    Err(ElabError::new(
        span,
        ElabErrorKind::Tactic(TacticErrorKind::RingUnequal {
            left: render(&left_p, &atoms),
            right: render(&right_p, &atoms),
        }),
    ))
}

/// A planner fault as reported: E0082 naming the exponent when a `pow` was
/// unfolded and the cap was reached (D4), else the planner's own E9998.
fn plan_fault_error(
    tactic: &Tactic,
    target: &Type,
    fault: &PlanFault,
    largest_exponent: Option<u64>,
) -> ElabError {
    match (fault, largest_exponent) {
        (PlanFault::CapReached { cap, .. }, Some(exponent)) => not_applicable(
            tactic,
            target,
            format!(
                "`ring` reached its cap of {cap} steps expanding `pow` at exponent \
                 {exponent}; cite `pow_succ` by hand"
            ),
        ),
        _ => ElabError::internal(tactic.span(), fault.describe()),
    }
}

/// `<library>::<lemma>`, as a path the elaborator resolves like any other.
fn library_path(lemma: Lemma, span: Span) -> Expr {
    Expr::Path(Path {
        segments: vec![
            Ident::new(lemma.library(), span),
            Ident::new(lemma.name(), span),
        ],
        span,
    })
}

/// The step's equation: the lemma applied to its arguments, with the sides
/// as the goal spells them. `None` when `lemma_ty` is not a `Π` per argument
/// ending in an equation. Only the kernel compares the two spellings, by
/// conversion (P0.2).
pub(crate) fn equation_for(step: &PlannedStep, head: Term, lemma_ty: &Type) -> Option<Equation> {
    let mut conclusion = lemma_ty.clone();
    let mut proof = head;
    for arg in &step.args {
        let Type::Pi(var, _, body) = conclusion else {
            return None;
        };
        conclusion = body.substitute_term(&var, arg);
        proof = Term::app(proof, arg.clone());
    }
    let (base, _, _) = goal_endpoints(&conclusion)?;
    Some(Equation {
        proof,
        base: base.clone(),
        left: step.left.clone(),
        right: step.right.clone(),
    })
}

/// One trace step of `ring`, at `span`, solving `goal`.
fn ring_step(span: Span, goal: MetaId, citation: Option<String>) -> TacticStep {
    TacticStep {
        tactic: RING,
        span,
        goal,
        exact_sorry: false,
        citation,
    }
}
