//! `ring`'s plan (ADR 4.10.26g D3). Pure: functions over Core terms.
//!
//! The plan is a sequence of library-lemma citations, each with **all** its
//! arguments, that rewrites both sides of the goal into D2's normal form.
//! Each step is the first redex, pre-order over the left side then the
//! right, of the earliest phase that has one:
//!
//! 1. numerals in a product — `x * k` expanded by `mul_succ_right`, ended by
//!    `mul_one_right`/`mul_zero_right`; `k * x` turned by `mul_comm`, or
//!    dropped by `mul_one_left`/`mul_zero_left`;
//! 2. zero summands — `add_zero_right`, `add_zero_left`;
//! 3. distribution — `mul_add`, `add_mul`;
//! 4. association to the right — `add_assoc`, `mul_assoc`;
//! 5. factors sorted in each monomial — `mul_left_comm`, `mul_comm`;
//! 6. monomials sorted in each sum — `add_left_comm`, `add_comm`.
//!
//! Because S2 rewrites **every** occurrence of a step's instance, inside
//! atoms too (2.10.26e D3), each step is simulated with S2's own
//! `abstract_occurrences`, the atoms are re-read from the result, and the next
//! step is planned from there. A revisited goal or [`RING_STEP_CAP`] steps is
//! a planner fault, never looped on.

use albite_core::Term;

pub(crate) use super::lemma::Lemma;
use super::normal_form::{display_term, numeral_value, shape, summand_key, AtomOrder, Shape};
use crate::elaborate::tactics::rewrite::abstraction::abstract_occurrences;

/// The most steps one `ring` plans (D3). Every corpus goal takes far fewer
/// (§6); reaching it is a planner fault.
pub(crate) const RING_STEP_CAP: usize = 512;

/// How many of the last steps a planner fault names.
pub(crate) const FAULT_STEPS_NAMED: usize = 4;

/// One step: a lemma, its arguments in its parameter order, and the
/// instance's two sides as the goal spells them. Only a numeral step's left
/// side differs from the lemma's, and by conversion alone: the goal's
/// `n * 2` for the lemma's `n * (1 + 1)` (P0.2).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedStep {
    pub(crate) lemma: Lemma,
    pub(crate) args: Vec<Term>,
    pub(crate) left: Term,
    pub(crate) right: Term,
}

impl PlannedStep {
    pub(crate) fn new(lemma: Lemma, args: Vec<Term>, left: &Term, right: Term) -> Self {
        PlannedStep {
            lemma,
            args,
            left: left.clone(),
            right,
        }
    }

    /// `nat_arith::add_comm(n, m)`: what a trace step and a fault name.
    pub(crate) fn citation(&self) -> String {
        let args: Vec<String> = self.args.iter().map(display_term).collect();
        format!(
            "{}::{}({})",
            self.lemma.library(),
            self.lemma.name(),
            args.join(", ")
        )
    }
}

/// A plan: its steps, and the two sides they reach.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    pub(crate) steps: Vec<PlannedStep>,
    pub(crate) left: Term,
    pub(crate) right: Term,
}

/// A planner fault (D3): a goal seen before, or the cap reached. Each names
/// the last steps planned, as citations, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanFault {
    Revisited { last: Vec<String> },
    CapReached { cap: usize, last: Vec<String> },
}

impl PlanFault {
    /// The fault as one sentence, for E9998.
    pub(crate) fn describe(&self) -> String {
        match self {
            PlanFault::Revisited { last } => format!(
                "`ring` planned a step back to a goal it had already reached; the last steps \
                 were {}",
                last.join(", ")
            ),
            PlanFault::CapReached { cap, last } => format!(
                "`ring` reached its cap of {cap} steps; the last steps were {}",
                last.join(", ")
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Numerals,
    Zeros,
    Distribute,
    Associate,
    SortFactors,
    SortSummands,
}

const PHASES: [Phase; 6] = [
    Phase::Numerals,
    Phase::Zeros,
    Phase::Distribute,
    Phase::Associate,
    Phase::SortFactors,
    Phase::SortSummands,
];

/// Plan the rewrite of `Eq Nat left right` into normal form, under `cap`.
pub(crate) fn plan(left: &Term, right: &Term, cap: usize) -> Result<Plan, PlanFault> {
    let (mut left, mut right) = (left.strip_spans(), right.strip_spans());
    let mut atoms = AtomOrder::default();
    let mut visited = vec![(left.clone(), right.clone())];
    let mut steps: Vec<PlannedStep> = Vec::new();
    loop {
        atoms.extend_from(&left);
        atoms.extend_from(&right);
        let Some(step) = next_step(&left, &right, &atoms) else {
            return Ok(Plan { steps, left, right });
        };
        if steps.len() == cap {
            return Err(PlanFault::CapReached {
                cap,
                last: last_citations(&steps),
            });
        }
        left = abstract_occurrences(&left, &step.left, &step.right);
        right = abstract_occurrences(&right, &step.left, &step.right);
        steps.push(step);
        if visited.contains(&(left.clone(), right.clone())) {
            return Err(PlanFault::Revisited {
                last: last_citations(&steps),
            });
        }
        visited.push((left.clone(), right.clone()));
    }
}

fn last_citations(steps: &[PlannedStep]) -> Vec<String> {
    let skip = steps.len().saturating_sub(FAULT_STEPS_NAMED);
    steps[skip..].iter().map(PlannedStep::citation).collect()
}

/// The earliest phase's first non-trivial redex, left side then right.
fn next_step(left: &Term, right: &Term, atoms: &AtomOrder) -> Option<PlannedStep> {
    PHASES.iter().find_map(|phase| {
        first_redex(left, *phase, atoms).or_else(|| first_redex(right, *phase, atoms))
    })
}

/// The first redex of `phase` in `term`, pre-order, outside atoms. A step
/// whose two sides are equal would rewrite nothing (S2's E0085), so it is
/// never planned.
fn first_redex(term: &Term, phase: Phase, atoms: &AtomOrder) -> Option<PlannedStep> {
    let (Shape::Add(a, b) | Shape::Mul(a, b)) = shape(term) else {
        return None;
    };
    redex_at(term, phase, atoms)
        .filter(|step| step.left != step.right)
        .or_else(|| first_redex(a, phase, atoms))
        .or_else(|| first_redex(b, phase, atoms))
}

fn add(a: &Term, b: &Term) -> Term {
    Term::nat_add(a.clone(), b.clone())
}

fn mul(a: &Term, b: &Term) -> Term {
    Term::nat_mul(a.clone(), b.clone())
}

/// `phase`'s step at `node`, a sum or a product, if it has one.
fn redex_at(node: &Term, phase: Phase, atoms: &AtomOrder) -> Option<PlannedStep> {
    match (phase, shape(node)) {
        (Phase::Numerals, Shape::Mul(x, k)) => numeral_step(node, x, k),
        (Phase::Zeros, Shape::Add(a, b)) => zero_step(node, a, b),
        (Phase::Distribute, Shape::Mul(x, y)) => distribute_step(node, x, y),
        (Phase::Associate, Shape::Add(ab, c) | Shape::Mul(ab, c)) => associate_step(node, ab, c),
        (Phase::SortFactors, Shape::Mul(a, b)) => sort_step(node, a, b, atoms, SortKind::Factors),
        (Phase::SortSummands, Shape::Add(a, b)) => sort_step(node, a, b, atoms, SortKind::Summands),
        _ => None,
    }
}

/// Phase 4 at `node = ab ∘ c`: a left-nested sum or product regrouped to
/// the right. The operator must be the same at both levels: `(a * b) + c`
/// is already right-nested as a sum.
fn associate_step(node: &Term, ab: &Term, c: &Term) -> Option<PlannedStep> {
    let (lemma, combine): (Lemma, fn(&Term, &Term) -> Term) = match (shape(node), shape(ab)) {
        (Shape::Add(..), Shape::Add(..)) => (Lemma::AddAssoc, add),
        (Shape::Mul(..), Shape::Mul(..)) => (Lemma::MulAssoc, mul),
        _ => return None,
    };
    let (Shape::Add(a, b) | Shape::Mul(a, b)) = shape(ab) else {
        return None;
    };
    Some(PlannedStep::new(
        lemma,
        vec![a.clone(), b.clone(), c.clone()],
        node,
        combine(a, &combine(b, c)),
    ))
}

/// Phase 1 at `node = x * k`: expand a numeral on the right, or move one on
/// the left to the right (conversion leaves `2 * n` stuck).
fn numeral_step(node: &Term, x: &Term, k: &Term) -> Option<PlannedStep> {
    if let Some(value) = numeral_value(k) {
        return Some(match value {
            0 => PlannedStep::new(Lemma::MulZeroRight, vec![x.clone()], node, Term::Zero),
            1 => PlannedStep::new(Lemma::MulOneRight, vec![x.clone()], node, x.clone()),
            _ => {
                let j = predecessor(k);
                let right = add(&mul(x, &j), x);
                PlannedStep::new(Lemma::MulSuccRight, vec![x.clone(), j], node, right)
            }
        });
    }
    Some(match numeral_value(x)? {
        0 => PlannedStep::new(Lemma::MulZeroLeft, vec![k.clone()], node, Term::Zero),
        1 => PlannedStep::new(Lemma::MulOneLeft, vec![k.clone()], node, k.clone()),
        _ => PlannedStep::new(Lemma::MulComm, vec![x.clone(), k.clone()], node, mul(k, x)),
    })
}

/// A positive numeral's predecessor, spelled as the numeral was.
pub(crate) fn predecessor(numeral: &Term) -> Term {
    match numeral {
        Term::Spanned(inner, _) => predecessor(inner),
        Term::Succ(inner) => (**inner).clone(),
        Term::NatLit(value) => Term::NatLit(value.saturating_sub(1)),
        other => other.clone(),
    }
}

/// Phase 2 at `node = a + b`: a zero summand dropped.
fn zero_step(node: &Term, a: &Term, b: &Term) -> Option<PlannedStep> {
    if numeral_value(b) == Some(0) {
        Some(PlannedStep::new(
            Lemma::AddZeroRight,
            vec![a.clone()],
            node,
            a.clone(),
        ))
    } else if numeral_value(a) == Some(0) {
        Some(PlannedStep::new(
            Lemma::AddZeroLeft,
            vec![b.clone()],
            node,
            b.clone(),
        ))
    } else {
        None
    }
}

/// Phase 3 at `node = x * y`: a product over a sum distributed.
fn distribute_step(node: &Term, x: &Term, y: &Term) -> Option<PlannedStep> {
    if let Shape::Add(a, b) = shape(y) {
        let right = add(&mul(x, a), &mul(x, b));
        return Some(PlannedStep::new(
            Lemma::MulAdd,
            vec![x.clone(), a.clone(), b.clone()],
            node,
            right,
        ));
    }
    let Shape::Add(a, b) = shape(x) else {
        return None;
    };
    let right = add(&mul(a, y), &mul(b, y));
    Some(PlannedStep::new(
        Lemma::AddMul,
        vec![a.clone(), b.clone(), y.clone()],
        node,
        right,
    ))
}

#[derive(Clone, Copy)]
enum SortKind {
    Factors,
    Summands,
}

/// Phases 5 and 6 at `node = a ∘ b`: swap `a` with the head of `b` when it
/// sorts first — by `*_left_comm` when `b` continues the chain, else by
/// `*_comm`. Equal keys never swap, so `add_comm(x, x)` is never planned.
fn sort_step(
    node: &Term,
    a: &Term,
    b: &Term,
    atoms: &AtomOrder,
    kind: SortKind,
) -> Option<PlannedStep> {
    let (rest, head) = match (kind, shape(b)) {
        (SortKind::Factors, Shape::Mul(head, rest))
        | (SortKind::Summands, Shape::Add(head, rest)) => (Some(rest), head),
        _ => (None, b),
    };
    let precedes = match kind {
        SortKind::Factors => atoms.index(head)? < atoms.index(a)?,
        SortKind::Summands => summand_key(head, atoms)? < summand_key(a, atoms)?,
    };
    if !precedes {
        return None;
    }
    let combine = |x: &Term, y: &Term| match kind {
        SortKind::Factors => mul(x, y),
        SortKind::Summands => add(x, y),
    };
    let (comm, left_comm) = match kind {
        SortKind::Factors => (Lemma::MulComm, Lemma::MulLeftComm),
        SortKind::Summands => (Lemma::AddComm, Lemma::AddLeftComm),
    };
    Some(match rest {
        Some(rest) => PlannedStep::new(
            left_comm,
            vec![a.clone(), head.clone(), rest.clone()],
            node,
            combine(head, &combine(a, rest)),
        ),
        None => PlannedStep::new(comm, vec![a.clone(), head.clone()], node, combine(head, a)),
    })
}
