//! D2's term and per-arm goals (ADR 3.10.26f), pure: nothing here
//! elaborates or looks anything up, so every shape is tested over hand-built
//! Core.
//!
//! The term is `Annot(M, G)` with `M` the `Case`/`AdtMatch` the elaborator's
//! own `match` lowering would emit for the scrutinee's type, and each arm
//! `(λz₁:A₁. … λzₖ:Aₖ. ?Gᵢ) π₁(b) … πₖ(b)` — exactly the shape 3.10.26o's
//! kernel rule reads. Each `?Gᵢ` targets `G[v := Cᵢ(z₁,…,zₖ)]`, with `Cᵢ`
//! spelled as the elaborator spells a constructor application.

use albite_core::types::unfold_mu_type;
use albite_core::{Term, Type};

/// How the scrutinee's type is split: the case form, and the μ it is
/// unfolded at when the type is recursive.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SplitShape {
    pub(crate) former: Former,
    /// The scrutinee's declared μ, when the split reads `unfold [μ] v`.
    pub(crate) mu: Option<Type>,
    /// The type a constructor value is annotated with: the scrutinee's type,
    /// unfolded one level when it is a μ.
    pub(crate) unfolded: Type,
    /// Each constructor's payload, by index.
    pub(crate) payloads: Vec<Type>,
}

/// The two case forms the elaborator lowers a `match` to (§1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Former {
    /// A binary sum: `case`, index 0 the `inl`.
    Sum,
    /// A flat ADT of three or more constructors: `adt_match`.
    Adt,
}

/// The split a value of type `ty` takes, or `None` when `ty` is not a sum
/// or a flat ADT, directly or under one μ.
#[must_use]
pub(crate) fn split_shape(ty: &Type) -> Option<SplitShape> {
    let (mu, unfolded) = match ty {
        Type::Mu(..) => (Some(ty.clone()), unfold_mu_type(ty)),
        _ => (None, ty.clone()),
    };
    let (former, payloads) = match &unfolded {
        Type::Sum(left, right) => (Former::Sum, vec![(**left).clone(), (**right).clone()]),
        Type::Adt(_, _, variants) => (
            Former::Adt,
            variants
                .iter()
                .map(|(_, payload)| payload.clone())
                .collect(),
        ),
        _ => return None,
    };
    Some(SplitShape {
        former,
        mu,
        unfolded,
        payloads,
    })
}

/// A payload's `count` field types: none for `Unit`, the payload itself for
/// one, else the components of `A₁ × (A₂ × … Aₖ)`. `None` when the payload
/// does not have that many — the scrutinee is not the constructor's type.
#[must_use]
pub(crate) fn field_types(payload: &Type, count: usize) -> Option<Vec<Type>> {
    if count == 0 {
        return (*payload == Type::Unit).then(Vec::new);
    }
    let mut fields = Vec::with_capacity(count);
    let mut rest = payload;
    for _ in 1..count {
        let Type::Product(first, second) = rest else {
            return None;
        };
        fields.push((**first).clone());
        rest = second;
    }
    fields.push(rest.clone());
    Some(fields)
}

/// `⟨z₁,…,zₖ⟩`: `()`, `z`, or right-nested pairs.
fn tuple(fields: &[String]) -> Term {
    let mut names = fields.iter().rev().map(Term::var);
    let Some(last) = names.next() else {
        return Term::Unit;
    };
    names.fold(last, |rest, name| Term::pair(name, rest))
}

/// `Cᵢ(z₁,…,zₖ)` as the elaborator spells it: `inl`/`inr` or
/// `adt_construct` at the unfolded type, folded at the μ when there is one.
#[must_use]
pub(crate) fn constructor_value(shape: &SplitShape, index: usize, fields: &[String]) -> Term {
    let payload = tuple(fields);
    let value = match (shape.former, index) {
        (Former::Sum, 0) => Term::inl(shape.unfolded.clone(), payload),
        (Former::Sum, _) => Term::inr(shape.unfolded.clone(), payload),
        (Former::Adt, _) => Term::adt_construct(shape.unfolded.clone(), index, payload),
    };
    match &shape.mu {
        Some(mu) => Term::fold(mu.clone(), value),
        None => value,
    }
}

/// Arm `i`'s goal: `G[v := Cᵢ(z₁,…,zₖ)]`.
#[must_use]
pub(crate) fn arm_goal(
    variable: &str,
    goal: &Type,
    shape: &SplitShape,
    index: usize,
    fields: &[String],
) -> Type {
    goal.substitute_term(variable, &constructor_value(shape, index, fields))
}

/// `πᵢ(b)` of a right-nested `count`-tuple: `fst (snd^i b)`, or `snd^i b`
/// for the last component.
fn projection(binder: &str, index: usize, count: usize) -> Term {
    let rest = (0..index).fold(Term::var(binder), |term, _| Term::snd(term));
    if index + 1 < count {
        Term::fst(rest)
    } else {
        rest
    }
}

/// An arm's body: `(λz₁:A₁. … λzₖ:Aₖ. hole) π₁(b) … πₖ(b)`, or the hole
/// itself when the constructor has no field.
#[must_use]
pub(crate) fn arm_body(binder: &str, fields: &[(String, Type)], hole: &str) -> Term {
    let count = fields.len();
    let function = fields
        .iter()
        .rev()
        .fold(Term::var(hole), |body, (name, ty)| {
            Term::lambda(name, ty.clone(), body)
        });
    (0..count).fold(function, |applied, index| {
        Term::app(applied, projection(binder, index, count))
    })
}

/// One arm of the split: the constructor index it covers, its binder, and
/// its body.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SplitArm {
    pub(crate) index: usize,
    pub(crate) binder: String,
    pub(crate) body: Term,
}

/// `Annot(M, goal)`, `M` the case form over `v` (or `unfold [μ] v`) with
/// `arms`. A sum's arms are placed by index, `inl` first; an ADT's keep
/// their order, which the kernel does not read. `None` when a sum's split
/// lacks an arm for index 0 or 1, which D1's checks rule out.
#[must_use]
pub(crate) fn split_term(
    variable: &str,
    shape: &SplitShape,
    arms: Vec<SplitArm>,
    goal: &Type,
) -> Option<Term> {
    let scrutinee = match &shape.mu {
        Some(mu) => Term::unfold(mu.clone(), Term::var(variable)),
        None => Term::var(variable),
    };
    let split = match shape.former {
        Former::Sum => {
            let (left, right): (Vec<SplitArm>, Vec<SplitArm>) =
                arms.into_iter().partition(|arm| arm.index == 0);
            let (left, right) = (left.into_iter().next()?, right.into_iter().next()?);
            Term::case(scrutinee, left.binder, left.body, right.binder, right.body)
        }
        Former::Adt => Term::adt_match(
            scrutinee,
            arms.into_iter()
                .map(|arm| (arm.index, arm.binder, Box::new(arm.body)))
                .collect(),
        ),
    };
    Some(Term::annot(split, goal.clone()))
}
