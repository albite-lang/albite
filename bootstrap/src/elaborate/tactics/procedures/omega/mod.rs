//! `omega` (ADR 6.10.26f): prove a `Le`-shaped or `Eq<Nat, …>` goal from
//! the `Le` and `Eq` hypotheses in scope, by witness synthesis plus `ring`
//! (decision_procedures.md Q4).
//!
//! The solver plans; the emitter builds Core from steps the kernel already
//! types, so the tactic adds no kernel rule (D5):
//!
//! - a `Le` goal is closed by `Annot(Pair(w, ?g), Σ…)`, 6.10.26e's term;
//! - `?g` is rewritten by S2's own back end along each σ equation until no
//!   eliminated side is left, then closed by `ring`'s planner and `refl`.
//!
//! Each rewrite and citation is one `omega` step in the trace, naming its
//! hypothesis or lemma, so a kernel refusal names it too.
//!
//! - `fragment.rs`: the shapes `omega` reads, pure (D2)
//! - `solve.rs`: orientation, the witness and the cancellation round, pure
//!   (D3), over `polynomial.rs`'s signed differences
//! - `cancellation.rs`: each σ equation's proof, a cancellation's built from
//!   its residual by forward `Subst`s, `trans` and `ring` (D5)

mod cancellation;
pub(crate) mod fragment;
pub(crate) mod polynomial;
pub(crate) mod solve;
#[cfg(test)]
mod tests;

use albite_core::{Term, Type};

use crate::ast::{Expr, Ident, Path, Tactic};
use crate::elaborate::error::{
    ElabError, ElabErrorKind, OmegaFailure, TacticErrorKind, NO_GOAL_LEFT,
};
use crate::elaborate::tactics::not_applicable;
use crate::elaborate::tactics::rewrite::abstraction::{goal_endpoints, occurs};
use crate::elaborate::tactics::rewrite::{Equation, EquationSite};
use crate::elaborate::tactics::state::{hole_name, MetaId, Partial, ProofState};
use crate::elaborate::tactics::TacticStep;
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};

use super::ring::equation_for;
use super::ring::normal_form::display_term;
use super::ring::plan::{plan, RING_STEP_CAP};
use fragment::{goal_shape, hypothesis, le_shape, outside_fragment, Hypothesis};
use solve::{solve, OmegaPlan};

/// The name every step and refusal of this tactic carries.
const OMEGA: &str = "omega";

/// D5's refusal when a citation does not resolve.
pub(crate) const OMEGA_NEEDS_THE_LIBRARY: &str =
    "`omega` cites the `nat_arith` library; add `mod nat_arith;`";

/// D2's refusal for a goal of neither shape.
pub(crate) const OMEGA_NEEDS_A_SHAPE: &str =
    "`omega` proves a `Le`-shaped goal (`exists k: Nat. s + k == t`) or an equation over `Nat`";

/// One run of `omega`: the tactic, for its refusals and span; the focused
/// goal's locals, under which citations resolve, and its target, which an
/// E0082 names; and the proof state and trace it extends.
struct OmegaRun<'a> {
    tactic: &'a Tactic,
    locals: Vec<(String, Type)>,
    target: Type,
    state: &'a mut ProofState,
    steps: &'a mut Vec<TacticStep>,
}

impl OmegaRun<'_> {
    fn span(&self) -> Span {
        self.tactic.span()
    }

    /// The focused goal, or E9998: `omega` never runs out of goals mid-plan.
    fn focused(&self) -> ElabResult<MetaId> {
        self.state
            .focus()
            .ok_or_else(|| ElabError::internal(self.span(), "`omega` lost its goal mid-plan"))
    }

    /// The focused goal's two sides, or E9998.
    fn focused_sides(&self) -> ElabResult<(MetaId, Term, Term)> {
        let goal = self.focused()?;
        let Some((_, left, right)) = goal_endpoints(&self.state.metas[goal].target) else {
            return Err(ElabError::internal(
                self.span(),
                "`omega` lost its equation",
            ));
        };
        Ok((goal, left.clone(), right.clone()))
    }

    /// Solve `goal` with `partial` and record the step.
    fn refine(&mut self, goal: MetaId, partial: Partial, citation: Option<String>) {
        self.state.refine(goal, partial);
        self.steps.push(TacticStep {
            tactic: OMEGA,
            span: self.span(),
            goal,
            exact_sorry: false,
            citation,
        });
    }

    /// Where the next rewrite stands, for S2's back end.
    fn site(&self, reverse: bool) -> EquationSite {
        EquationSite {
            tactic: OMEGA,
            position: self.steps.len() + 1,
            span: self.span(),
            reverse,
        }
    }
}

/// One σ equation as emitted: its proof and sides, and what the trace calls
/// it.
struct SigmaEquation {
    equation: Equation,
    citation: String,
}

impl Elaborator<'_> {
    /// Run `omega` on the focused goal, recording its steps (D1–D5).
    pub(in crate::elaborate::tactics) fn run_omega(
        &mut self,
        tactic: &Tactic,
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let Some(goal) = state.focus() else {
            return Err(not_applicable(tactic, &Type::Unit, NO_GOAL_LEFT));
        };
        let target = state.metas[goal].target.clone();
        let mut run = OmegaRun {
            tactic,
            locals: state.metas[goal].locals.clone(),
            target: target.clone(),
            state,
            steps,
        };
        let expanded = self.normalize_for_comparison(&target);
        // Poison passes through (ADR 15.8.26d): its fault is already reported.
        if expanded == Type::Error {
            run.refine(goal, Partial::closed(Term::Sorry), None);
            return Ok(());
        }
        let Some(shape) = goal_shape(&expanded) else {
            return Err(not_applicable(tactic, &target, OMEGA_NEEDS_A_SHAPE));
        };
        if let Some(term) = outside_fragment(&shape) {
            let term = display_term(&term);
            let failure = OmegaFailure::OutsideFragment { term };
            return Err(omega_error(run.span(), failure));
        }
        let hypotheses = self.omega_candidates(&run.locals);
        let plan =
            solve(&hypotheses, &shape).map_err(|failure| omega_error(run.span(), failure))?;
        let equations = self.sigma_equations(&mut run, &plan, &hypotheses)?;
        if let (Some((bound, _, _)), Type::Sigma(_, _, body)) = (le_shape(&expanded), &expanded) {
            let witness = plan.witness.clone().unwrap_or(Term::Zero);
            let hole = run
                .state
                .fresh_meta(run.locals.clone(), body.substitute_term(bound, &witness));
            let template = Term::annot(
                Term::pair(witness, Term::var(hole_name(hole))),
                expanded.clone(),
            );
            let holes = vec![(hole_name(hole), hole)];
            run.refine(goal, Partial { template, holes }, None);
        }
        self.rewrite_goal_by_sigma(&mut run, &plan, &equations)?;
        self.close_by_ring(&mut run)
    }

    /// Every local in scope of a `Le` or `Eq<Nat, …>` shape, in proof-state
    /// order: the theorem's parameters by binding level, then the goal's own
    /// locals. A goal local shadows a parameter of its name.
    fn omega_candidates(&self, locals: &[(String, Type)]) -> Vec<Hypothesis> {
        let mut parameters: Vec<_> = self.env.local_bindings().collect();
        parameters.sort_by_key(|binding| binding.level);
        let scope = parameters
            .into_iter()
            .filter(|binding| !locals.iter().any(|(name, _)| *name == binding.name))
            .map(|binding| (binding.name.clone(), binding.ty.clone()))
            .chain(locals.iter().cloned());
        scope
            .filter_map(|(name, ty)| hypothesis(&name, &self.normalize_for_comparison(&ty)))
            .collect()
    }

    /// Rewrite the focused goal by each σ entry whose side it mentions, until
    /// none is left, through S2's back end: one step each.
    fn rewrite_goal_by_sigma(
        &mut self,
        run: &mut OmegaRun<'_>,
        plan: &OmegaPlan,
        equations: &[SigmaEquation],
    ) -> ElabResult<()> {
        for _ in 0..=plan.sigma.len() {
            let mut changed = false;
            for (entry, emitted) in plan.sigma.iter().zip(equations) {
                let (goal, left, right) = run.focused_sides()?;
                if !occurs(&entry.var, &left) && !occurs(&entry.var, &right) {
                    continue;
                }
                let at = run.site(!entry.var_on_left);
                let equation = emitted.equation.clone();
                let partial = self.rewrite_by_equation(run.state, goal, equation, at)?;
                run.refine(goal, partial, Some(emitted.citation.clone()));
                changed = true;
            }
            if !changed {
                break;
            }
        }
        Ok(())
    }

    /// A proof of `left == right` by `ring`, on an auxiliary goal focused for
    /// the purpose and zonked once closed.
    fn prove_by_ring(
        &mut self,
        run: &mut OmegaRun<'_>,
        left: &Term,
        right: &Term,
    ) -> ElabResult<Term> {
        let target = Type::eq(Type::Nat, left.clone(), right.clone());
        let auxiliary = run.state.fresh_meta(run.locals.clone(), target);
        run.state.goals.insert(0, auxiliary);
        self.close_by_ring(run)?;
        run.state.zonk(auxiliary).map_err(|_| {
            ElabError::internal(run.span(), "`omega` left an auxiliary `ring` goal open")
        })
    }

    /// `ring`'s plan on the focused `Eq<Nat, …>` goal, one citation per step
    /// through S2's back end, then `refl`. The solver found the sides equal
    /// as polynomials, so a refusal here is the tactic's fault (E9998).
    fn close_by_ring(&mut self, run: &mut OmegaRun<'_>) -> ElabResult<()> {
        let (_, left, right) = run.focused_sides()?;
        let planned = plan(&left, &right, RING_STEP_CAP)
            .map_err(|fault| ElabError::internal(run.span(), fault.describe()))?;
        for step in &planned.steps {
            let (head, lemma_ty) = self.cite(run, step.lemma.name())?;
            let Some(equation) = equation_for(step, head, &lemma_ty) else {
                let why = "`omega` cited a lemma of another shape";
                return Err(ElabError::internal(run.span(), why));
            };
            let goal = run.focused()?;
            let at = run.site(false);
            let partial = self.rewrite_by_equation(run.state, goal, equation, at)?;
            run.refine(goal, partial, Some(step.citation()));
        }
        let goal = run.focused()?;
        let target = run.state.metas[goal].target.clone();
        let partial = self
            .run_refl(run.span(), &run.locals, &target)
            .map_err(|refusal| {
                let why = refusal.kind.default_message();
                let message =
                    format!("`omega` reduced the goal to `{target}`, which `refl` refuses: {why}");
                ElabError::internal(run.span(), message)
            })?;
        run.refine(goal, partial, None);
        Ok(())
    }

    /// `nat_arith::<lemma>` and its type, resolved under the goal's locals;
    /// E0082 naming the library when it is not in scope (D5).
    fn cite(&mut self, run: &OmegaRun<'_>, lemma: &str) -> ElabResult<(Term, Type)> {
        let path = library_path(lemma, run.span());
        self.with_goal_locals(&run.locals, |elab| elab.infer(&path))
            .map_err(|_| not_applicable(run.tactic, &run.target, OMEGA_NEEDS_THE_LIBRARY))
    }
}

/// `nat_arith::<lemma>`, as a path the elaborator resolves like any other.
fn library_path(lemma: &str, span: Span) -> Expr {
    Expr::Path(Path {
        segments: vec![Ident::new("nat_arith", span), Ident::new(lemma, span)],
        span,
    })
}

/// E0088 at `span`.
pub(crate) fn omega_error(span: Span, failure: OmegaFailure) -> ElabError {
    ElabError::new(
        span,
        ElabErrorKind::Tactic(TacticErrorKind::OmegaUnproved(failure)),
    )
}
