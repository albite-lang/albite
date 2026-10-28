//! The sort judgment: which class a type belongs to (ADR 4.10.26i D1,
//! `universe_hierarchy.md` Q2 as amended by 3.10.26h D4).
//!
//! `Prop` and `Type₀` are not types — no term has either as its type. They
//! classify types: `Eq`, `Void` and the connectives over them are
//! propositions, everything else is data. ⊤ is `0 == 0`, never `Unit`, so a
//! two-constructor enum (`Sum(Unit, Unit)`) stays data.
//!
//! There is no context argument: a type variable is `Type₀`, and a predicate
//! variable is never used bare — applied, it is a `PredApp`, which sorts
//! `Prop` structurally (ADR 6.10.26h D6).
//!
//! Erasure never reads this (D8): `sort_of` is wider than
//! [`Type::is_proposition`], which stays the erasure key.

use super::Type;

/// The two sorts of the first sort layer: propositions and data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Propositions: `Eq`, `Void`, and ∧ ∨ → ∃ over them.
    Prop,
    /// Data, spelled `Type` in source: every other type.
    Type0,
}

impl Type {
    /// The sort that classifies `self`, or `None` for a poison (`Error`) or a
    /// deferred application (`App`), neither of which reaches final Core.
    #[must_use]
    pub fn sort_of(&self) -> Option<Sort> {
        match self {
            // A predicate variable applied sorts `Prop` structurally, so Γ is
            // still not an argument (ADR 6.10.26h D6 overturns 4.10.26i D1's
            // plan to add one).
            Type::Eq(..) | Type::Void | Type::PredApp(..) => Some(Sort::Prop),
            Type::Product(left, right) | Type::Sum(left, right) => {
                Some(both_prop(left.sort_of()?, right.sort_of()?))
            }
            Type::Arrow(_, codomain)
            | Type::Pi(_, _, codomain)
            | Type::Sigma(_, _, codomain)
            | Type::Forall(_, codomain)
            | Type::ForallPred(_, _, codomain) => codomain.sort_of(),
            // An instantiating predicate is not a type with inhabitants
            Type::Error | Type::App(..) | Type::PredAbs(..) => None,
            Type::Bool
            | Type::Nat
            | Type::Int
            | Type::Unit
            | Type::String
            | Type::TyVar(_)
            | Type::Mu(..)
            | Type::Adt(..)
            | Type::Ptr(_)
            | Type::Ref(_) => Some(Sort::Type0),
        }
    }
}

impl Type {
    /// Whether this type is a proposition: an equality, `Void`, or a function
    /// type ending in one — never a data type. Codegen erases proofs of
    /// them (ADR 3.10.26e D4); tactics make goals of the wider
    /// `is_tactic_proposition`, a `sort_of` test (ADR 6.10.26e D2), which is
    /// not the erasure key. Moved beside [`Type::sort_of`], its natural
    /// reader's neighbour, by ADR 6.10.26h D4; a `PredApp` sorts `Prop` and is
    /// still not one here (D7).
    #[must_use]
    pub fn is_proposition(&self) -> bool {
        match self {
            Type::Eq(..) | Type::Void => true,
            Type::Arrow(_, body) | Type::Pi(_, _, body) => body.is_proposition(),
            _ => false,
        }
    }
}

/// ∧ and ∨ are propositions only when both sides are.
fn both_prop(left: Sort, right: Sort) -> Sort {
    if left == Sort::Prop && right == Sort::Prop {
        Sort::Prop
    } else {
        Sort::Type0
    }
}

#[cfg(test)]
mod tests;
