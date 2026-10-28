//! The kernel's soundness gates (ADR 29.9.26e §2.1).
//!
//! Each gate is a pure function over its inputs and reuses an existing
//! engine rather than re-deriving it, so the kernel and the elaborator cannot
//! disagree about what they refuse:
//!
//! - [`proof_prepass`] — D5, D9, D10: no `return`, `sorry` or poisoned type
//!   anywhere in a proof term, its embedded types, the witness terms inside
//!   those types, or its context's hypotheses.
//! - [`mu_positivity`] — D6: the strict-positivity engine
//!   (`types::positivity`), over a `μ` presented as a one-constructor
//!   definition. Runs in both modes.
//! - [`fix_admission`] — D3: the termination engine
//!   (`terms::termination::analyze`), over a `fix` presented as a
//!   one-definition set. Proof mode only.

use std::collections::{BTreeMap, BTreeSet};

use crate::context::Context;
use crate::terms::termination::{analyze, AdmissionState, DefRole, DefView, TerminationAnnotation};
use crate::terms::Term;
use crate::types::positivity::{
    check_strict_positivity, param_occurrences, PositivityCtor, PositivityDef, PositivityDefs,
};
use crate::types::Type;

use super::error::TypeError;
use super::TypeResult;

// ============================================================================
// D5 / D9 / D10 — the proof pre-pass
// ============================================================================

/// Refuse a proof that carries a `return`, a `sorry` or a poisoned type.
///
/// A pre-pass rather than a rule check, so these refusals cannot depend on
/// the proof flag reaching the sub-derivation that holds the node.
pub(super) fn proof_prepass(ctx: &Context, term: &Term) -> TypeResult<()> {
    for (_, hypothesis) in ctx.term_vars() {
        type_admissible_in_proof(hypothesis)?;
    }
    term_admissible_in_proof(term)
}

fn term_admissible_in_proof(term: &Term) -> TypeResult<()> {
    match term {
        Term::Return(_) => return Err(TypeError::ReturnInProof),
        Term::Region(..) => return Err(TypeError::RegionInProof),
        Term::CallFrame(_) => return Err(TypeError::CallFrameInProof),
        Term::Sorry => return Err(TypeError::SorryInProof),
        _ => {}
    }
    let mut verdict = Ok(());
    term.for_each_embedded_type(|ty| {
        if verdict.is_ok() {
            verdict = type_admissible_in_proof(ty);
        }
    });
    term.for_each_subterm(|sub| {
        if verdict.is_ok() {
            verdict = term_admissible_in_proof(sub);
        }
    });
    verdict
}

/// No poison in the type, and nothing inadmissible in the witness terms any
/// `Eq` inside it carries.
pub(super) fn type_admissible_in_proof(ty: &Type) -> TypeResult<()> {
    if ty.contains_poison() {
        return Err(TypeError::PoisonInProof { ty: ty.clone() });
    }
    eq_witnesses_admissible(ty)
}

fn eq_witnesses_admissible(ty: &Type) -> TypeResult<()> {
    if let Type::Eq(_, left, right) = ty {
        term_admissible_in_proof(left)?;
        term_admissible_in_proof(right)?;
    }
    // A predicate's arguments are screened as `Eq`'s are (ADR 6.10.26h D6)
    if let Type::PredApp(_, args) = ty {
        args.iter().try_for_each(term_admissible_in_proof)?;
    }
    ty.children()
        .into_iter()
        .try_for_each(eq_witnesses_admissible)
}

// ============================================================================
// D6 — strict positivity at every fold/unfold
// ============================================================================

/// Refuse a `μ` whose variable occurs non-strictly positively.
///
/// `μα. τ` is presented as one positional definition named by a fresh `g`,
/// whose one field is `τ` with every spelling of `α` replaced by `g`, and
/// checked as the singleton group `{g}` — the same predicate and the same
/// parameter-occurrence fixpoint the elaborator's gate uses. Anything that is
/// not a `μ` passes: the rules report a malformed one themselves.
pub(super) fn mu_positivity(mu: &Type) -> TypeResult<()> {
    let Type::Mu(binder, body) = mu else {
        return Ok(());
    };
    let member = fresh_type_name(binder, body);
    let field = present_mu_field(binder, body, &member);
    let definition = PositivityDef {
        params: Vec::new(),
        ctors: vec![PositivityCtor::positional("fold", vec![field.clone()])],
        is_record: false,
    };
    let defs = PositivityDefs::new(
        BTreeMap::from([(member.clone(), definition)]),
        &BTreeMap::new(),
        BTreeSet::new(),
    );
    let occs = param_occurrences(&defs);
    if check_strict_positivity(&BTreeSet::from([member]), &defs, &occs).is_empty() {
        Ok(())
    } else {
        Err(TypeError::NonStrictlyPositiveMu {
            mu: mu.clone(),
            field: field.to_string(),
        })
    }
}

/// `body` with every spelling of `binder` the executable equality identifies
/// with it — itself, and its `α_`- and `@`-stripped forms — replaced by
/// `member`. Missing one would present `μα_B. B → Void` as `B → Void`, where
/// positivity sees no occurrence at all (ADR 29.9.26e §1.4, route b).
fn present_mu_field(binder: &str, body: &Type, member: &str) -> Type {
    let bare = binder.strip_prefix("α_").unwrap_or(binder);
    let spellings = [binder.to_string(), bare.to_string(), format!("@{bare}")];
    let replacement = Type::TyVar(member.to_string());
    spellings.iter().fold(body.clone(), |field, spelling| {
        field.substitute(spelling, &replacement)
    })
}

/// `mu#<binder>`, suffixed until no name in `body` spells it.
fn fresh_type_name(binder: &str, body: &Type) -> String {
    let base = format!("mu#{binder}");
    let mut candidate = base.clone();
    let mut suffix = 0;
    while type_mentions_name(body, &candidate) {
        suffix += 1;
        candidate = format!("{base}#{suffix}");
    }
    candidate
}

fn type_mentions_name(ty: &Type, name: &str) -> bool {
    let here = match ty {
        Type::TyVar(v)
        | Type::Mu(v, _)
        | Type::Forall(v, _)
        | Type::App(v, _)
        | Type::Adt(v, _, _) => v == name,
        _ => false,
    };
    here || ty
        .children()
        .into_iter()
        .any(|child| type_mentions_name(child, name))
}

// ============================================================================
// D3 — a proof's fixpoint must be certified total
// ============================================================================

/// Refuse `fix name: ty. body` unless the termination engine certifies it
/// `Total`.
///
/// The fixpoint is presented as the one-definition set `{g}`, with `g` fresh
/// among `body`'s globals and `body[name := Global(g)]` wrapped in one
/// `TyAbs` per type variable in scope. Those wrappers are load-bearing: the
/// engine treats a bare type variable as a nominal descent root unless the
/// definition binds it, so without them a fixpoint could "descend" on an
/// abstract `a`.
///
/// `decreasing` is the definition's `#[decreasing(arg)]`, a hint the engine
/// re-checks rather than trusts. The result is the certified decreasing
/// position — an index into the term parameters — which δ's `fix` rule
/// guards its unrolling on (ADR 2.10.26l D4).
pub(super) fn fix_admission(
    name: &str,
    ty: &Type,
    body: &Term,
    type_params: &[String],
    decreasing: Option<&str>,
) -> TypeResult<usize> {
    let global = fresh_global_name(name, body);
    let recursive = body.substitute(name, &Term::Global(global.clone()));
    let presented = type_params.iter().rev().fold(recursive, |inner, param| {
        Term::TyAbs(param.clone(), Box::new(inner))
    });
    let annotation = TerminationAnnotation {
        partial: false,
        decreasing: decreasing.map(str::to_string),
    };
    let view = DefView {
        ty,
        term: &presented,
        annotation: &annotation,
        role: DefRole::Proof,
    };
    let report = analyze(&BTreeMap::from([(global.clone(), view)]));
    if report.admission.get(&global) == Some(&AdmissionState::Total) {
        // A `Total` fixpoint that never calls itself has no group and so no
        // recorded position; it never needs to unroll, and position 0 is as
        // good as any — its body names no fixpoint to unroll into.
        return Ok(report.decreasing.get(&global).copied().unwrap_or(0));
    }
    let reason = report
        .failures
        .iter()
        .map(|failure| format!("{:?}", failure.reason))
        .collect::<Vec<_>>()
        .join("; ");
    Err(TypeError::FixNotStructural {
        name: name.to_string(),
        reason,
    })
}

/// `fix#<name>`, suffixed until no `Global` in `body` spells it.
fn fresh_global_name(name: &str, body: &Term) -> String {
    let base = format!("fix#{name}");
    let mut candidate = base.clone();
    let mut suffix = 0;
    while term_mentions_global(body, &candidate) {
        suffix += 1;
        candidate = format!("{base}#{suffix}");
    }
    candidate
}

/// Whether a `Global(name)` node occurs anywhere in `term`.
pub(super) fn term_mentions_global(term: &Term, name: &str) -> bool {
    if matches!(term, Term::Global(global) if global == name) {
        return true;
    }
    let mut found = false;
    term.for_each_subterm(|sub| found = found || term_mentions_global(sub, name));
    found
}

#[cfg(test)]
mod tests;
