//! `omega`'s solver (ADR 6.10.26f D3). Pure: Core terms in, a plan out.
//!
//! 1. **Unpack** — each candidate is an equation (`fragment.rs`).
//! 2. **Orient** — in order, an equation one of whose sides is a bare local
//!    `v`, not yet eliminated and absent from σ applied to the other side,
//!    records `v ↦ other` in σ; the occurs check keeps σ triangular. Any
//!    other equation is a residual.
//! 3. **Solve** — `N(σ t) − N(σ s)` over `ring`'s monomials, with integer
//!    coefficients: all non-negative is the witness of a `Le` goal, zero
//!    closes an `Eq` goal.
//! 4. **Cancel, once** — a residual whose two normal forms share a summand
//!    `C` yields `A == B` with `A + C`, `B + C` its sides. `B = 0` with `A`
//!    a sum of witness atoms sends each atom to `0`; otherwise `A == B` is
//!    oriented as in step 2. Then step 3 again.

use std::collections::BTreeMap;

use albite_core::Term;

use super::fragment::{Goal, Hypothesis};
use super::polynomial::{polynomial_term, Signed};
use crate::elaborate::error::OmegaFailure;
use crate::elaborate::tactics::procedures::ring::normal_form::{
    display_term, polynomial, AtomOrder, Polynomial,
};
use crate::elaborate::tactics::rewrite::abstraction::{abstract_occurrences, occurs};

/// What proves one σ equation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// Candidate `n`'s own equation.
    Hypothesis(usize),
    /// Cancellation `n`'s conclusion, `A == B`.
    Cancellation(usize),
    /// The `index`th zero atom of cancellation `cancellation`: `atom == 0`.
    ZeroAtom { cancellation: usize, index: usize },
}

/// One σ entry: `var ↦ value`, from an equation whose `var` side is the
/// left one when `var_on_left`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Oriented {
    pub(crate) var: Term,
    pub(crate) value: Term,
    pub(crate) var_on_left: bool,
    pub(crate) source: Source,
}

/// A witness atom sent to `0` by `a + rest = 0 → a = 0`; `rest` is `None`
/// when `A` is the atom alone.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ZeroAtom {
    pub(crate) atom: Term,
    pub(crate) rest: Option<Term>,
}

/// One cancellation of residual `residual`, read under the first
/// `sigma_len` σ entries: `left_rest + common == right_rest + common`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cancellation {
    pub(crate) residual: usize,
    pub(crate) sigma_len: usize,
    pub(crate) common: Term,
    pub(crate) left_rest: Term,
    pub(crate) right_rest: Term,
    pub(crate) zero_atoms: Vec<ZeroAtom>,
}

/// The plan: σ in order, the cancellations, a `Le` goal's witness, and the
/// candidates that neither oriented nor cancelled.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OmegaPlan {
    pub(crate) sigma: Vec<Oriented>,
    pub(crate) cancellations: Vec<Cancellation>,
    pub(crate) witness: Option<Term>,
    pub(crate) unused: Vec<usize>,
}

/// `σ` applied to `term`: every eliminated side replaced by its value,
/// until none is left. σ is triangular, so each pass shortens every chain
/// and `σ.len() + 1` passes reach the fixed point.
pub(crate) fn apply_sigma(term: &Term, sigma: &[Oriented]) -> Term {
    let mut out = term.strip_spans();
    for _ in 0..=sigma.len() {
        let mut changed = false;
        for entry in sigma {
            if occurs(&entry.var, &out) {
                out = abstract_occurrences(&out, &entry.var, &entry.value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    out
}

/// Step 2: `left == right` oriented, left side first, as `(var, value,
/// var_on_left)`, or `None` when it stays a residual.
pub(crate) fn orient(left: &Term, right: &Term, sigma: &[Oriented]) -> Option<(Term, Term, bool)> {
    [(left, right, true), (right, left, false)]
        .into_iter()
        .find_map(|(side, other, on_left)| {
            let side = side.strip_spans();
            let bare = matches!(side, Term::Var(_));
            let eliminated = sigma.iter().any(|entry| entry.var == side);
            (bare && !eliminated && !occurs(&side, &apply_sigma(other, sigma)))
                .then(|| (side, other.strip_spans(), on_left))
        })
}

/// `term` under σ as a polynomial, indexing any atom σ created.
fn normal(term: &Term, sigma: &[Oriented], atoms: &mut AtomOrder) -> Polynomial {
    let applied = apply_sigma(term, sigma);
    atoms.extend_from(&applied);
    polynomial(&applied, atoms).unwrap_or_default()
}

/// Step 3 on `goal` under σ: the witness of a `Le` goal (`None` for an
/// `Eq` goal that closes), or the failing difference and what it is of.
fn close(
    goal: &Goal,
    sigma: &[Oriented],
    atoms: &mut AtomOrder,
) -> Result<Option<Term>, (Signed, &'static str)> {
    let (left, right) = goal.sides();
    let (s, t) = (normal(left, sigma, atoms), normal(right, sigma, atoms));
    let difference = Signed::difference(&t, &s);
    let negative = difference.monomials.values().any(|c| *c < 0) || difference.constant < 0;
    match goal {
        Goal::Le { .. } if !negative => {
            let witness = Polynomial {
                monomials: difference
                    .monomials
                    .iter()
                    .map(|(m, c)| (m.clone(), c.unsigned_abs()))
                    .collect(),
                constant: difference.constant.unsigned_abs(),
            };
            Ok(Some(polynomial_term(&witness, atoms)))
        }
        Goal::Eq { .. } if difference.is_zero() => Ok(None),
        Goal::Eq { .. } if !negative => Err((difference.negated(), "s - t")),
        _ => Err((difference, "t - s")),
    }
}

/// The monomial-wise minimum of two normal forms: their common summand.
fn common_summand(left: &Polynomial, right: &Polynomial) -> Polynomial {
    Polynomial {
        monomials: left
            .monomials
            .iter()
            .filter_map(|(m, c)| right.monomials.get(m).map(|d| (m.clone(), (*c).min(*d))))
            .collect(),
        constant: left.constant.min(right.constant),
    }
}

/// `p − q`, for `q` a summand of `p`.
fn minus(p: &Polynomial, q: &Polynomial) -> Polynomial {
    let mut out = p.clone();
    for (monomial, coefficient) in &q.monomials {
        if let Some(entry) = out.monomials.get_mut(monomial) {
            *entry -= coefficient;
        }
    }
    out.monomials.retain(|_, coefficient| *coefficient != 0);
    out.constant -= q.constant;
    out
}

fn is_empty(p: &Polynomial) -> bool {
    p.monomials.is_empty() && p.constant == 0
}

/// The solver's working state across steps 2–4.
struct Solver<'a> {
    hypotheses: &'a [Hypothesis],
    atoms: AtomOrder,
    plan: OmegaPlan,
}

impl Solver<'_> {
    /// Each atom of `a` when every one is a witness atom with no other
    /// factor, or `None`.
    fn witness_atoms(&self, a: &Polynomial) -> Option<Vec<usize>> {
        let witnesses: Vec<Term> = self
            .hypotheses
            .iter()
            .filter_map(|h| h.witness.as_ref().map(Term::strip_spans))
            .collect();
        let mut out = Vec::new();
        for monomial in a.monomials.keys() {
            let [index] = monomial.as_slice() else {
                return None;
            };
            witnesses.contains(self.atoms.atom(*index)).then_some(())?;
            out.push(*index);
        }
        (a.constant == 0 && !out.is_empty()).then_some(out)
    }

    /// Step 4 on residual `residual`: whether it was used.
    fn cancel(&mut self, residual: usize) -> bool {
        let hypothesis = &self.hypotheses[residual];
        let sigma = &self.plan.sigma;
        let left = normal(&hypothesis.left, sigma, &mut self.atoms);
        let right = normal(&hypothesis.right, sigma, &mut self.atoms);
        let common = common_summand(&left, &right);
        if is_empty(&common) {
            return false;
        }
        let (a, b) = (minus(&left, &common), minus(&right, &common));
        let index = self.plan.cancellations.len();
        let mut cancellation = Cancellation {
            residual,
            sigma_len: self.plan.sigma.len(),
            common: polynomial_term(&common, &self.atoms),
            left_rest: polynomial_term(&a, &self.atoms),
            right_rest: polynomial_term(&b, &self.atoms),
            zero_atoms: Vec::new(),
        };
        if let Some(atoms) = is_empty(&b).then(|| self.witness_atoms(&a)).flatten() {
            cancellation.zero_atoms = self.zero_atoms(index, &a, &atoms);
        } else {
            let left_rest = &cancellation.left_rest;
            let Some((var, value, var_on_left)) =
                orient(left_rest, &cancellation.right_rest, &self.plan.sigma)
            else {
                return false;
            };
            self.plan.sigma.push(Oriented {
                var,
                value,
                var_on_left,
                source: Source::Cancellation(index),
            });
        }
        self.plan.cancellations.push(cancellation);
        true
    }

    /// The zero atoms of `a`, each with `a` less one copy of it, and their
    /// σ entries; an atom already sent to `0` is not sent twice.
    fn zero_atoms(
        &mut self,
        cancellation: usize,
        a: &Polynomial,
        atoms: &[usize],
    ) -> Vec<ZeroAtom> {
        let mut out = Vec::new();
        for atom_index in atoms {
            let atom = self.atoms.atom(*atom_index).clone();
            let one = Polynomial {
                monomials: BTreeMap::from([(vec![*atom_index], 1)]),
                constant: 0,
            };
            let rest = minus(a, &one);
            let rest = (!is_empty(&rest)).then(|| polynomial_term(&rest, &self.atoms));
            if !self.plan.sigma.iter().any(|entry| entry.var == atom) {
                self.plan.sigma.push(Oriented {
                    var: atom.clone(),
                    value: Term::Zero,
                    var_on_left: true,
                    source: Source::ZeroAtom {
                        cancellation,
                        index: out.len(),
                    },
                });
            }
            out.push(ZeroAtom { atom, rest });
        }
        out
    }
}

/// D3: the plan that proves `goal` from `hypotheses`, or D7's second or
/// third message.
pub(crate) fn solve(hypotheses: &[Hypothesis], goal: &Goal) -> Result<OmegaPlan, OmegaFailure> {
    let mut solver = Solver {
        hypotheses,
        atoms: AtomOrder::default(),
        plan: OmegaPlan {
            sigma: Vec::new(),
            cancellations: Vec::new(),
            witness: None,
            unused: Vec::new(),
        },
    };
    let mut residuals = Vec::new();
    for (index, hypothesis) in hypotheses.iter().enumerate() {
        match orient(&hypothesis.left, &hypothesis.right, &solver.plan.sigma) {
            Some((var, value, var_on_left)) => solver.plan.sigma.push(Oriented {
                var,
                value,
                var_on_left,
                source: Source::Hypothesis(index),
            }),
            None => residuals.push(index),
        }
    }
    let mut outcome = close(goal, &solver.plan.sigma, &mut solver.atoms);
    if outcome.is_err() {
        residuals.retain(|residual| !solver.cancel(*residual));
        outcome = close(goal, &solver.plan.sigma, &mut solver.atoms);
    }
    solver.plan.unused = residuals;
    match outcome {
        Ok(witness) => Ok(OmegaPlan {
            witness,
            ..solver.plan
        }),
        Err(_) if !solver.plan.unused.is_empty() => Err(OmegaFailure::Unoriented {
            unused: solver
                .plan
                .unused
                .iter()
                .map(|index| unused_entry(&hypotheses[*index]))
                .collect(),
        }),
        Err((difference, difference_of)) => Err(OmegaFailure::NoWitness {
            difference_of: difference_of.to_string(),
            difference: difference.render(&solver.atoms),
            negative: difference
                .first_negative(&solver.atoms)
                .unwrap_or_else(|| difference.render(&solver.atoms)),
        }),
    }
}

/// An unused candidate as D7's third message names it.
fn unused_entry(hypothesis: &Hypothesis) -> (String, String) {
    (
        hypothesis.name.clone(),
        format!(
            "{} == {}",
            display_term(&hypothesis.left),
            display_term(&hypothesis.right)
        ),
    )
}
