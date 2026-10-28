//! The pair arm: `cases h { (k, e) => tactics }` over a local `h : Σx:A. B`
//! (ADR 6.10.26e D6).
//!
//! The installed term is 6.10.26c's β-redex, built by `split.rs`'s own
//! `arm_body`: `Annot((λk:A. λe:B[x := k]. ?arm) (fst h) (snd h), G)`. The
//! arm's goal is the focused goal `G` exactly as stated — never
//! `G[h := (k, e)]`, which would need surjective pairing — and its hole sits
//! under `λk. λe`, so the syntactic graft lands its solution where both names
//! are bound. The kernel does `k := fst h` with its own λ and application
//! rules; nothing here substitutes after the meta is solved.

use albite_core::{Term, Type};

use crate::ast::{Expr, Ident, Tactic};
use crate::elaborate::{ElabResult, Elaborator};

use crate::elaborate::tactics::not_applicable;
use crate::elaborate::tactics::state::{hole_name, Partial, ProofState};

use super::split::arm_body;
use super::{scrutinee_name, GoalScope};

/// The pair arm's two fields: the witness `k : A` and the evidence
/// `e : B[x := k]`, for `names` `[k, e]` and the `exists` `Σx:A. B`.
/// `None` when `pair` is no `exists`.
#[must_use]
pub(crate) fn pair_fields(pair: &Type, names: [String; 2]) -> Option<Vec<(String, Type)>> {
    let Type::Sigma(bound, witness_ty, evidence_ty) = pair else {
        return None;
    };
    let [witness, evidence] = names;
    let evidence_ty = evidence_ty.substitute_term(bound, &Term::var(witness.clone()));
    Some(vec![
        (witness, (**witness_ty).clone()),
        (evidence, evidence_ty),
    ])
}

/// `Annot((λk. λe. hole) (fst variable) (snd variable), goal)`.
#[must_use]
pub(crate) fn pair_template(
    variable: &str,
    fields: &[(String, Type)],
    hole: &str,
    goal: &Type,
) -> Term {
    Term::annot(arm_body(variable, fields, hole), goal.clone())
}

impl Elaborator<'_> {
    /// `cases h { (k, e) => … }`: `h` must be a local whose type is an
    /// `exists`. Installs the redex with one hole, whose locals are the
    /// goal's plus `k` and `e`, and whose target is the goal unchanged; the
    /// run loop then runs the arm on it alone.
    pub(in crate::elaborate::tactics) fn run_cases_pair(
        &mut self,
        tactic: &Tactic,
        scrutinee: &Expr,
        binders: [&Option<Ident>; 2],
        state: &mut ProofState,
        scope: GoalScope<'_>,
    ) -> ElabResult<Partial> {
        let goal = scope.target;
        let refuse = |why: String| not_applicable(tactic, goal, why);
        let variable = scrutinee_name(scrutinee)
            .ok_or_else(|| refuse("the scrutinee is not a local".to_string()))?;
        let scrutinee_ty = self
            .cases_scrutinee_type(variable, scope.locals)
            .map_err(refuse)?;
        let pair = self.normalize_for_comparison(&scrutinee_ty);
        // Poison passes through (ADR 15.8.26d): its fault is already reported.
        if pair == Type::Error {
            return Ok(Partial::closed(Term::Sorry));
        }
        let binders = [binders[0].clone(), binders[1].clone()];
        let [witness, evidence]: [String; 2] = self
            .cases_field_names(variable, &binders, 0, scope)
            .map_err(refuse)?
            .try_into()
            .map_err(|_| refuse("a pair arm binds two names".to_string()))?;
        let fields = pair_fields(&pair, [witness, evidence]).ok_or_else(|| {
            refuse(format!(
                "a pair arm needs an `exists` local, and `{variable}` is `{scrutinee_ty}`"
            ))
        })?;
        let mut arm_locals = scope.locals.to_vec();
        arm_locals.extend(fields.iter().cloned());
        let hole = state.fresh_meta(arm_locals, goal.clone());
        Ok(Partial {
            template: pair_template(variable, &fields, &hole_name(hole), goal),
            holes: vec![(hole_name(hole), hole)],
        })
    }
}
