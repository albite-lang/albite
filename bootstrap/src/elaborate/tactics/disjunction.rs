//! `left` and `right`: prove a disjunction by one side (ADR 7.10.26l D2).
//!
//! On a focused goal `A + B` that is a disjunction, `left` leaves `A` as the
//! goal under the template `Inl(A + B, ?h)`, and `right` leaves `B` under
//! `Inr`. `exists w` is the model (ADR 6.10.26e D4): one step that builds the
//! goal's former and leaves its component open. The kernel types the
//! assembled `Inl`/`Inr` with the rule it already has.

use albite_core::{Term, Type};

use crate::ast::Tactic;
use crate::elaborate::exprs::{is_disjunction, Side};
use crate::elaborate::{ElabResult, Elaborator};

use super::not_applicable;
use super::state::{hole_name, Partial, ProofState};

impl Elaborator<'_> {
    /// `left` or `right` on `target`: the chosen side becomes a new goal.
    pub(super) fn run_disjunction_side(
        &mut self,
        tactic: &Tactic,
        side: Side,
        state: &mut ProofState,
        locals: &[(String, Type)],
        target: &Type,
    ) -> ElabResult<Partial> {
        let goal = self.normalize_for_comparison(target);
        let (left, right) = match &goal {
            Type::Sum(left, right) if is_disjunction(&goal) => (left, right),
            // Poison passes through (ADR 15.8.26d): its fault is already reported.
            Type::Error => return Ok(Partial::closed(Term::Sorry)),
            _ => {
                return Err(not_applicable(
                    tactic,
                    target,
                    "the goal is not a disjunction `A + B` of propositions",
                ))
            }
        };
        let (component, inject): (&Type, fn(Type, Term) -> Term) = match side {
            Side::Left => (left, Term::inl),
            Side::Right => (right, Term::inr),
        };
        let hole = state.fresh_meta(locals.to_vec(), component.clone());
        Ok(Partial {
            template: inject(goal.clone(), Term::var(hole_name(hole))),
            holes: vec![(hole_name(hole), hole)],
        })
    }
}
