//! `cases x { C₁ => tac, C₂(a, b) => { tac; tac } }` over a user data type
//! (ADR 3.10.26f D1, D2, D4).
//!
//! The focused goal `G` becomes one goal per constructor, `G[x := Cᵢ(z…)]`,
//! and its hole term is the annotated `case`/`adt_match` 3.10.26o's kernel
//! rule types (`split.rs`). Each arm's tactics then run on that arm's goal
//! alone and must close it. Nothing here is trusted: the kernel checks the
//! assembled whole, so a wrong split is an E0081 refusal, never a proof.
//!
//! A disjunction `A + B` splits the same way, its arms named `inl` and `inr`
//! where no constructor of those names is in scope (ADR 7.10.26l D3).
//!
//! `pair.rs` is the pair arm, `cases h { (k, e) => … }`, which unpacks an
//! `exists` local rather than splitting a data type (ADR 6.10.26e D6).

mod pair;
mod split;
#[cfg(test)]
mod tests;

use albite_core::{Term, Type};

use crate::ast::{CasesArm, Expr, Ident, Path, Tactic};
use crate::elaborate::env::TypeDefKind;
use crate::elaborate::error::{ElabError, ElabErrorKind, TacticErrorKind};
use crate::elaborate::exprs::{is_disjunction, ownership, Side};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};

use super::not_applicable;
use super::state::{hole_name, MetaId, Partial, ProofState};
use super::TacticStep;
use split::{arm_body, arm_goal, field_types, split_shape, split_term, SplitArm, SplitShape};

/// The prefix of every name `cases` synthesises — an arm's binder and a `_`
/// field. `%` fails the lexer's `is_ident_start`, so no source name is one.
const CASES_NAME_PREFIX: &str = "%cases";

/// The focused goal `cases` splits: its meta, its locals and its target.
#[derive(Clone, Copy)]
pub(super) struct GoalScope<'a> {
    pub(super) id: MetaId,
    pub(super) locals: &'a [(String, Type)],
    pub(super) target: &'a Type,
}

/// One resolved arm: the constructor index it covers and the names and
/// types of the fields it binds.
struct ResolvedArm {
    index: usize,
    fields: Vec<(String, Type)>,
}

/// What an arm's name resolved to: the owning type (`None` for a
/// disjunction's built-in `inl`/`inr`), the constructor index and its arity.
struct ArmConstructor {
    owner: Option<String>,
    index: usize,
    arity: usize,
}

/// The scrutinee's name, when it is a plain variable.
fn scrutinee_name(scrutinee: &Expr) -> Option<&str> {
    match scrutinee {
        Expr::Paren(inner, _) => scrutinee_name(inner),
        Expr::Path(path) if path.is_simple() => Some(path.item_name().name.as_str()),
        _ => None,
    }
}

impl Elaborator<'_> {
    /// `cases x { … }`: resolve `x` and the arms, refuse D4's cases, and
    /// install the split with one hole per arm, in the arms' order.
    pub(super) fn run_cases(
        &mut self,
        tactic: &Tactic,
        scrutinee: &Expr,
        arms: &[CasesArm],
        state: &mut ProofState,
        scope: GoalScope<'_>,
    ) -> ElabResult<Partial> {
        let GoalScope {
            id: goal_id,
            locals,
            target: goal,
        } = scope;
        let refuse = |why: String| not_applicable(tactic, goal, why);
        let variable = scrutinee_name(scrutinee)
            .ok_or_else(|| refuse("the scrutinee is not a local".to_string()))?;
        let scrutinee_ty = self
            .cases_scrutinee_type(variable, locals)
            .map_err(refuse)?;
        // Poison passes through (ADR 15.8.26d): its fault is already reported.
        let Some(shape) = self.cases_shape(&scrutinee_ty).map_err(refuse)? else {
            return Ok(Partial::closed(Term::Sorry));
        };
        let resolved = self
            .resolve_cases_arms(variable, &scrutinee_ty, &shape, arms, scope)
            .map_err(refuse)?;
        let mut holes = Vec::with_capacity(resolved.len());
        let mut split_arms = Vec::with_capacity(resolved.len());
        for arm in resolved {
            let names: Vec<String> = arm.fields.iter().map(|(name, _)| name.clone()).collect();
            let mut arm_locals = locals.to_vec();
            arm_locals.extend(arm.fields.iter().cloned());
            let target = arm_goal(variable, goal, &shape, arm.index, &names);
            let hole = state.fresh_meta(arm_locals, target);
            let binder = format!("{CASES_NAME_PREFIX}{goal_id}_{}", arm.index);
            split_arms.push(SplitArm {
                index: arm.index,
                body: arm_body(&binder, &arm.fields, &hole_name(hole)),
                binder,
            });
            holes.push((hole_name(hole), hole));
        }
        let template = split_term(variable, &shape, split_arms, goal).ok_or_else(|| {
            ElabError::internal(tactic.span(), "a sum's split lacks an arm".to_string())
        })?;
        Ok(Partial { template, holes })
    }

    /// The type of the local `variable`, or why it is not a local.
    fn cases_scrutinee_type(
        &self,
        variable: &str,
        locals: &[(String, Type)],
    ) -> Result<Type, String> {
        let local = locals
            .iter()
            .rev()
            .find(|(local, _)| local == variable)
            .map(|(_, ty)| ty)
            .or_else(|| self.env.lookup_local(variable).map(|binding| &binding.ty));
        match local {
            Some(ty) => Ok(ty.clone()),
            None if self.env.has_value(variable) => Err(format!(
                "the scrutinee is not a local: `{variable}` is a global"
            )),
            None => Err(format!("`{variable}` is not a variable in scope")),
        }
    }

    /// The split the scrutinee's type takes; `None` for a poisoned type.
    fn cases_shape(&self, ty: &Type) -> Result<Option<SplitShape>, String> {
        match ty {
            Type::Error => Ok(None),
            Type::Nat => Err("`Nat` is split by `induction`".to_string()),
            Type::Bool => Err("`Bool` is not a user data type".to_string()),
            Type::TyVar(name) if self.is_record_type(name) => Err(format!(
                "`{}` is a record: it has one constructor, reached by projection",
                name.strip_prefix('@').unwrap_or(name)
            )),
            _ => split_shape(ty)
                .map(Some)
                .ok_or_else(|| format!("`{ty}` is not a user data type")),
        }
    }

    /// Whether `name` (`@`-prefixed while it is collected) names a record.
    fn is_record_type(&self, name: &str) -> bool {
        let name = name.strip_prefix('@').unwrap_or(name);
        self.env
            .lookup_type(name)
            .is_some_and(|def| matches!(def.kind, TypeDefKind::Record(_)))
    }

    /// D1's arm checks: each names a constructor of the scrutinee's type,
    /// binds one name per declared field, and every constructor has exactly
    /// one arm. Returns the arms in source order with their fields named.
    fn resolve_cases_arms(
        &self,
        variable: &str,
        scrutinee_ty: &Type,
        shape: &SplitShape,
        arms: &[CasesArm],
        scope: GoalScope<'_>,
    ) -> Result<Vec<ResolvedArm>, String> {
        let mut type_name: Option<String> = None;
        let mut covered = vec![false; shape.payloads.len()];
        let mut resolved = Vec::with_capacity(arms.len());
        for arm in arms {
            let ctor = &arm.ctor.name;
            let info = self.cases_arm_constructor(&arm.ctor, scrutinee_ty)?;
            let payload = shape.payloads.get(info.index);
            // The arms agree with each other, and their owner with the
            // scrutinee (ADR 4.10.26d D3): an index means nothing outside
            // the type that assigned it. A disjunction's `inl`/`inr` has no
            // owner: its index is its side.
            if let Some(info_owner) = &info.owner {
                let owner = type_name.get_or_insert_with(|| info_owner.clone());
                if owner != info_owner
                    || payload.is_none()
                    || !self.cases_owner_admits(owner, shape)
                {
                    return Err(format!("`{ctor}` is not a constructor of `{scrutinee_ty}`"));
                }
            }
            if arm.binders.len() != info.arity {
                return Err(format!(
                    "constructor `{ctor}` has {} field(s), the arm binds {}",
                    info.arity,
                    arm.binders.len()
                ));
            }
            if std::mem::replace(&mut covered[info.index], true) {
                return Err(format!("`{ctor}` has two arms"));
            }
            let types = payload
                .and_then(|payload| field_types(payload, info.arity))
                .ok_or_else(|| format!("`{ctor}` is not a constructor of `{scrutinee_ty}`"))?;
            let names = self.cases_field_names(variable, &arm.binders, info.index, scope)?;
            resolved.push(ResolvedArm {
                index: info.index,
                fields: names.into_iter().zip(types).collect(),
            });
        }
        if let Some(missing) = covered.iter().position(|seen| !seen) {
            return Err(format!(
                "no arm for `{}`",
                self.constructor_name(type_name.as_deref(), missing)
            ));
        }
        Ok(resolved)
    }

    /// The constructor an arm names. A constructor in scope is resolved
    /// first, so every user type splits as before and a user `inl` wins;
    /// only when none is found, and the scrutinee is a disjunction, do `inl`
    /// and `inr` name its sides (ADR 7.10.26l D3).
    fn cases_arm_constructor(
        &self,
        ctor: &Ident,
        scrutinee_ty: &Type,
    ) -> Result<ArmConstructor, String> {
        let found = self
            .env
            .resolve_constructor_path(&Path::simple(ctor.clone()), &self.current_module)
            .ok()
            .flatten();
        if let Some(info) = found {
            return Ok(ArmConstructor {
                owner: Some(info.type_name.clone()),
                index: info.index,
                arity: info.arity,
            });
        }
        let side = Side::of_name(&ctor.name)
            .filter(|_| is_disjunction(&self.normalize_for_comparison(scrutinee_ty)));
        match side {
            Some(side) => Ok(ArmConstructor {
                owner: None,
                index: usize::from(side == Side::Right),
                arity: 1,
            }),
            None => Err(format!("no constructor `{}`", ctor.name)),
        }
    }

    /// Whether the split's payloads are `owner`'s encoding — D1(b) over the
    /// shape `cases` already read. An owner with no constructor list (a
    /// stub) is not read.
    fn cases_owner_admits(&self, owner: &str, shape: &SplitShape) -> bool {
        let Some(TypeDefKind::ADT(ctors)) = self.env.lookup_type(owner).map(|def| &def.kind) else {
            return true;
        };
        ctors.len() == shape.payloads.len()
            && ctors
                .iter()
                .zip(&shape.payloads)
                .all(|(ctor, payload)| ownership::payload_admits(&ctor.fields, payload))
    }

    /// The name of constructor `index` of `type_name`, for a missing arm. No
    /// owner means every arm was a disjunction's, so the name is its side.
    fn constructor_name(&self, type_name: Option<&str>, index: usize) -> String {
        if type_name.is_none() {
            return ["inl", "inr"].get(index).map_or_else(
                || format!("constructor {index}"),
                |side| (*side).to_string(),
            );
        }
        type_name
            .and_then(|name| self.env.lookup_type(name))
            .and_then(|def| match &def.kind {
                TypeDefKind::ADT(ctors) => ctors.iter().find(|c| c.index == index),
                _ => None,
            })
            .map_or_else(|| format!("constructor {index}"), |ctor| ctor.name.clone())
    }

    /// An arm's field names: each user binder, checked fresh (3.10.26o D1's
    /// no-capture rule, refused here with the name), and a synthesised name
    /// for each `_`. Every free name of the goal is a parameter or a local,
    /// and a global it cites is bound in the kernel's context too, so a field
    /// may be none of the three. A pair arm's two names take the same rule
    /// (ADR 6.10.26e D6).
    fn cases_field_names(
        &self,
        variable: &str,
        binders: &[Option<Ident>],
        index: usize,
        scope: GoalScope<'_>,
    ) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = Vec::with_capacity(binders.len());
        for (position, binder) in binders.iter().enumerate() {
            let Some(binder) = binder else {
                names.push(format!(
                    "{CASES_NAME_PREFIX}{}_{index}_{position}",
                    scope.id
                ));
                continue;
            };
            let name = &binder.name;
            let bound = self.env.lookup_local(name).is_some()
                || scope.locals.iter().any(|(local, _)| local == name)
                || self.env.has_value(name);
            if name == variable || bound {
                return Err(format!("`{name}` is already bound"));
            }
            if names.contains(name) {
                return Err(format!("`{name}` is bound twice in one arm"));
            }
            names.push(name.clone());
        }
        Ok(names)
    }

    /// Run each arm's tactics on its own goal — the holes `run_cases` solved
    /// `split` with, in the arms' order — and refuse an arm that leaves its
    /// goal open (E0083, at the arm). The goals after the split's are
    /// restored once every arm is closed. A poisoned split has no hole, and
    /// its arms do not run.
    pub(super) fn run_cases_arms(
        &mut self,
        arms: &[CasesArm],
        split: MetaId,
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let bodies: Vec<(&[Tactic], Span)> = arms
            .iter()
            .map(|arm| (arm.body.as_slice(), arm.span))
            .collect();
        self.run_split_arms(&bodies, split, state, steps)
    }

    /// [`Elaborator::run_cases_arms`] over each arm's tactics and the span an
    /// open goal is reported at: a pair arm's is its whole step (ADR
    /// 6.10.26e D6).
    pub(super) fn run_split_arms(
        &mut self,
        arms: &[(&[Tactic], Span)],
        split: MetaId,
        state: &mut ProofState,
        steps: &mut Vec<TacticStep>,
    ) -> ElabResult<()> {
        let arm_goals: Vec<MetaId> = state.metas[split]
            .solution
            .as_ref()
            .map(|partial| partial.holes.iter().map(|(_, meta)| *meta).collect())
            .unwrap_or_default();
        let rest: Vec<MetaId> = state
            .goals
            .iter()
            .copied()
            .filter(|goal| !arm_goals.contains(goal))
            .collect();
        for ((body, span), goal) in arms.iter().zip(arm_goals) {
            state.goals = vec![goal];
            self.run_tactics(body, state, steps)?;
            if !state.goals.is_empty() {
                let goals = state
                    .goals
                    .iter()
                    .map(|open| {
                        let meta = &state.metas[*open];
                        (meta.locals.clone(), meta.target.clone())
                    })
                    .collect();
                return Err(ElabError::new(
                    *span,
                    ElabErrorKind::Tactic(TacticErrorKind::UnsolvedGoals { goals }),
                ));
            }
        }
        state.goals = rest;
        Ok(())
    }
}
