//! `induction x` over a `Nat` local (ADR 3.10.26e D1, D2, D6).
//!
//! The focused goal `G` becomes two goals, `G[x := 0]` and
//! `Π x:Nat. G → G[x := x + 1]`, and its hole term is
//! `natind [x. Nat → G] ?base ?step x` — the shape 29.9.26d's kernel rule
//! already types. Nothing here is trusted: the kernel checks the assembled
//! whole (D5), so a wrong instance is an E0081 refusal, never a proof.

use albite_core::{Term, Type};

use crate::ast::{Ident, Tactic};
use crate::elaborate::exprs::natind_motive::motive_instance;
use crate::elaborate::{ElabResult, Elaborator};

use super::not_applicable;
use super::state::{hole_name, Partial, ProofState};

/// The motive and the two goals `induction` splits a goal into.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InductionGoals {
    /// `Nat → G`, stored as `natind` stores its motive.
    pub(crate) motive: Type,
    /// `G[x := 0]`.
    pub(crate) base: Type,
    /// `Π x:Nat. G → G[x := x + 1]`.
    pub(crate) step: Type,
}

/// `x + 1`, spelled as the source `x + 1` elaborates, so the library's
/// `+ 1` lemmas still meet the step goal under S2's syntactic `rewrite`
/// (D2). The kernel meets the rule's `succ x` by conversion.
#[must_use]
pub(crate) fn successor_spelling(target: &str) -> Term {
    Term::NatAdd(Box::new(Term::var(target)), Box::new(Term::nat_smart(1)))
}

/// D2's goals for induction on `target` in `goal`. The motive's binder is the
/// target's own name, so `motive_instance` captures no outer name.
#[must_use]
pub(crate) fn induction_goals(target: &str, goal: &Type) -> InductionGoals {
    let successor_case = motive_instance(target, goal, &successor_spelling(target));
    InductionGoals {
        motive: Type::arrow(Type::Nat, goal.clone()),
        base: motive_instance(target, goal, &Term::Zero),
        step: Type::pi(target, Type::Nat, Type::arrow(goal.clone(), successor_case)),
    }
}

/// `natind [target. motive] base_hole step_hole target`.
#[must_use]
pub(crate) fn induction_template(
    target: &str,
    goals: &InductionGoals,
    base_hole: &str,
    step_hole: &str,
) -> Term {
    Term::natind(
        target,
        goals.motive.clone(),
        Term::var(base_hole),
        Term::var(step_hole),
        Term::var(target),
    )
}

/// The first of `locals` other than the target whose type mentions it: the
/// hypothesis Lean would revert, refused here instead (D6).
#[must_use]
pub(crate) fn local_mentioning<'a>(
    target: &str,
    locals: impl IntoIterator<Item = (&'a str, &'a Type)>,
) -> Option<&'a str> {
    locals
        .into_iter()
        .find(|(name, ty)| *name != target && ty.free_term_vars().contains(target))
        .map(|(name, _)| name)
}

impl Elaborator<'_> {
    /// `induction x`: resolve `x`, refuse D6's three cases, and install the
    /// two-hole `natind` with the base goal focused first.
    pub(super) fn run_induction(
        &mut self,
        tactic: &Tactic,
        target: &Ident,
        state: &mut ProofState,
        locals: &[(String, Type)],
        goal: &Type,
    ) -> ElabResult<Partial> {
        let name = target.name.as_str();
        let mut parameters: Vec<_> = self.env.local_bindings().collect();
        parameters.sort_by_key(|binding| binding.level);
        let target_ty = locals
            .iter()
            .rev()
            .find(|(local, _)| local == name)
            .map(|(_, ty)| ty)
            .or_else(|| self.env.lookup_local(name).map(|binding| &binding.ty));
        let Some(target_ty) = target_ty else {
            return Err(not_applicable(
                tactic,
                goal,
                format!("`{name}` is not a variable in scope"),
            ));
        };
        // Poison passes through (ADR 15.8.26d): its fault is already reported.
        if !matches!(target_ty, Type::Nat | Type::Error) {
            return Err(not_applicable(
                tactic,
                goal,
                "`induction` is over `Nat` only; a user type waits on `cases`",
            ));
        }
        let in_scope = parameters
            .iter()
            .map(|binding| (binding.name.as_str(), &binding.ty))
            .chain(locals.iter().map(|(local, ty)| (local.as_str(), ty)));
        if let Some(mentioning) = local_mentioning(name, in_scope) {
            return Err(not_applicable(
                tactic,
                goal,
                format!("`{mentioning}` mentions `{name}`"),
            ));
        }
        let goals = induction_goals(name, goal);
        let base = state.fresh_meta(locals.to_vec(), goals.base.clone());
        let step = state.fresh_meta(locals.to_vec(), goals.step.clone());
        Ok(Partial {
            template: induction_template(name, &goals, &hole_name(base), &hole_name(step)),
            holes: vec![(hole_name(base), base), (hole_name(step), step)],
        })
    }
}
