//! `ring`'s `pow` arm (ADR 9.10.26j D4). Pure: functions over Core terms.
//!
//! A `pow(x, e)` with an open `e` is an ordinary atom, compared like any
//! other. One with a numeral `e` is unfolded before the plan, by citing
//! `nat_pow::pow_succ(x, e - 1)` down to `nat_pow::pow_zero(x)`, so
//! `pow(x + y, 2)` becomes `(1 * (x + y)) * (x + y)` and the plan's own
//! steps distribute it.
//!
//! Core spells every global by its bare name (P0.3), so the library's `pow`
//! is told apart by `nat_pow::pow_succ`'s type: the caller resolves that
//! lemma in the goal's scope and passes the head its type mentions
//! ([`unfolded_head`]). Where it does not resolve — inside nat_pow.tg itself,
//! or a file with its own `pow` and no `mod nat_pow;` — nothing unfolds.

use albite_core::{Term, Type};

use super::lemma::Lemma;
use super::normal_form::{display_term, numeral_value};
use super::plan::{predecessor, PlannedStep};
use crate::elaborate::tactics::rewrite::abstraction::{
    abstract_occurrences, binds, goal_endpoints,
};

/// The largest numeral exponent `ring` unfolds (P0.4): every base P0.4
/// measured — an atom, a binomial and a numeral — plans within half of
/// `RING_STEP_CAP` up to it. `ring` collects no like terms, so `x + 1` grows
/// fastest: 129 steps at exponent 4, 403 at 5.
pub(crate) const POW_UNFOLD_CAP: u64 = 4;

/// The unfolding: its steps, the two sides they reach, and the largest
/// exponent unfolded (`None` when nothing was).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unfolded {
    pub(crate) steps: Vec<PlannedStep>,
    pub(crate) left: Term,
    pub(crate) right: Term,
    pub(crate) largest_exponent: Option<u64>,
}

/// A numeral exponent past [`POW_UNFOLD_CAP`]: the `pow` and its exponent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PastCap {
    pub(crate) power: String,
    pub(crate) exponent: u64,
    pub(crate) cap: u64,
}

impl PastCap {
    /// The refusal, for E0082.
    pub(crate) fn describe(&self) -> String {
        format!(
            "`ring` unfolds `pow` up to exponent {}, and `{}` has exponent {}; cite `pow_succ` \
             by hand",
            self.cap, self.power, self.exponent
        )
    }
}

/// The head `nat_pow::pow_succ`'s type unfolds — `pow` in
/// `Π a n. pow(a, n + 1) == pow(a, n) * a` — or `None` for any other type.
pub(crate) fn unfolded_head(pow_succ_type: &Type) -> Option<String> {
    let Type::Pi(_, _, after_base) = pow_succ_type else {
        return None;
    };
    let Type::Pi(_, _, equation) = &**after_base else {
        return None;
    };
    let (_, left, _) = goal_endpoints(equation)?;
    let (head, _, _) = power_parts(left)?;
    Some(head.to_string())
}

/// `term` as `head(base, exponent)`, looking through spans.
fn power_parts(term: &Term) -> Option<(&str, &Term, &Term)> {
    let Term::App(applied, exponent) = unspanned(term) else {
        return None;
    };
    let Term::App(global, base) = unspanned(applied) else {
        return None;
    };
    let Term::Global(head) = unspanned(global) else {
        return None;
    };
    Some((head, base, exponent))
}

fn unspanned(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) => unspanned(inner),
        other => other,
    }
}

/// Unfold every `head(x, k)` with a numeral `k` on both sides, refusing an
/// exponent past `cap` before any step is planned. Each step rewrites every
/// occurrence, as S2 will; the first `pow` in post-order — a base's before
/// its host's, the left side's before the right's — goes first.
pub(crate) fn unfold_powers(
    left: &Term,
    right: &Term,
    head: &str,
    cap: u64,
) -> Result<Unfolded, PastCap> {
    let (mut left, mut right) = (left.strip_spans(), right.strip_spans());
    let mut largest_exponent = None;
    for side in [&left, &right] {
        if let Some((power, exponent)) = largest_power(side, head) {
            if exponent > cap {
                return Err(PastCap {
                    power,
                    exponent,
                    cap,
                });
            }
            largest_exponent = largest_exponent.max(Some(exponent));
        }
    }
    let mut steps = Vec::new();
    while let Some(step) = first_power(&left, head).or_else(|| first_power(&right, head)) {
        left = abstract_occurrences(&left, &step.left, &step.right);
        right = abstract_occurrences(&right, &step.left, &step.right);
        steps.push(step);
    }
    Ok(Unfolded {
        steps,
        left,
        right,
        largest_exponent,
    })
}

/// The unfolding step for `node`, if it is `head(x, k)` at a numeral `k`.
fn unfolding_step(node: &Term, head: &str) -> Option<PlannedStep> {
    let (name, base, exponent) = power_parts(node)?;
    if name != head {
        return None;
    }
    Some(match numeral_value(exponent)? {
        0 => PlannedStep::new(
            Lemma::PowZero,
            vec![base.clone()],
            node,
            Term::Succ(Box::new(Term::Zero)),
        ),
        _ => {
            let lower = predecessor(exponent);
            let power = Term::app(Term::app(Term::global(head), base.clone()), lower.clone());
            PlannedStep::new(
                Lemma::PowSucc,
                vec![base.clone(), lower],
                node,
                Term::nat_mul(power, base.clone()),
            )
        }
    })
}

/// The first unfoldable `pow` of `term` in post-order, outside binders —
/// where S2's rewrite cannot reach.
fn first_power(term: &Term, head: &str) -> Option<PlannedStep> {
    let term = unspanned(term);
    if binds(term) {
        return None;
    }
    let mut found = None;
    term.for_each_subterm(|child| {
        if found.is_none() {
            found = first_power(child, head);
        }
    });
    found.or_else(|| unfolding_step(term, head))
}

/// The unfoldable `pow` of `term` with the largest exponent, spelled
/// `pow(x, k)`, and that exponent, outside binders.
fn largest_power(term: &Term, head: &str) -> Option<(String, u64)> {
    let term = unspanned(term);
    if binds(term) {
        return None;
    }
    let here = power_parts(term)
        .filter(|(name, _, _)| *name == head)
        .and_then(|(_, base, exponent)| {
            let exponent = numeral_value(exponent)?;
            Some((
                format!("{head}({}, {exponent})", display_term(base)),
                exponent,
            ))
        });
    let mut largest = here;
    term.for_each_subterm(|child| {
        if let Some((power, exponent)) = largest_power(child, head) {
            if largest.as_ref().is_none_or(|(_, known)| exponent > *known) {
                largest = Some((power, exponent));
            }
        }
    });
    largest
}
