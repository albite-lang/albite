//! `omega` (ADR 6.10.26f AC 2, AC 3): the solver over hand-built Core, and
//! the tactic through the elaborator.

mod elaboration;
mod solve;

use albite_core::{Term, Type};

use super::fragment::{Goal, Hypothesis};

pub(super) fn var(name: &str) -> Term {
    Term::var(name)
}

pub(super) fn add(a: Term, b: Term) -> Term {
    Term::nat_add(a, b)
}

pub(super) fn one() -> Term {
    Term::succ(Term::Zero)
}

/// `h : Le<s, t>` as the fragment reads it: witness `fst(h)`, equation
/// `s + fst(h) == t`.
pub(super) fn le_hypothesis(name: &str, s: Term, t: Term) -> Hypothesis {
    let witness = Term::fst(var(name));
    Hypothesis {
        name: name.to_string(),
        left: add(s, witness.clone()),
        right: t,
        witness: Some(witness),
    }
}

/// `h : Eq<Nat, s, t>`.
pub(super) fn eq_hypothesis(name: &str, s: Term, t: Term) -> Hypothesis {
    Hypothesis {
        name: name.to_string(),
        witness: None,
        left: s,
        right: t,
    }
}

/// The `Le` goal `Le<s, t>`.
pub(super) fn le_goal(s: Term, t: Term) -> Goal {
    Goal::Le { left: s, right: t }
}

/// `Σk:Nat. Eq(Nat, s + k, t)`.
pub(super) fn le_type(s: Term, t: Term) -> Type {
    Type::sigma("k", Type::Nat, Type::eq(Type::Nat, add(s, var("k")), t))
}
