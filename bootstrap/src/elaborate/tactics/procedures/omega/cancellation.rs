//! Each σ equation's proof (ADR 6.10.26f D5): a hypothesis's own, or a
//! cancellation's conclusion — which is built, not found.
//!
//! S2's back end and `ring`'s emitter both rewrite only the goal, so a
//! residual `e : L == R` is carried to the cancellation's premise by
//! `trans(ring(A + C == σL), trans(Subst(σ-eqs, …, e), ring(σR == B + C)))`,
//! each `ring(…)` a proof of an auxiliary goal, and handed to
//! `nat_arith::add_right_cancel`. A zero atom of `A == 0` is one more `ring`
//! reordering and `nat_arith::add_eq_zero_left`.

use albite_core::{Term, Type};

use crate::elaborate::exprs::combinators::synthesised_motive;
use crate::elaborate::tactics::procedures::ring::normal_form::display_term;
use crate::elaborate::tactics::rewrite::abstraction::{
    goal_endpoints, motive_and_new_goal, occurs,
};
use crate::elaborate::tactics::rewrite::Equation;
use crate::elaborate::{ElabError, ElabResult, Elaborator};

use super::fragment::Hypothesis;
use super::solve::{Cancellation, OmegaPlan, Oriented, Source, ZeroAtom};
use super::{OmegaRun, SigmaEquation};

impl Elaborator<'_> {
    /// Each σ entry's equation, in σ's order: a hypothesis's own, or a
    /// cancellation's, built under the entries before it.
    pub(super) fn sigma_equations(
        &mut self,
        run: &mut OmegaRun<'_>,
        plan: &OmegaPlan,
        hypotheses: &[Hypothesis],
    ) -> ElabResult<Vec<SigmaEquation>> {
        let mut equations: Vec<SigmaEquation> = Vec::with_capacity(plan.sigma.len());
        let mut conclusions: Vec<Option<Term>> = vec![None; plan.cancellations.len()];
        for entry in &plan.sigma {
            let index = match entry.source {
                Source::Hypothesis(index) => {
                    equations.push(hypothesis_equation(&hypotheses[index]));
                    continue;
                }
                Source::Cancellation(index)
                | Source::ZeroAtom {
                    cancellation: index,
                    ..
                } => index,
            };
            let cancellation = &plan.cancellations[index];
            let conclusion = match conclusions[index].clone() {
                Some(conclusion) => conclusion,
                None => {
                    let residual = &hypotheses[cancellation.residual];
                    let prior = (&plan.sigma[..cancellation.sigma_len], &equations[..]);
                    let built = self.cancel(run, cancellation, residual, prior)?;
                    conclusions[index] = Some(built.clone());
                    built
                }
            };
            let equation = match entry.source {
                Source::ZeroAtom { index, .. } => {
                    let zero = &cancellation.zero_atoms[index];
                    self.zero_atom_equation(run, &cancellation.left_rest, zero, conclusion)?
                }
                _ => cancelled_equation(cancellation, conclusion),
            };
            equations.push(equation);
        }
        Ok(equations)
    }

    /// `left_rest == right_rest` for `cancellation`, from its residual
    /// carried forward under σ's first entries, two `ring` proofs and
    /// `add_right_cancel`.
    fn cancel(
        &mut self,
        run: &mut OmegaRun<'_>,
        cancellation: &Cancellation,
        residual: &Hypothesis,
        (sigma, equations): (&[Oriented], &[SigmaEquation]),
    ) -> ElabResult<Term> {
        let start = hypothesis_equation(residual).equation;
        let (carried, left, right) = self.carry_forward(run, start, sigma, equations)?;
        let (a, b, c) = (
            &cancellation.left_rest,
            &cancellation.right_rest,
            &cancellation.common,
        );
        let a_plus_c = Term::nat_add(a.clone(), c.clone());
        let b_plus_c = Term::nat_add(b.clone(), c.clone());
        let to_left = self.prove_by_ring(run, &a_plus_c, &left)?;
        let to_right = self.prove_by_ring(run, &right, &b_plus_c)?;
        let premise = trans(&a_plus_c, to_left, trans(&left, carried, to_right));
        let (head, _) = self.cite(run, "add_right_cancel")?;
        let applied = [a, b, c]
            .into_iter()
            .fold(head, |applied, arg| Term::app(applied, arg.clone()));
        Ok(Term::app(applied, premise))
    }

    /// One zero atom's `atom == 0`, from the cancellation's `A == 0`: the
    /// conclusion itself when `A` is the atom alone, else a `ring`
    /// reordering to `atom + rest` and `add_eq_zero_left`.
    fn zero_atom_equation(
        &mut self,
        run: &mut OmegaRun<'_>,
        sum: &Term,
        zero: &ZeroAtom,
        conclusion: Term,
    ) -> ElabResult<SigmaEquation> {
        let Some(rest) = &zero.rest else {
            return Ok(nat_equation(
                conclusion,
                &zero.atom,
                &Term::Zero,
                display_term(sum),
            ));
        };
        let reordered = Term::nat_add(zero.atom.clone(), rest.clone());
        let to_sum = self.prove_by_ring(run, &reordered, sum)?;
        let premise = trans(&reordered, to_sum, conclusion);
        let (head, _) = self.cite(run, "add_eq_zero_left")?;
        let proof = Term::app(
            Term::app(Term::app(head, zero.atom.clone()), rest.clone()),
            premise,
        );
        let citation = format!(
            "nat_arith::add_eq_zero_left({}, {})",
            display_term(&zero.atom),
            display_term(rest)
        );
        Ok(nat_equation(proof, &zero.atom, &Term::Zero, citation))
    }

    /// `start`, an equation, rewritten forward by each σ entry whose side it
    /// mentions, until none is left: one `Subst` per rewrite, transporting
    /// along `var == value` (D5's forward `Subst`). Returns the proof and its
    /// new sides.
    fn carry_forward(
        &self,
        run: &OmegaRun<'_>,
        start: Equation,
        sigma: &[Oriented],
        equations: &[SigmaEquation],
    ) -> ElabResult<(Term, Term, Term)> {
        let (mut proof, mut left, mut right) = (start.proof, start.left, start.right);
        for _ in 0..=sigma.len() {
            let mut changed = false;
            for (entry, emitted) in sigma.iter().zip(equations) {
                if !occurs(&entry.var, &left) && !occurs(&entry.var, &right) {
                    continue;
                }
                let endpoints = (&Type::Nat, &left, &right);
                let (motive, carried) =
                    motive_and_new_goal(endpoints, &Type::Nat, &entry.var, &entry.value);
                self.validate_synthesised_motive(&motive, &Type::Nat, run.span())?;
                let along = self.along(run, entry, &emitted.equation)?;
                proof = Term::subst(Type::Nat, motive, along, proof);
                let Some((_, new_left, new_right)) = goal_endpoints(&carried) else {
                    let why = "a carried equation is an equation";
                    return Err(ElabError::internal(run.span(), why));
                };
                (left, right) = (new_left.clone(), new_right.clone());
                changed = true;
            }
            if !changed {
                break;
            }
        }
        Ok((proof, left, right))
    }

    /// A proof of `var == value` for `entry`: its equation, or `sym` of it
    /// when the variable is on the right.
    fn along(&self, run: &OmegaRun<'_>, entry: &Oriented, equation: &Equation) -> ElabResult<Term> {
        if entry.var_on_left {
            return Ok(equation.proof.clone());
        }
        self.sym_term(
            &Type::Nat,
            &equation.left,
            equation.proof.clone(),
            run.span(),
        )
    }
}

/// A cancellation's own σ equation, `A == B`, citing `add_right_cancel`.
fn cancelled_equation(cancellation: &Cancellation, conclusion: Term) -> SigmaEquation {
    let (a, b, c) = (
        &cancellation.left_rest,
        &cancellation.right_rest,
        &cancellation.common,
    );
    let citation = format!(
        "nat_arith::add_right_cancel({}, {}, {})",
        display_term(a),
        display_term(b),
        display_term(c)
    );
    nat_equation(conclusion, a, b, citation)
}

/// A candidate's own equation: `snd(h)` for a `Le` shape, `h` for an `Eq`.
fn hypothesis_equation(hypothesis: &Hypothesis) -> SigmaEquation {
    let local = Term::var(hypothesis.name.clone());
    let proof = if hypothesis.witness.is_some() {
        Term::snd(local)
    } else {
        local
    };
    let citation = hypothesis.name.clone();
    nat_equation(proof, &hypothesis.left, &hypothesis.right, citation)
}

fn nat_equation(proof: Term, left: &Term, right: &Term, citation: String) -> SigmaEquation {
    SigmaEquation {
        equation: Equation {
            proof,
            base: Type::Nat,
            left: left.clone(),
            right: right.clone(),
        },
        citation,
    }
}

/// `trans(first, second)` over `Nat`, for `first : left == b` and
/// `second : b == c`: the `Subst` `infer_trans` builds.
fn trans(left: &Term, first: Term, second: Term) -> Term {
    let motive = synthesised_motive(&Type::Nat, |x| Type::eq(Type::Nat, left.clone(), x));
    Term::subst(Type::Nat, motive, second, first)
}
