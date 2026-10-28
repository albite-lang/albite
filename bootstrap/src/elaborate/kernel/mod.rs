//! The kernel boundary: every tactic proof goes through `check_proof`
//! (ADR 2.10.26c D5–D8).
//!
//! `check_proof` needs every cited global bound with its admission marker,
//! and admission is the termination report's verdict over the whole project,
//! which does not exist while one definition elaborates. So
//! `elaborate_theorem` records a [`KernelObligation`], and:
//!
//! - the module-tree walk — the one caller of [`set_defer_kernel`] — carries
//!   it to `admit_or_reject`, which calls [`discharge`] on the driver's
//!   caller thread after the conversion discharge;
//! - every other path discharges it at the end of its body pass
//!   ([`discharge_local`]), before its output exists.
//!
//! [`discharge`] reads no enforcement level (2.10.26b D1): a kernel refusal is
//! a hard error (E0081) at every level.
//!
//! Since ADR 2.10.26d every term-mode `theorem` and `lemma` records a
//! [`ObligationOrigin::Term`] obligation too. Its refusal is classified with
//! the census's own classifier and probes, matched against the exemption
//! register ([`Register`]), and decided by [`decide`]; only the census's
//! reporting window ([`KernelMode::Report`]) turns that gate off.
//!
//! [`set_defer_kernel`]: crate::elaborate::CollectedElaborator::set_defer_kernel

mod attribution;
mod census;
mod context;
mod gate;
mod presentation;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use albite_core::terms::termination::AdmissionState;
use albite_core::{check_proof, Context, Term, Type, TypeError};

use gate::TermChecks;
pub(crate) use gate::{term_verdicts, TermGate};

use crate::elaborate::tactics::state::MetaId;
use crate::elaborate::tactics::{TacticStep, TacticTrace};
use crate::elaborate::termination::TerminationInput;
use crate::elaborate::{CoreDef, DefTerminationMeta, ElabError, ElabErrorKind};
use crate::span::Span;

pub(crate) use attribution::{attribute, Culprit, WHOLE_BLOCK};
pub(crate) use census::delta_candidates;
pub use census::{
    classify, mismatch_pair, register_class, Cause, CensusTally, ElaboratedProbes, Probes,
    Refinement,
};
pub use context::{
    bind_globals, bind_with_bodies, decreasing_hints, delta_admission, mentioned_globals,
    present_definitions, DecreasingHints,
};
pub use gate::{
    decide, register_key, repository_root, ExemptTally, Exemption, GateVerdict, KernelMode,
    KernelOutcome, Register, RegisterClass, Violation,
};
pub use presentation::{RecordPresentation, Unpresented};

/// The prefix of an abstracted `sorry`'s hypothesis: like a hole's, impossible
/// as a source name (D8, P0.4).
pub(crate) const HYPOTHESIS_PREFIX: &str = "%hyp";

/// Where the proof came from — how a refusal names its culprit.
#[derive(Debug, Clone, PartialEq)]
pub enum ObligationOrigin {
    Tactics(TacticTrace),
    /// A term-mode `theorem` or `lemma` (ADR 2.10.26d D1, D2).
    Term,
}

/// A discharge's errors and the theorems it exempted by class (D11).
#[derive(Debug, Default)]
pub(crate) struct KernelDischarge {
    pub(crate) errors: Vec<ElabError>,
    pub(crate) exempt: ExemptTally,
}

/// A proof the kernel must check once admission is known (D5): the
/// definition it belongs to, named by name, span **and** defining module, so
/// a span that coincides across files cannot link it to the wrong one.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelObligation {
    pub def: String,
    pub span: Span,
    pub module: Option<PathBuf>,
    pub origin: ObligationOrigin,
}

/// One definition the discharge may link an obligation to or bind as a
/// global: the definition and the source file that defines it. `None` is a
/// pass that does not know its definitions' files — a single body pass, one
/// combined AST — and links on name and span alone; two matches still refuse.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DefinitionSite<'d> {
    pub(crate) module: Option<&'d Path>,
    pub(crate) def: &'d CoreDef,
}

impl crate::elaborate::CollectedElaborator<'_> {
    /// Leave kernel obligations on the output for the admission boundary
    /// instead of discharging them at the end of the pass (D5). Only the
    /// per-module Body Elaboration walk calls this.
    pub fn set_defer_kernel(&mut self, enabled: bool) {
        self.elaborator.defer_kernel = enabled;
    }
}

/// Check every obligation against `defs` and `admission`; one E0081 per
/// refusal, in obligation order. Takes no enforcement level: a refusal is
/// never a warning (Q4).
pub(crate) fn discharge(
    defs: &[DefinitionSite<'_>],
    admission: &BTreeMap<String, AdmissionState>,
    obligations: Vec<KernelObligation>,
) -> Vec<ElabError> {
    discharge_gated(defs, admission, obligations, &TermGate::LOCAL).errors
}

/// [`discharge`], with `Term` obligations checked against `gate`.
pub(crate) fn discharge_gated(
    defs: &[DefinitionSite<'_>],
    admission: &BTreeMap<String, AdmissionState>,
    obligations: Vec<KernelObligation>,
    gate: &TermGate<'_>,
) -> KernelDischarge {
    let presented = present_definitions(defs.iter().map(|site| site.def), gate.presentation);
    let defs: Vec<DefinitionSite<'_>> = defs
        .iter()
        .zip(&presented)
        .map(|(site, def)| DefinitionSite {
            module: site.module,
            def,
        })
        .collect();
    let defs = &defs[..];
    let defs_by_name: HashMap<&str, &Type> = defs
        .iter()
        .map(|site| (site.def.name.as_str(), &site.def.ty))
        .collect();
    let mut terms = TermChecks::new(defs, admission, gate);
    let mut out = KernelDischarge::default();
    for (index, obligation) in obligations.iter().enumerate() {
        // A file reached by two import paths is elaborated once per path, so
        // its obligations arrive twice (ADR 9.10.26p D3). An obligation equal
        // to an earlier one — whole value, trace included — has an equal
        // verdict, so checking it again only duplicates the error and the tally.
        if obligations[..index].contains(obligation) {
            continue;
        }
        match discharge_one(defs, &defs_by_name, obligation, &mut terms) {
            Ok(Some(class)) => out.exempt.record(class),
            Ok(None) => {}
            Err(error) => out.errors.push(error),
        }
    }
    out
}

/// Discharge on a path with no admission step: admission is computed over the
/// definitions this pass produced, with the census's own call. A name the
/// pass does not hold stays unbound, and the kernel refuses it — fail closed.
pub(crate) fn discharge_local(
    defs: &[CoreDef],
    termination_meta: &HashMap<String, DefTerminationMeta>,
    presentation: &RecordPresentation,
    obligations: Vec<KernelObligation>,
) -> Vec<ElabError> {
    let admission = TerminationInput::from_meta(termination_meta)
        .check(defs)
        .admission;
    let sites: Vec<DefinitionSite<'_>> = defs
        .iter()
        .map(|def| DefinitionSite { module: None, def })
        .collect();
    let hints = decreasing_hints(termination_meta);
    let gate = TermGate {
        decreasing: Some(&hints),
        presentation,
        ..TermGate::LOCAL
    };
    discharge_gated(&sites, &admission, obligations, &gate).errors
}

/// One obligation: `Ok(None)` passes, `Ok(Some(class))` is exempt by the
/// register, `Err` is the diagnostic.
fn discharge_one(
    defs: &[DefinitionSite<'_>],
    defs_by_name: &HashMap<&str, &Type>,
    obligation: &KernelObligation,
    terms: &mut TermChecks<'_>,
) -> Result<Option<RegisterClass>, ElabError> {
    // A scan, not a map by name: a later duplicate must not overwrite an
    // earlier one, and two matches are as much a refusal as none — unless
    // every match names the obligation's own file (below).
    let matching: Vec<&DefinitionSite<'_>> = defs
        .iter()
        .filter(|site| {
            site.def.name == obligation.def
                && site.def.span == obligation.span
                && site
                    .module
                    .map_or(true, |module| Some(module) == obligation.module.as_deref())
        })
        .collect();
    match matching[..] {
        [site] => check_definition(site.def, defs_by_name, obligation, terms),
        // A file-stamped match equals the obligation's file, so these are the
        // copies one file contributes, once per import path (ADR 9.10.26p D2).
        [_, _, ..] if matching.iter().all(|site| site.module.is_some()) => {
            let copies: Vec<&CoreDef> = matching.iter().map(|site| site.def).collect();
            check_copies(&copies, defs_by_name, obligation, terms)
        }
        _ => Err(refusal(
            obligation,
            None,
            "the proof's definition could not be identified".to_string(),
        )),
    }
}

/// D2: each copy is checked, and the copies must agree — on `ty`, and on the
/// verdict (pass, the same exemption class, or refused). The copies are
/// checked in a context keyed by name, where the last copy wins, so per-copy
/// checking is sound only because they agree; and agreement makes the result
/// independent of the walk's order. A refusal is the first copy's.
fn check_copies(
    copies: &[&CoreDef],
    defs_by_name: &HashMap<&str, &Type>,
    obligation: &KernelObligation,
    terms: &mut TermChecks<'_>,
) -> Result<Option<RegisterClass>, ElabError> {
    let same_ty = copies.iter().all(|copy| copy.ty == copies[0].ty);
    let verdicts: Vec<_> = copies
        .iter()
        .map(|copy| check_definition(copy, defs_by_name, obligation, terms))
        .collect();
    if !same_ty || !verdicts_agree(&verdicts) {
        return Err(refusal(obligation, None, COPIES_DISAGREE.to_string()));
    }
    verdicts.into_iter().next().expect("two or more copies")
}

/// Whether every verdict is the first's: the same pass or exemption class,
/// or a refusal beside a refusal.
fn verdicts_agree(verdicts: &[Result<Option<RegisterClass>, ElabError>]) -> bool {
    verdicts
        .iter()
        .all(|verdict| match (&verdicts[0], verdict) {
            (Ok(class), Ok(other)) => class == other,
            (Err(_), Err(_)) => true,
            _ => false,
        })
}

/// The cause of [`check_copies`]' refusal.
const COPIES_DISAGREE: &str = "copies of one definition disagree";

/// The kernel's verdict on the one definition an obligation linked to.
fn check_definition(
    def: &CoreDef,
    defs_by_name: &HashMap<&str, &Type>,
    obligation: &KernelObligation,
    terms: &mut TermChecks<'_>,
) -> Result<Option<RegisterClass>, ElabError> {
    match &obligation.origin {
        ObligationOrigin::Tactics(trace) => check_tactic_proof(def, trace, defs_by_name, terms)
            .map_or(Ok(None), |(culprit, cause)| {
                Err(refusal(obligation, Some(culprit), cause))
            }),
        ObligationOrigin::Term => terms.check(def, defs_by_name, obligation),
    }
}

/// The kernel's verdict on one tactic proof: `None` when admitted, or the
/// culprit and the rendered refusal.
fn check_tactic_proof(
    def: &CoreDef,
    trace: &TacticTrace,
    defs_by_name: &HashMap<&str, &Type>,
    terms: &mut TermChecks<'_>,
) -> Option<(Culprit, String)> {
    let (overrides, hypotheses) = abstract_sorries(trace);
    // With nothing to abstract, the definition's own term is what is checked.
    // `def` is presented already; an assembled proof is presented here.
    let term = if overrides.is_empty() {
        def.term.term.clone()
    } else {
        match trace.assemble(&overrides) {
            Ok(term) => terms.presentation().present_elaborated_term(&term),
            Err(_) => return Some((block_culprit(trace), "the proof is incomplete".to_string())),
        }
    };
    let mut mentioned = mentioned_globals(&term, &def.ty);
    mentioned.remove(&def.name);
    let mut base = terms.bind(&mentioned, defs_by_name);
    for name in &trace.type_params {
        base = base.with_type_var(name.clone());
    }
    for (name, ty) in &hypotheses {
        base = base.with_term(
            name.clone(),
            terms.presentation().present_elaborated_type(ty),
        );
    }
    let error = check_proof(&base, &term, &def.ty).err()?;
    // A step-less origin keeps the plain exemption (the 2.10.26d hand-off).
    if trace.steps.is_empty() && error == TypeError::SorryInProof {
        return None;
    }
    let culprit = attribute_refusal(trace, &base, &overrides, terms.presentation());
    Some((culprit, error.to_string()))
}

/// D8's Holes rule: each `exact` whose term contains `sorry` is replaced by
/// `h_i p₁ … p_m l₁ … l_k` — the theorem's value parameters, then every
/// `intro` local in scope at that goal — with `h_i` bound to
/// `Π(params)(locals). target`. The rest still meets every gate.
pub(crate) fn abstract_sorries(
    trace: &TacticTrace,
) -> (BTreeMap<MetaId, Term>, Vec<(String, Type)>) {
    let mut overrides = BTreeMap::new();
    let mut hypotheses = Vec::new();
    for step in trace.steps.iter().filter(|step| step.exact_sorry) {
        let name = format!("{HYPOTHESIS_PREFIX}{}", hypotheses.len());
        let meta = &trace.state.metas[step.goal];
        let binders: Vec<&(String, Type)> = trace.value_params.iter().chain(&meta.locals).collect();
        let mut hypothesis_ty = meta.target.clone();
        for (binder, ty) in binders.iter().rev() {
            hypothesis_ty = Type::pi_or_arrow(binder.clone(), ty.clone(), hypothesis_ty);
        }
        let mut applied = Term::var(name.clone());
        for (binder, _) in &binders {
            applied = Term::app(applied, Term::var(binder.clone()));
        }
        overrides.insert(step.goal, applied);
        hypotheses.push((name, hypothesis_ty));
    }
    (overrides, hypotheses)
}

/// Check each step's contribution against its own goal, under the theorem's
/// parameters and that goal's locals, and name a minimal refused one.
fn attribute_refusal(
    trace: &TacticTrace,
    base: &Context,
    overrides: &BTreeMap<MetaId, Term>,
    presentation: &RecordPresentation,
) -> Culprit {
    let culprit_names: Vec<String> = trace.steps.iter().map(TacticStep::culprit_name).collect();
    let named: Vec<(&str, Span)> = culprit_names
        .iter()
        .zip(&trace.steps)
        .map(|(name, step)| (name.as_str(), step.span))
        .collect();
    attribute(&named, trace.block_span, |index| {
        let goal = trace.steps[index].goal;
        let meta = &trace.state.metas[goal];
        let Ok(contribution) = trace.state.zonk_with(goal, overrides) else {
            return true;
        };
        let mut ctx = base.clone();
        for (name, ty) in trace.value_params.iter().chain(&meta.locals) {
            ctx = ctx.with_term(name.clone(), presentation.present_elaborated_type(ty));
        }
        let contribution = presentation.present_elaborated_term(&contribution);
        let target = presentation.present_elaborated_type(&meta.target);
        check_proof(&ctx, &contribution, &target).is_err()
    })
}

fn block_culprit(trace: &TacticTrace) -> Culprit {
    attribute(&[], trace.block_span, |_| false)
}

/// E0081 for `obligation`, at the culprit's span when there is one.
fn refusal(obligation: &KernelObligation, culprit: Option<Culprit>, cause: String) -> ElabError {
    let span = culprit.as_ref().map_or(obligation.span, |c| c.span);
    let error = ElabError::new(
        span,
        ElabErrorKind::KernelRefusedProof {
            def: obligation.def.clone(),
            tactic: culprit.map(|c| (c.tactic, c.span)),
            cause,
        },
    );
    attach_file(error, obligation)
}

fn attach_file(error: ElabError, obligation: &KernelObligation) -> ElabError {
    match &obligation.module {
        Some(path) => error.with_file_path(path.clone()),
        None => error,
    }
}
