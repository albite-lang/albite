//! `Mode::Run` evaluation: find `main`, evaluate it on the environment
//! evaluator, and report the outcome.
//!
//! A black hole (ADR 22.7.26a — a global re-entering its own forcing) is a
//! diagnostic plus a failing exit here, where pre-detection it was a
//! stack-overflow abort. Split from `pipeline.rs` to keep that file under the
//! size limit.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use albite_core::{
    eval::{eval_with_env, eval_with_env_and_limit, EvalStopped},
    Term,
};

use crate::elaborate::ElabOutput;

use crate::driver::diagnostics::json::{self, DiagnosticRecord};
use crate::driver::diagnostics::{emit_record, error_format, ErrorFormat};
use crate::driver::{render_diagnostics, PipelineResult};

/// Evaluate `main` for `albite run`, or report the no-main error (E0030).
pub(super) fn evaluate_main(output: &ElabOutput, source: &str, filename: &str) -> PipelineResult {
    let Some(main_def) = output.defs.iter().find(|d| d.name == "main") else {
        // Point to end of file since we don't have a better location.
        let eof_span = crate::span::Span::new(source.len() as u32, source.len() as u32);
        let err = crate::ElabError::no_main_function(eof_span);
        render_diagnostics(source, filename, &[err], &[]);
        return PipelineResult::Failed;
    };

    // Build globals map (excluding main) for environment-based evaluation.
    // This avoids exponential term blowup from naive substitution. The env
    // also carries the lazy `__cmp<T>` synthesis callback (ADR 29.6.26f
    // §T11.2a / P6′) so the generic `compare` intrinsic resolves.
    let mut globals = crate::comparator::eval::defs_as_globals(&output.defs);
    globals.remove("main");
    let static_defs: HashSet<String> = globals.keys().cloned().collect();
    let types = crate::comparator::ComparatorTypes::new(
        output.record_types.clone(),
        &output.encoded_types,
        &output.type_provenance,
        output.adt_types.clone(),
        &output.mutual_recursion_groups,
    );
    let env = crate::comparator::eval::eval_env(globals, &types);
    match eval_with_env(&main_def.term.term, &env) {
        Ok(value) => {
            // The small-step loop stops when no rule applies and hands back the
            // residual term. Printed bare that is a wall of Core IR in which the
            // actual cause is one token among thousands, so name it first.
            // Comparators are synthesized lazily into the env, so the static
            // def list alone would report every one of them "never resolved"
            // (ADR 1.8.26b).
            let mut defined = static_defs;
            defined.extend(env.registered_comparators());
            if let Some(diagnosis) = stuck_diagnosis(&value, &defined) {
                emit_run_report(stuck_report(&diagnosis, error_format()));
            }
            PipelineResult::Evaluated {
                value,
                ty: main_def.ty.clone(),
            }
        }
        // The program has no value: a black hole (ADR 22.7.26a) or a
        // comparison that never ran (ADR 1.8.26b D3). Both used to surface as
        // an ordinary residual, which `run` printed with a warning and a
        // success exit.
        Err(stopped) => {
            emit_run_report(stopped_report(&stopped, error_format()));
            PipelineResult::Failed
        }
    }
}

/// Evaluate `term` against `output`'s globals under a step bound.
///
/// The globals exclude `main` (evaluating it is the job at hand, not a
/// lookup), and the env carries the lazy `__cmp<T>` synthesis callback so the
/// generic `compare` intrinsic resolves — the same wiring `albite run` uses.
///
/// Moved here from the playground by ADR 5.10.26d D3.2, so the playground and
/// `fuzz-test`'s `run-never-stuck` property share one bounded entry point. It
/// touches no clock (`Instant` panics on `wasm32-unknown-unknown`), and it is
/// never unbounded: a diverging program stops at `step_limit`.
pub fn bounded_eval(
    term: &Term,
    output: &ElabOutput,
    step_limit: usize,
) -> Result<Term, EvalStopped> {
    let mut globals = crate::comparator::eval::defs_as_globals(&output.defs);
    globals.remove("main");
    let types = crate::comparator::ComparatorTypes::new(
        output.record_types.clone(),
        &output.encoded_types,
        &output.type_provenance,
        output.adt_types.clone(),
        &output.mutual_recursion_groups,
    );
    let env = crate::comparator::eval::eval_env(globals, &types);
    eval_with_env_and_limit(term, &env, step_limit)
}

/// The warning line over a stuck residual.
const STUCK_MESSAGE: &str =
    "evaluation got stuck — the result below is a residual term, not a value";

/// What `run` writes to stderr for one runtime outcome: human lines, or one
/// schema-1 record in JSON mode (ADR 29.9.26f D9).
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // Reason: one short-lived value per run; boxing touches every construction site (ADR 1.10.26j D5)
pub(super) enum RunReport {
    Lines(Vec<String>),
    Record(DiagnosticRecord),
}

/// The report for a stopped evaluation: `error` record, `code: null`.
pub(super) fn stopped_report(stopped: &EvalStopped, format: ErrorFormat) -> RunReport {
    let note = stopped_note(stopped);
    match format {
        ErrorFormat::Json => RunReport::Record(json::stopped_record(&stopped.to_string(), note)),
        ErrorFormat::Human => RunReport::Lines(
            std::iter::once(format!("error: {stopped}"))
                .chain(note.map(|note| format!("  = note: {note}")))
                .collect(),
        ),
    }
}

/// The report for a stuck residual: `warning` record, `code: null`.
pub(super) fn stuck_report(diagnosis: &str, format: ErrorFormat) -> RunReport {
    match format {
        ErrorFormat::Json => RunReport::Record(json::stuck_record(STUCK_MESSAGE, diagnosis)),
        ErrorFormat::Human => RunReport::Lines(vec![
            format!("warning: {STUCK_MESSAGE}"),
            format!("  = stuck at: {diagnosis}"),
        ]),
    }
}

fn emit_run_report(report: RunReport) {
    match report {
        RunReport::Lines(lines) => lines.iter().for_each(|line| eprintln!("{line}")),
        RunReport::Record(record) => emit_record(&record),
    }
}

/// The follow-up line under a stopped evaluation — the reader's next move,
/// which differs per stop. `None` where the `Display` already says everything.
pub(super) fn stopped_note(stopped: &EvalStopped) -> Option<&'static str> {
    match stopped {
        EvalStopped::BlackHole { .. } => Some("the definition re-enters itself while being forced"),
        EvalStopped::Uncomparable(_) => Some(
            "run `albite doctor check comparable <T> <file>` (cost 3) to see \
             where the comparator breaks",
        ),
        EvalStopped::ComparisonNeverRan { .. } => Some(
            "the assertion consumed a comparison that never produced a result, \
             so it asserted nothing (ADR 1.8.26b)",
        ),
        EvalStopped::MalformedElimination { .. } => Some("see docs/repo-memory/adt-abi-safety.md"),
        // The `Display` line IS the native program's stderr line (ADR
        // 14.9.26c AC 3): nothing may follow it, or the two sides diverge.
        EvalStopped::IntTrap { .. } => None,
        EvalStopped::UnwrittenByteRead { .. } => Some(
            "the native core read a byte its write-cover rule should have proved \
             written (ADR 9.10.26n D3)",
        ),
        EvalStopped::TimedOut { .. } | EvalStopped::StepLimit { .. } => None,
    }
}

/// One-line explanation of why an evaluated term is not a value, or `None`
/// when it is one.
///
/// A **projection applied to a non-pair** is checked first, because it is
/// unconditional: `Fst` of a scalar can never step, whatever else is in scope.
/// It is what a constructor-payload nesting mismatch looks like at runtime
/// (`docs/repo-memory/adt-abi-safety.md`). Since ADR 1.8.26b D3 that shape
/// never reaches here on the `run` path — the evaluator records it as
/// `EvalStopped::MalformedElimination` and the run fails before a residual is
/// printed — so this arm is a **backstop** for any future entry point that
/// consults a residual without that recording, and the unit tests below are
/// what keep it honest.
///
/// A **`Global` absent from `defined`** is checked second, and only then,
/// because its absence is weaker evidence: a global sitting in a branch the
/// evaluator never reached was not "unresolved" — it was never forced. Getting
/// this order wrong blames `compare_Nat` for a residual whose actual cause is a
/// `Fst(Succ(Zero))` three nodes away, which is what the first cut did.
///
/// `defined` must include the env's **synthesized** comparators, not just the
/// static def list. Comparators are registered lazily during evaluation, so a
/// static-only set reports every one of them "never resolved" — a false lead
/// that cost ADR 1.8.26b its first D2 root-cause thesis. `evaluate_main` unions
/// them in; a caller that forgets will get confidently wrong answers.
///
/// Anything else falls back to naming the head constructor, which is still
/// more than the caller had before.
pub fn stuck_diagnosis<S: BuildHasher>(
    term: &Term,
    defined: &HashSet<String, S>,
) -> Option<String> {
    if term.is_value() {
        return None;
    }
    if let Some(projection) = first_stuck_projection(term) {
        return Some(format!(
            "`{projection}` applied to a non-pair — usually a constructor-payload nesting \
             mismatch (see docs/repo-memory/adt-abi-safety.md)"
        ));
    }
    if let Some(name) = first_undefined_global(term, defined) {
        if name.starts_with("compare_") || name == "__cmp" {
            return Some(format!(
                "comparator `{name}` never resolved — no comparator was synthesized for \
                 this type, so `compare` never ran (try `albite doctor check comparable <T> <file>`)"
            ));
        }
        return Some(format!("global `{name}` never resolved"));
    }
    Some(format!("{} (no evaluation rule applies)", head_label(term)))
}

/// Depth-first search for a `Global` the environment does not define.
fn first_undefined_global<S: BuildHasher>(
    term: &Term,
    defined: &HashSet<String, S>,
) -> Option<String> {
    if let Term::Global(name) = term {
        if !defined.contains(name) {
            return Some(name.clone());
        }
    }
    child_terms(term)
        .into_iter()
        .find_map(|child| first_undefined_global(child, defined))
}

/// Depth-first search for `Fst`/`Snd` applied to a value that is not a pair.
fn first_stuck_projection(term: &Term) -> Option<&'static str> {
    let here = match term {
        Term::Fst(inner) if inner.is_value() && !is_pair(inner) => Some("Fst"),
        Term::Snd(inner) if inner.is_value() && !is_pair(inner) => Some("Snd"),
        _ => None,
    };
    here.or_else(|| {
        child_terms(term)
            .into_iter()
            .find_map(first_stuck_projection)
    })
}

fn is_pair(term: &Term) -> bool {
    match term {
        Term::Pair(_, _) => true,
        Term::Spanned(inner, _) => is_pair(inner),
        _ => false,
    }
}

/// The term's constructor name, for the fallback message — and, public since
/// ADR 5.10.26d D3.2, the fixed class a `run-never-stuck` violation is filed
/// under (never program text, so shrinking keeps the same violation).
pub fn head_label(term: &Term) -> &'static str {
    match term {
        Term::App(_, _) => "application",
        Term::Case(_, _, _, _, _) => "case",
        Term::AdtMatch(_, _) => "match",
        Term::Fst(_) | Term::Snd(_) => "projection",
        Term::Unfold(_, _) => "unfold",
        Term::ExternCall(_, _) => "extern call",
        Term::Global(_) => "global",
        _ => "term",
    }
}

/// Immediate subterms, for the generic walks above.
///
/// Deliberately structural rather than exhaustive-by-variant: the two searches
/// only need to *reach* every subterm, and a `_ => vec![]` arm on a large enum
/// keeps this from breaking every time a variant is added.
fn child_terms(term: &Term) -> Vec<&Term> {
    match term {
        Term::App(f, a) => vec![f, a],
        Term::Pair(a, b) => vec![a, b],
        Term::Fst(t) | Term::Snd(t) => vec![t],
        Term::Inl(_, t) | Term::Inr(_, t) => vec![t],
        Term::Fold(_, t) | Term::Unfold(_, t) => vec![t],
        Term::Succ(t) => vec![t],
        Term::Spanned(t, _) => vec![t],
        Term::Annot(t, _) => vec![t],
        Term::TyApp(t, _) => vec![t],
        Term::Lambda(_, _, body) | Term::TyAbs(_, body) => vec![body],
        Term::Let(_, _, bound, body) => vec![bound, body],
        Term::Case(scrutinee, _, left, _, right) => vec![scrutinee, left, right],
        Term::If(c, t, e) => vec![c, t, e],
        Term::AdtConstruct(_, _, payload) => vec![payload],
        Term::ExternCall(_, args) => args.iter().collect(),
        Term::AdtMatch(scrutinee, arms) => {
            let mut out = vec![scrutinee.as_ref()];
            out.extend(arms.iter().map(|(_, _, body)| body.as_ref()));
            out
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod bounded_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod tests;
