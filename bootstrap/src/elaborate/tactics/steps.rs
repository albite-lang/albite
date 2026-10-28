//! One function per tactic (ADR 2.10.26c D3, D4). Each returns the
//! [`Partial`] that solves the focused goal; the run loop installs it.
//!
//! Every expression a user writes goes through the elaborator's own `check`
//! or `infer`, so its errors are today's. The engine itself checks only the
//! shape it assembles: a `λ` against an arrow, an application against a
//! matched conclusion.

use std::collections::BTreeMap;

use albite_core::{Term, Type};

use crate::ast::{Expr, Ident, Tactic};
use crate::elaborate::exprs::Side;
use crate::elaborate::{ElabError, ElabResult, Elaborator};
use crate::span::{Span, Spanned};

use super::cases::GoalScope;
use super::matching::{instantiate, match_conclusion};
use super::rewrite::RuleStep;
use super::state::{hole_name, MetaId, Partial, ProofState};
use super::{is_tactic_proposition, not_applicable};

/// One argument `apply` passes, in the lemma's binder order: a `Π` variable
/// the matcher must bind, or an arrow premise that becomes a goal.
enum ApplyArgument {
    Matched(String),
    Premise(Type),
}

impl Elaborator<'_> {
    /// Dispatch on the tactic. `step` is the step's index within `tactic`:
    /// a `rewrite` runs one step per rule (ADR 2.10.26e D6), the rest one.
    /// `simp` takes as many steps as its rounds do, so it records its own
    /// (`run_simp`, ADR 4.10.26e D4) and never reaches this dispatch; nor
    /// does `ring`, which records one step per citation (`run_ring`, ADR
    /// 4.10.26g D4), nor `omega` (`run_omega`, ADR 6.10.26f D5).
    pub(super) fn run_tactic(
        &mut self,
        tactic: &Tactic,
        step: usize,
        state: &mut ProofState,
        goal: MetaId,
    ) -> ElabResult<Partial> {
        let locals = state.metas[goal].locals.clone();
        let target = state.metas[goal].target.clone();
        match tactic {
            Tactic::Rewrite(rules, _) => {
                let step = RuleStep {
                    tactic: tactic.name(),
                    rule: &rules[step],
                    position: step + 1,
                };
                self.run_rewrite_rule(step, state, goal)
            }
            Tactic::Simp(..) => Err(ElabError::internal(
                tactic.span(),
                "`simp` is run by `run_simp`, not the one-step dispatch".to_string(),
            )),
            Tactic::Ring(_) => Err(ElabError::internal(
                tactic.span(),
                "`ring` is run by `run_ring`, not the one-step dispatch".to_string(),
            )),
            Tactic::Omega(_) => Err(ElabError::internal(
                tactic.span(),
                "`omega` is run by `run_omega`, not the one-step dispatch".to_string(),
            )),
            Tactic::Intro(binder, _) => self.run_intro(tactic, binder, state, &locals, &target),
            Tactic::Exact(expr, _) => self
                .with_goal_locals(&locals, |elab| elab.check(expr, &target))
                .map(Partial::closed),
            Tactic::Refl(span) => self.run_refl(*span, &locals, &target),
            Tactic::Apply(expr, _) => self.run_apply(tactic, expr, state, &locals, &target),
            Tactic::Induction(variable, _) => {
                self.run_induction(tactic, variable, state, &locals, &target)
            }
            Tactic::Cases(scrutinee, arms, _) => {
                let scope = GoalScope {
                    id: goal,
                    locals: &locals,
                    target: &target,
                };
                self.run_cases(tactic, scrutinee, arms, state, scope)
            }
            Tactic::CasesPair(scrutinee, witness, evidence, _, _) => {
                let scope = GoalScope {
                    id: goal,
                    locals: &locals,
                    target: &target,
                };
                self.run_cases_pair(tactic, scrutinee, [witness, evidence], state, scope)
            }
            Tactic::Exists(witness, _) => self.run_exists(tactic, witness, state, &locals, &target),
            Tactic::Left(_) => {
                self.run_disjunction_side(tactic, Side::Left, state, &locals, &target)
            }
            Tactic::Right(_) => {
                self.run_disjunction_side(tactic, Side::Right, state, &locals, &target)
            }
        }
    }

    /// `exists w` on `Σx:A. B`: check `w : A`, and emit `(w, ?g) : Σx:A. B`
    /// with `?g : B[x := w]` — the term 4.10.26h D5's tuple check builds, so
    /// the kernel types it with the annotation rule it already has (ADR
    /// 6.10.26e D4).
    fn run_exists(
        &mut self,
        tactic: &Tactic,
        witness: &Expr,
        state: &mut ProofState,
        locals: &[(String, Type)],
        target: &Type,
    ) -> ElabResult<Partial> {
        let goal = self.normalize_for_comparison(target);
        let (bound, domain, body) = match &goal {
            Type::Sigma(bound, domain, body) => (bound, domain, body),
            // Poison passes through (ADR 15.8.26d): its fault is already reported.
            Type::Error => return Ok(Partial::closed(Term::Sorry)),
            _ => {
                return Err(not_applicable(
                    tactic,
                    target,
                    "`exists` needs an `exists` goal",
                ))
            }
        };
        let witness = self.with_goal_locals(locals, |elab| elab.check(witness, domain))?;
        let hole = state.fresh_meta(locals.to_vec(), body.substitute_term(bound, &witness));
        Ok(Partial {
            template: Term::annot(
                Term::pair(witness, Term::var(hole_name(hole))),
                goal.clone(),
            ),
            holes: vec![(hole_name(hole), hole)],
        })
    }

    /// Run `f` with the goal's `intro` locals bound, as `elaborate_theorem_body`
    /// binds the parameters.
    pub(super) fn with_goal_locals<T>(
        &mut self,
        locals: &[(String, Type)],
        f: impl FnOnce(&mut Self) -> ElabResult<T>,
    ) -> ElabResult<T> {
        let saved_depth = self.depth;
        self.env.push_scope();
        for (name, ty) in locals {
            self.env.bind_local(name.clone(), ty.clone(), self.depth);
            self.depth += 1;
        }
        let result = f(self);
        self.env.pop_scope();
        self.depth = saved_depth;
        result
    }

    /// `refl` is `check(refl, target)`: E0070, E0071, the δ-free fast path and
    /// 28.9.26d's deferred δ all come with it. `simp` closes with it too
    /// (ADR 4.10.26e D2); a refusal records nothing and defers nothing.
    pub(super) fn run_refl(
        &mut self,
        span: Span,
        locals: &[(String, Type)],
        target: &Type,
    ) -> ElabResult<Partial> {
        self.with_goal_locals(locals, |elab| elab.check(&Expr::Refl(span), target))
            .map(Partial::closed)
    }

    /// `intro x` on `A → B` or `Π y:A. B`: `λx:A. ?g` with `?g : B[y := x]`.
    fn run_intro(
        &mut self,
        tactic: &Tactic,
        binder: &Ident,
        state: &mut ProofState,
        locals: &[(String, Type)],
        target: &Type,
    ) -> ElabResult<Partial> {
        let name = &binder.name;
        let (domain, body) = match target {
            Type::Arrow(domain, body) => ((**domain).clone(), (**body).clone()),
            Type::Pi(bound, domain, body) => (
                (**domain).clone(),
                body.substitute_term(bound, &Term::var(name.clone())),
            ),
            // Poison passes through (ADR 15.8.26d): its fault is already reported.
            Type::Error => (Type::Error, Type::Error),
            _ => {
                return Err(not_applicable(
                    tactic,
                    target,
                    "the goal is not a function type",
                ))
            }
        };
        // Shadowing would let the grafted `λ` capture a name the target or an
        // outer hole still uses with another meaning (D4).
        if self.env.lookup_local(name).is_some() || locals.iter().any(|(n, _)| n == name) {
            return Err(not_applicable(
                tactic,
                target,
                format!("`{name}` is already bound"),
            ));
        }
        let mut inner_locals = locals.to_vec();
        inner_locals.push((name.clone(), domain.clone()));
        let hole = state.fresh_meta(inner_locals, body);
        Ok(Partial {
            template: Term::lambda(name, domain, Term::var(hole_name(hole))),
            holes: vec![(hole_name(hole), hole)],
        })
    }

    /// `apply e`: peel `e`'s type into `Π` variables and arrow premises, match
    /// the conclusion against the target, and emit `e a₁ … ?gᵢ …` (D3).
    fn run_apply(
        &mut self,
        tactic: &Tactic,
        expr: &Expr,
        state: &mut ProofState,
        locals: &[(String, Type)],
        target: &Type,
    ) -> ElabResult<Partial> {
        let (head, lemma_ty) = self.with_goal_locals(locals, |elab| elab.infer(expr))?;
        // The head is assembled Core: abstracting a hole in it would exempt
        // the very shape `apply` builds (D8).
        if head.contains_sorry() {
            return Err(not_applicable(tactic, target, "sorry in an apply head"));
        }
        if matches!(lemma_ty, Type::Forall(..)) {
            return Err(not_applicable(
                tactic,
                target,
                "a generic lemma cannot be applied in S1",
            ));
        }
        let (arguments, conclusion) = peel_lemma(&lemma_ty);
        let vars: Vec<String> = arguments
            .iter()
            .filter_map(|argument| match argument {
                ApplyArgument::Matched(var) => Some(var.clone()),
                ApplyArgument::Premise(_) => None,
            })
            .collect();
        let Some(bindings) = match_conclusion(&conclusion, &vars, target) else {
            return Err(not_applicable(
                tactic,
                target,
                format!("its conclusion `{conclusion}` does not match"),
            ));
        };
        if let Some(unbound) = vars.iter().find(|var| !bindings.contains_key(*var)) {
            return Err(not_applicable(
                tactic,
                target,
                format!("cannot infer `{unbound}`"),
            ));
        }
        let mut premises = arguments.iter().filter_map(|argument| match argument {
            ApplyArgument::Premise(premise) => Some(premise),
            ApplyArgument::Matched(_) => None,
        });
        if let Some(why) = premises.find_map(premise_refusal) {
            return Err(not_applicable(tactic, target, why));
        }
        Ok(assemble_application(
            head, &arguments, &bindings, state, locals,
        ))
    }
}

/// Why `apply` cannot make `premise` a goal, or `None` when it can (ADR
/// 6.10.26e D2). A non-dependent data parameter peels as an arrow, but S1
/// makes no term-valued goal (2.10.26c D3): only a premise that sorts `Prop`
/// becomes one, a `Le` or an `exists` included. Of those, one whose spine
/// reaches a `Forall` is refused too: no tactic introduces a type variable,
/// so its goal could never be opened.
pub(crate) fn premise_refusal(premise: &Type) -> Option<String> {
    if !is_tactic_proposition(premise) {
        return Some(format!("cannot infer an argument of type `{premise}`"));
    }
    reaches_forall(premise).then(|| "a generic premise cannot become a goal in S1".to_string())
}

/// Whether `ty`'s `Arrow`/`Pi` spine reaches a `Forall`.
fn reaches_forall(ty: &Type) -> bool {
    match ty {
        Type::Forall(..) => true,
        Type::Arrow(_, body) | Type::Pi(_, _, body) => reaches_forall(body),
        _ => false,
    }
}

/// A lemma's type as its arguments, in order, and the conclusion under them.
fn peel_lemma(lemma_ty: &Type) -> (Vec<ApplyArgument>, Type) {
    let mut arguments = Vec::new();
    let mut rest = lemma_ty.clone();
    loop {
        match rest {
            Type::Pi(var, _, body) => {
                arguments.push(ApplyArgument::Matched(var));
                rest = *body;
            }
            Type::Arrow(premise, body) => {
                arguments.push(ApplyArgument::Premise(*premise));
                rest = *body;
            }
            conclusion => return (arguments, conclusion),
        }
    }
}

/// `head a₁ … aₙ`, one `App` per argument in order: a matched variable's
/// term, or a fresh goal's hole for each premise, instantiated by the match.
fn assemble_application(
    head: Term,
    arguments: &[ApplyArgument],
    bindings: &BTreeMap<String, Term>,
    state: &mut ProofState,
    locals: &[(String, Type)],
) -> Partial {
    let mut template = head;
    let mut holes = Vec::new();
    for argument in arguments {
        let argument_term = match argument {
            ApplyArgument::Matched(var) => bindings[var].clone(),
            ApplyArgument::Premise(premise) => {
                let goal = state.fresh_meta(locals.to_vec(), instantiate(premise, bindings));
                holes.push((hole_name(goal), goal));
                Term::var(hole_name(goal))
            }
        };
        template = Term::app(template, argument_term);
    }
    Partial { template, holes }
}
