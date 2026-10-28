//! Tactic blocks: `theorem … -> T by { tactic; … }` (ADR 2.10.26c).
//!
//! A tactic does not prove anything; it builds Core. `exact` and `refl` hand
//! their expression to the elaborator, which checks it as it checks any term
//! (D4). `intro` and `apply` assemble a `λ` and an application themselves, so
//! the assembled whole has exactly one complete check: the kernel's, recorded
//! as an obligation at the end of the definition and discharged at the
//! admission boundary (`elaborate::kernel`, D5).
//!
//! - `state.rs`: goals, metavariables and syntactic grafting (D2)
//! - `matching.rs`: `apply`'s first-order matcher (D3)
//! - `steps.rs`: one function per tactic (D3, D4)
//! - `rewrite/`: `rewrite [r₁, …, rₙ]`, one `Subst` per rule (ADR 2.10.26e)
//! - `induction.rs`: `induction x`, a two-hole `natind` (ADR 3.10.26e)
//! - `cases/`: `cases x { … }`, an annotated split with a hole per arm
//!   (ADR 3.10.26f), and its pair arm over an `exists` (ADR 6.10.26e D6)
//! - `steps.rs` also holds `exists w`, an annotated pair with one hole
//!   (ADR 6.10.26e D4)
//! - `disjunction.rs`: `left` and `right`, an `Inl`/`Inr` with one hole
//!   (ADR 7.10.26l D2); `cases/` splits a disjunction by `inl`/`inr` arms
//! - `simp/`: `simp [r₁, …, rₙ]`, `rewrite` steps repeated under a bound,
//!   then `refl` (ADR 4.10.26e)
//! - `procedures/`: decision procedures — `ring`, semiring normalisation by
//!   `nat_arith` citations, then `refl` (ADR 4.10.26g), and `omega`, a
//!   witness solved from the hypotheses, then `ring` (ADR 6.10.26f)

mod cases;
mod disjunction;
mod induction;
pub(crate) mod matching;
mod procedures;
mod rewrite;
pub(crate) mod simp;
pub(crate) mod state;
mod steps;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use albite_core::types::Sort;
use albite_core::{Term, Type};

use crate::ast::{Tactic, TacticBlock};
use crate::elaborate::error::{ElabError, ElabErrorKind, TacticErrorKind, NO_GOAL_LEFT};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};

use state::{MetaId, ProofState, UnsolvedMeta};

/// One tactic as it ran: what it was, where, and which goal it solved.
#[derive(Debug, Clone, PartialEq)]
pub struct TacticStep {
    pub(crate) tactic: &'static str,
    pub(crate) span: Span,
    pub(crate) goal: MetaId,
    /// An `exact` whose term contains `sorry`: the kernel boundary abstracts
    /// its whole contribution as a hypothesis (D8's Holes rule).
    pub(crate) exact_sorry: bool,
    /// The lemma instance a `ring` step rewrote by, `nat_arith::add_comm(n,
    /// m)`: a kernel refusal names it beside the tactic (ADR 4.10.26g D4).
    pub(crate) citation: Option<String>,
}

impl TacticStep {
    /// The name a kernel refusal attributes this step to: the tactic, and
    /// its citation when it has one.
    pub(crate) fn culprit_name(&self) -> String {
        match &self.citation {
            Some(citation) => format!("{} ({citation})", self.tactic),
            None => self.tactic.to_string(),
        }
    }
}

/// Everything the kernel boundary needs to re-assemble, abstract and
/// attribute a tactic proof: the solved state, the steps in order, and the
/// theorem's parameters, which the elaborator's scope held while it ran.
#[derive(Debug, Clone, PartialEq)]
pub struct TacticTrace {
    pub(crate) state: ProofState,
    pub(crate) steps: Vec<TacticStep>,
    pub(crate) value_params: Vec<(String, Type)>,
    pub(crate) type_params: Vec<String>,
    pub(crate) block_span: Span,
}

impl TacticTrace {
    /// The definition's term: the root goal zonked with `overrides`, under a
    /// `λ` per value parameter and a `Λ` per type parameter — exactly how
    /// `elaborate_theorem` wraps the body.
    ///
    /// # Errors
    ///
    /// A meta left unsolved, which a finished block cannot have.
    pub(crate) fn assemble(
        &self,
        overrides: &BTreeMap<MetaId, Term>,
    ) -> Result<Term, UnsolvedMeta> {
        let mut term = self.state.zonk_with(0, overrides)?;
        for (name, ty) in self.value_params.iter().rev() {
            term = Term::lambda(name, ty.clone(), term);
        }
        for name in self.type_params.iter().rev() {
            term = Term::TyAbs(name.clone(), Box::new(term));
        }
        Ok(term)
    }
}

/// The span of each step `tactic` records in the trace: one per rule of a
/// `rewrite`, as if each were written alone (ADR 2.10.26e D6), else one.
fn step_spans(tactic: &Tactic) -> Vec<Span> {
    match tactic {
        Tactic::Rewrite(rules, _) => rules.iter().map(|rule| rule.span).collect(),
        _ => vec![tactic.span()],
    }
}

/// Whether `ty` may become a tactic goal: it sorts `Prop` (ADR 6.10.26e D2,
/// 4.10.26i D1), so ∧, ∨, ∃ and a family over them do, beside `Eq`.
///
/// This is not the erasure key. Codegen erases a proof by the narrower
/// [`Type::is_proposition`], and replacing one with the other there is a
/// miscompile (D7), so this predicate has no caller outside `tactics/`.
pub(crate) fn is_tactic_proposition(ty: &Type) -> bool {
    ty.sort_of() == Some(Sort::Prop)
}

/// E0082 for `tactic` against `goal`.
pub(crate) fn not_applicable(tactic: &Tactic, goal: &Type, why: impl Into<String>) -> ElabError {
    ElabError::new(
        tactic.span(),
        ElabErrorKind::Tactic(TacticErrorKind::TacticNotApplicable {
            tactic: tactic.name().to_string(),
            goal: goal.clone(),
            why: why.into(),
        }),
    )
}

impl Elaborator<'_> {
    /// Run `tactics` in order, each on the focused goal, recording one step
    /// per run. A `cases` step is recorded before its arms' steps, so a kernel
    /// refusal walks the arms first (D8's minimal refused step).
    fn run_tactics(
        &mut self,
        tactics: &[Tactic],
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        for tactic in tactics {
            if let Tactic::Simp(rules, _) = tactic {
                self.run_simp(tactic, rules, state, steps)?;
                continue;
            }
            if let Tactic::Ring(_) = tactic {
                self.run_ring(tactic, state, steps)?;
                continue;
            }
            if let Tactic::Omega(_) = tactic {
                self.run_omega(tactic, state, steps)?;
                continue;
            }
            for (index, span) in step_spans(tactic).into_iter().enumerate() {
                let Some(goal) = state.focus() else {
                    return Err(not_applicable(tactic, &Type::Unit, NO_GOAL_LEFT));
                };
                let partial = self.run_tactic(tactic, index, state, goal)?;
                let exact_sorry =
                    matches!(tactic, Tactic::Exact(..)) && partial.template.contains_sorry();
                state.refine(goal, partial);
                steps.push(TacticStep {
                    tactic: tactic.name(),
                    span,
                    goal,
                    exact_sorry,
                    citation: None,
                });
                if let Tactic::Cases(_, arms, _) = tactic {
                    self.run_cases_arms(arms, goal, state, steps)?;
                }
                if let Tactic::CasesPair(_, _, _, body, span) = tactic {
                    self.run_split_arms(&[(body.as_slice(), *span)], goal, state, steps)?;
                }
            }
        }
        Ok(())
    }

    /// Run `block` against `expected`, under the theorem's parameters already
    /// in scope. Returns the zonked proof term and the trace; the caller fills
    /// in the parameters and records the kernel obligation (D5).
    pub(crate) fn elaborate_tactic_block(
        &mut self,
        block: &TacticBlock,
        expected: &Type,
    ) -> ElabResult<(Term, TacticTrace)> {
        let mut state = ProofState::new(expected.clone());
        let mut steps = Vec::with_capacity(block.tactics.len());
        self.run_tactics(&block.tactics, &mut state, &mut steps)?;
        if !state.goals.is_empty() {
            let goals = state
                .goals
                .iter()
                .map(|goal| {
                    let meta = &state.metas[*goal];
                    (meta.locals.clone(), meta.target.clone())
                })
                .collect();
            return Err(ElabError::new(
                block.span,
                ElabErrorKind::Tactic(TacticErrorKind::UnsolvedGoals { goals }),
            ));
        }
        let term = state.zonk(0).map_err(|UnsolvedMeta(meta)| {
            ElabError::new(
                block.span,
                ElabErrorKind::InternalError(format!(
                    "a finished `by` block left meta {meta} unsolved"
                )),
            )
        })?;
        Ok((
            term,
            TacticTrace {
                state,
                steps,
                value_params: Vec::new(),
                type_params: Vec::new(),
                block_span: block.span,
            },
        ))
    }
}
