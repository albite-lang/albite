//! Dependent case analysis on a local variable, proof mode only (ADR 3.10.26o).
//!
//! `Annot(M, T)` with `M` a `Case`/`AdtMatch` on `v` (or `unfold v`) types as
//! `T` when each arm proves `T[v := Cᵢ(⟨z₁,…,zₖ⟩)]` for its own constructor,
//! where the arm body is `(λz₁:A₁. … λzₖ:Aₖ. P) π₁(b) … πₖ(b)` over its
//! binder `b` (D1). The chain's length carries the constructor's field count,
//! which the encoding cannot: a record payload and a two-field payload are
//! both products.
//!
//! The rule is admissible in the standard model but not derivable: it asserts
//! surjective pairing and unit η (D3). It stays harmless because the kernel
//! never re-types a reduct of a proof term — `conversion/` imports nothing from
//! the type checker.

use std::collections::HashSet;

use crate::context::Context;
use crate::terms::{Term, Var};
use crate::types::Type;

use crate::typecheck::error::TypeError;
use crate::typecheck::rules::{type_of, types_equal_in};
use crate::typecheck::TypeResult;

#[cfg(test)]
thread_local! {
    /// How many splits were checked on this thread (D5's step count).
    pub(crate) static SPLITS_CHECKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether `term` has the outer shape D1 splits: a `Case` or an `AdtMatch`.
pub(in crate::typecheck) fn is_case_split(term: &Term) -> bool {
    matches!(peel(term), Term::Case(..) | Term::AdtMatch(..))
}

/// D1: type `split` as `goal` by dependent case analysis.
///
/// # Errors
///
/// `TypeError::DependentCaseShape` when the term is not D1's shape, or the
/// first `TypeError` an arm's proof raises against its instance.
pub(in crate::typecheck) fn type_of_dependent_case(
    ctx: &Context,
    split: &Term,
    goal: &Type,
) -> TypeResult<Type> {
    #[cfg(test)]
    SPLITS_CHECKED.with(|count| count.set(count.get() + 1));
    if !ctx.is_proof() {
        return Err(shape("executable mode does not split"));
    }
    let scrutinee = scrutinee_of(split)?;
    let (variable, mu) = scrutinee_variable(ctx, scrutinee)?;
    let scrutinee_ty = type_of(ctx, scrutinee)?;
    let (former, arms) = arms_at(split, &scrutinee_ty)?;

    let goal_free = goal.free_term_vars();
    let chains = arms
        .iter()
        .map(|arm| arm_chain(ctx, &goal_free, variable, arm))
        .collect::<TypeResult<Vec<_>>>()?;

    for (arm, chain) in arms.iter().zip(&chains) {
        let arm_ctx = chain.fields.iter().fold(ctx.clone(), |acc, (name, ty)| {
            acc.with_term(*name, (*ty).clone())
        });
        let constructed = former.construct(&scrutinee_ty, arm.index, chain.tuple(), mu);
        let instance = goal.substitute_term(variable, &constructed);
        let got = type_of(&arm_ctx, chain.proof)?;
        if !types_equal_in(&arm_ctx, &got, &instance) {
            return Err(TypeError::TypeMismatch {
                expected: instance,
                got,
            });
        }
    }
    Ok(goal.clone())
}

fn shape(why: &'static str) -> TypeError {
    TypeError::DependentCaseShape { why }
}

fn peel(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) => peel(inner),
        _ => term,
    }
}

/// One arm before its shape is checked: the constructor index it covers, its
/// binder, its body and the payload type the scrutinee gives it.
struct SplitArm<'a> {
    index: usize,
    binder: &'a str,
    body: &'a Term,
    payload: Type,
}

/// How a constructor of the scrutinee's type is written.
#[derive(Clone, Copy)]
enum Former {
    Sum,
    Adt,
}

impl Former {
    /// `Cᵢ(payload)` at the scrutinee's own type, folded at `v`'s μ when the
    /// scrutinee unfolds it.
    fn construct(
        self,
        scrutinee_ty: &Type,
        index: usize,
        payload: Term,
        mu: Option<&Type>,
    ) -> Term {
        let value = match (self, index) {
            (Former::Sum, 0) => Term::inl(scrutinee_ty.clone(), payload),
            (Former::Sum, _) => Term::inr(scrutinee_ty.clone(), payload),
            (Former::Adt, _) => Term::adt_construct(scrutinee_ty.clone(), index, payload),
        };
        match mu {
            Some(mu) => Term::fold(mu.clone(), value),
            None => value,
        }
    }
}

/// The scrutinee of a `Case`/`AdtMatch`.
fn scrutinee_of(split: &Term) -> TypeResult<&Term> {
    match peel(split) {
        Term::Case(scrutinee, ..) | Term::AdtMatch(scrutinee, _) => Ok(scrutinee),
        _ => Err(shape("not a case or adt_match")),
    }
}

/// The split's arms, each with the payload type `scrutinee_ty` gives it.
fn arms_at<'a>(split: &'a Term, scrutinee_ty: &Type) -> TypeResult<(Former, Vec<SplitArm<'a>>)> {
    let arm = |index, binder: &'a Var, body: &'a Term, payload: &Type| SplitArm {
        index,
        binder,
        body,
        payload: payload.clone(),
    };
    match (peel(split), scrutinee_ty) {
        (Term::Case(_, left_binder, left, right_binder, right), Type::Sum(left_ty, right_ty)) => {
            let arms = vec![
                arm(0, left_binder, left, left_ty),
                arm(1, right_binder, right, right_ty),
            ];
            Ok((Former::Sum, arms))
        }
        (Term::AdtMatch(_, arms), Type::Adt(_, _, variants)) => {
            covers_each_constructor_once(arms, variants.len())?;
            let arms = arms
                .iter()
                .map(|(index, binder, body)| arm(*index, binder, body, &variants[*index].1))
                .collect();
            Ok((Former::Adt, arms))
        }
        (_, Type::Error) => Err(TypeError::PoisonInProof { ty: Type::Error }),
        (Term::Case(..), _) => Err(shape("a case scrutinee that is not a sum")),
        _ => Err(shape("an adt_match scrutinee that is not an ADT")),
    }
}

/// D1: an `AdtMatch`'s arm indices are exactly `0..count`, each once.
fn covers_each_constructor_once(arms: &[(usize, Var, Box<Term>)], count: usize) -> TypeResult<()> {
    let mut seen = vec![false; count];
    for (index, _, _) in arms {
        match seen.get_mut(*index) {
            None => return Err(shape("an arm names no constructor")),
            Some(true) => return Err(shape("a constructor has two arms")),
            Some(slot) => *slot = true,
        }
    }
    if seen.contains(&false) {
        return Err(shape("a constructor has no arm"));
    }
    Ok(())
}

/// The variable `v` the split abstracts, and the μ it is unfolded at, if any.
/// `v` must be a term binding: a global's value is not a variable the motive
/// may abstract.
fn scrutinee_variable<'a>(
    ctx: &Context,
    scrutinee: &'a Term,
) -> TypeResult<(&'a str, Option<&'a Type>)> {
    let (folded, mu) = match peel(scrutinee) {
        Term::Unfold(mu, inner) => (peel(inner), Some(mu)),
        other => (other, None),
    };
    let Term::Var(variable) = folded else {
        return Err(shape("the scrutinee is not a variable"));
    };
    let is_term_binding =
        ctx.lookup_term(variable).is_some() && ctx.lookup_global_admission(variable).is_none();
    if is_term_binding {
        Ok((variable, mu))
    } else {
        Err(shape("the scrutinee is not a term binding of the context"))
    }
}

/// An arm read as D1's chain: the fields it binds and the proof under them.
struct ArmChain<'a> {
    fields: Vec<(&'a str, &'a Type)>,
    proof: &'a Term,
}

impl ArmChain<'_> {
    /// `⟨z₁,…,zₖ⟩`: `()`, `z`, or right-nested pairs.
    fn tuple(&self) -> Term {
        let mut names = self.fields.iter().rev().map(|(name, _)| Term::var(*name));
        let Some(last) = names.next() else {
            return Term::Unit;
        };
        names.fold(last, |rest, name| Term::pair(name, rest))
    }
}

/// D1's shape for one arm: the chain over the binder's projections, its
/// binder types the payload's components, and no name that could capture.
fn arm_chain<'a>(
    ctx: &Context,
    goal_free: &HashSet<Var>,
    variable: &str,
    arm: &'a SplitArm<'a>,
) -> TypeResult<ArmChain<'a>> {
    let chain = read_chain(arm.binder, arm.body)?;
    let components = payload_components(&arm.payload, chain.fields.len())?;
    for ((_, field_ty), component) in chain.fields.iter().zip(components) {
        if !types_equal_in(ctx, field_ty, component) {
            return Err(shape("a binder's type is not its payload component"));
        }
    }
    let mut names = HashSet::new();
    for name in std::iter::once(arm.binder).chain(chain.fields.iter().map(|(name, _)| *name)) {
        fresh_name(ctx, goal_free, variable, name)?;
    }
    for (name, _) in &chain.fields {
        if !names.insert(*name) {
            return Err(shape("two fields of one arm share a name"));
        }
    }
    Ok(chain)
}

/// Read `body` as `(λz₁. … λzₖ. P) π₁(b) … πₖ(b)`; with no application of a
/// λ, `k = 0` and the body is `P`.
fn read_chain<'a>(binder: &str, body: &'a Term) -> TypeResult<ArmChain<'a>> {
    let mut args = Vec::new();
    let mut head = peel(body);
    while let Term::App(function, arg) = head {
        args.push(&**arg);
        head = peel(function);
    }
    args.reverse();
    if args.is_empty() || !matches!(head, Term::Lambda(..)) {
        return Ok(ArmChain {
            fields: Vec::new(),
            proof: body,
        });
    }
    let count = args.len();
    let mut fields = Vec::with_capacity(count);
    let mut proof = head;
    for (index, arg) in args.iter().enumerate() {
        let Term::Lambda(name, ty, inner) = peel(proof) else {
            return Err(shape("fewer λs than projections"));
        };
        if !is_projection(arg, binder, index, count) {
            return Err(shape("an argument is not the binder's projection"));
        }
        fields.push((name.as_str(), ty));
        proof = inner;
    }
    Ok(ArmChain { fields, proof })
}

/// Whether `arg` is `πᵢ(b)` of a right-nested `count`-tuple: `fst (snd^i b)`,
/// or `snd^i b` for the last component.
fn is_projection(arg: &Term, binder: &str, index: usize, count: usize) -> bool {
    let mut term = peel(arg);
    if index + 1 < count {
        let Term::Fst(inner) = term else {
            return false;
        };
        term = peel(inner);
    }
    for _ in 0..index {
        let Term::Snd(inner) = term else {
            return false;
        };
        term = peel(inner);
    }
    matches!(term, Term::Var(name) if name == binder)
}

/// The payload's `count` components: `Unit` for none, the payload for one,
/// else `A₁ × (A₂ × … Aₖ)`.
fn payload_components(payload: &Type, count: usize) -> TypeResult<Vec<&Type>> {
    if count == 0 {
        return if *payload == Type::Unit {
            Ok(Vec::new())
        } else {
            Err(shape("an arm with no fields needs a Unit payload"))
        };
    }
    let mut components = Vec::with_capacity(count);
    let mut rest = payload;
    for _ in 1..count {
        match rest {
            Type::Product(first, second) => {
                components.push(&**first);
                rest = second;
            }
            Type::Error => return Err(TypeError::PoisonInProof { ty: Type::Error }),
            _ => return Err(shape("the payload has fewer components than the chain")),
        }
    }
    components.push(rest);
    Ok(components)
}

/// No capture (D1): `name` is not the scrutinee, not free in the goal, and
/// shadows no binding of the context.
fn fresh_name(
    ctx: &Context,
    goal_free: &HashSet<Var>,
    variable: &str,
    name: &str,
) -> TypeResult<()> {
    if name == variable {
        Err(shape("a binder shadows the scrutinee"))
    } else if goal_free.contains(name) {
        Err(shape("a binder occurs free in the goal"))
    } else if ctx.lookup_term(name).is_some() {
        Err(shape("a binder shadows a binding of the context"))
    } else {
        Ok(())
    }
}
