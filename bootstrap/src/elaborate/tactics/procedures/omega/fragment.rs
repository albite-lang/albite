//! `omega`'s fragment (ADR 6.10.26f D2). Pure: functions over `Type`s the
//! caller has already expanded, so a family such as `Le<a, b>` arrives as its
//! `exists` and is read structurally, never by name.
//!
//! A **`Le` shape** is `Σk:Nat. Eq(Nat, s + k, t)` with `k` free in neither
//! `s` nor `t`; an **`Eq` shape** is `Eq(Nat, s, t)`. A goal of either shape
//! is in the fragment unless a side holds a term `ring` cannot read; a local
//! of either shape is a candidate hypothesis, and every other local is
//! ignored, never refused.

use albite_core::{Term, Type};

use crate::elaborate::tactics::procedures::ring::normal_form::{shape, Shape};

/// The first atom of `term` that applies a function named `pow`, or `None`.
/// `omega` keeps D7's refusal: `ring` unfolds a numeral power since ADR
/// 9.10.26j, but `omega`'s solver reads each side as linear, and a power is
/// not.
fn find_pow(term: &Term) -> Option<Term> {
    match shape(term) {
        Shape::Add(left, right) | Shape::Mul(left, right) => {
            find_pow(left).or_else(|| find_pow(right))
        }
        Shape::Numeral(_) => None,
        Shape::Atom => applies_pow(term).then(|| term.strip_spans()),
    }
}

/// Whether `term` is an application whose head is a global named `pow`.
fn applies_pow(term: &Term) -> bool {
    match term {
        Term::Spanned(inner, _) => applies_pow(inner),
        Term::App(head, _) => applies_pow(head),
        Term::Global(name) => name == "pow" || name.ends_with("::pow"),
        _ => false,
    }
}

/// `s` and `t` of a `Le` shape `Σk:Nat. Eq(Nat, s + k, t)`, with the binder
/// `k`, or `None`.
#[must_use]
pub(crate) fn le_shape(ty: &Type) -> Option<(&str, &Term, &Term)> {
    let Type::Sigma(bound, domain, body) = ty else {
        return None;
    };
    if **domain != Type::Nat {
        return None;
    }
    let (left, right) = eq_nat_shape(body)?;
    let Term::NatAdd(s, k) = strip(left) else {
        return None;
    };
    if strip(k) != &Term::Var(bound.clone()) {
        return None;
    }
    let free = |term: &Term| term.free_vars().contains(bound);
    (!free(s) && !free(right)).then_some((bound.as_str(), s, right))
}

/// `s` and `t` of `Eq(Nat, s, t)`, or `None`.
#[must_use]
pub(crate) fn eq_nat_shape(ty: &Type) -> Option<(&Term, &Term)> {
    match ty {
        Type::Eq(base, left, right) if **base == Type::Nat => Some((left, right)),
        _ => None,
    }
}

/// A term with its outer spans removed, borrowed.
fn strip(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) => strip(inner),
        other => other,
    }
}

/// The goal, as `omega` reads it: `s` and `t` of either shape.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Goal {
    /// `Le` shape: a witness `w` with `s + w == t` is to be found.
    Le { left: Term, right: Term },
    /// `Eq<Nat, s, t>`: `s` and `t` must agree once the hypotheses are used.
    Eq { left: Term, right: Term },
}

impl Goal {
    /// The goal's two sides, `s` then `t`.
    #[must_use]
    pub(crate) fn sides(&self) -> (&Term, &Term) {
        match self {
            Goal::Le { left, right } | Goal::Eq { left, right } => (left, right),
        }
    }
}

/// `goal` read as a `Le` or `Eq` shape, or `None` when it is neither
/// (E0082, D2).
#[must_use]
pub(crate) fn goal_shape(goal: &Type) -> Option<Goal> {
    if let Some((_, left, right)) = le_shape(goal) {
        return Some(Goal::Le {
            left: left.clone(),
            right: right.clone(),
        });
    }
    eq_nat_shape(goal).map(|(left, right)| Goal::Eq {
        left: left.clone(),
        right: right.clone(),
    })
}

/// The first term in `goal`'s sides outside `omega`'s fragment — a `pow` —
/// or `None` (D7's first message).
#[must_use]
pub(crate) fn outside_fragment(goal: &Goal) -> Option<Term> {
    let (left, right) = goal.sides();
    find_pow(left).or_else(|| find_pow(right))
}

/// A candidate hypothesis: its name, its witness atom `fst(h)` when it is
/// a `Le` shape, and the equation it proves, `left == right` — `snd(h)`'s
/// `s + fst(h) == t`, or an `Eq` local's own sides.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hypothesis {
    pub(crate) name: String,
    pub(crate) witness: Option<Term>,
    pub(crate) left: Term,
    pub(crate) right: Term,
}

/// The local `name : ty` as a candidate, or `None` when `ty` is neither
/// shape or a side holds a term `ring` cannot read: such a local is
/// ignored, as any other local is.
#[must_use]
pub(crate) fn hypothesis(name: &str, ty: &Type) -> Option<Hypothesis> {
    let candidate = if let Some((_, s, t)) = le_shape(ty) {
        let witness = Term::fst(Term::var(name));
        Hypothesis {
            name: name.to_string(),
            left: Term::nat_add(s.clone(), witness.clone()),
            right: t.clone(),
            witness: Some(witness),
        }
    } else {
        let (left, right) = eq_nat_shape(ty)?;
        Hypothesis {
            name: name.to_string(),
            witness: None,
            left: left.clone(),
            right: right.clone(),
        }
    };
    let readable = find_pow(&candidate.left).is_none() && find_pow(&candidate.right).is_none();
    readable.then_some(candidate)
}
