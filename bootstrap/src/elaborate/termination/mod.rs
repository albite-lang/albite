//! The termination gate: admission at the trusted boundary (ADR 29.6.26e).
//!
//! The analysis itself is `albite_core::terms::termination`; this module is
//! the adapter. It builds the engine's input from an assembled definition set
//! plus the annotations the elaborator recorded, and turns the engine's
//! failures into `ElabError`s.
//!
//! **Where it runs matters.** The gate is applied to the whole project's
//! definitions, once, after every module has been elaborated — not per module
//! during Body Elaboration. That is the only point at which imported and
//! cache-reconstructed definitions are in the same set as freshly elaborated
//! ones, and admitting them by separate routes is exactly the hole the ADR
//! exists to close.
//!
//! The elaboration cache is the one route that would otherwise slip past it: a
//! signature-cache hit skips body elaboration, so the module's `CoreDef`s are
//! simply absent and a whole-project gate would certify a set that no longer
//! contains them. [`CachedTermination`] closes that — the entry carries the
//! module's verdict and the names and edges taint needs, and the gate replays
//! both.

mod cache;
mod location;
mod rollout;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};

use albite_core::terms::termination::{
    analyze_with_carried, CarriedDefs, DefRole, DefView, FailureReason, OccurrenceGraph,
    TerminationAnnotation, TerminationFailure, TerminationReport,
};
use serde::{Deserialize, Serialize};

use crate::span::Span;

use super::{CoreDef, DefTerminationMeta, ElabError, ElabErrorKind};

pub use cache::CachedTermination;
pub use location::{DefinitionSites, ProofSite};
pub use rollout::{
    enforcement, reporting_window_open, reset_enforcement, set_enforcement, Enforcement,
    ReportingOnly,
};

/// The annotations, converted once so the borrowed views can point at them.
///
/// The engine takes `&TerminationAnnotation`, and the elaborator stores
/// `DefTerminationMeta`; this owns the converted values for the duration of a
/// check.
pub struct TerminationInput {
    annotations: BTreeMap<String, TerminationAnnotation>,
    proofs: BTreeMap<String, bool>,
}

impl TerminationInput {
    /// Convert the elaborator's per-definition metadata.
    #[must_use]
    pub fn from_meta(meta: &HashMap<String, DefTerminationMeta>) -> Self {
        TerminationInput {
            annotations: meta
                .iter()
                .map(|(name, entry)| {
                    (
                        name.clone(),
                        TerminationAnnotation {
                            partial: entry.attrs.partial,
                            decreasing: entry
                                .attrs
                                .decreasing
                                .as_ref()
                                .map(|ident| ident.name.clone()),
                        },
                    )
                })
                .collect(),
            proofs: meta
                .iter()
                .map(|(name, entry)| (name.clone(), entry.is_proof))
                .collect(),
        }
    }

    /// Run the analysis over `defs`.
    ///
    /// A definition with no recorded metadata is executable and unannotated —
    /// the overwhelmingly common case, which is why the elaborator only records
    /// the exceptions.
    #[must_use]
    pub fn check(&self, defs: &[CoreDef]) -> TerminationReport {
        self.check_with_carried(defs, &CarriedDefs::default())
    }

    /// [`TerminationInput::check`], with cache-carried definitions taking part
    /// in taint propagation.
    #[must_use]
    pub fn check_with_carried(&self, defs: &[CoreDef], carried: &CarriedDefs) -> TerminationReport {
        let unannotated = TerminationAnnotation::default();
        let views: BTreeMap<String, DefView<'_>> = defs
            .iter()
            .map(|def| {
                (
                    def.name.clone(),
                    DefView {
                        ty: &def.ty,
                        term: &def.term.term,
                        annotation: self.annotations.get(&def.name).unwrap_or(&unannotated),
                        role: if self.proofs.get(&def.name).copied().unwrap_or(false) {
                            DefRole::Proof
                        } else {
                            DefRole::Executable
                        },
                    },
                )
            })
            .collect();
        analyze_with_carried(&views, carried)
    }

    /// Whether `name` is proof-relevant.
    #[must_use]
    pub fn is_proof(&self, name: &str) -> bool {
        self.proofs.get(name).copied().unwrap_or(false)
    }

    /// Whether `failure` matters to soundness rather than only to confidence:
    /// the rejected definition is itself a proof, or the reason *is* the proof
    /// boundary (`#[partial]` or foreign, ADR 3.10.26c D3).
    #[must_use]
    pub fn is_proof_relevant(&self, failure: &TerminationFailure) -> bool {
        self.is_proof(&failure.function) || failure.reason.is_proof_boundary()
    }

    /// A report's rejections rendered against `sites`, each with whether it
    /// is proof-relevant ([`TerminationInput::is_proof_relevant`]).
    #[must_use]
    pub fn rejections(
        &self,
        report: &TerminationReport,
        sites: &DefinitionSites<'_>,
    ) -> Vec<(ElabError, bool)> {
        report
            .failures
            .iter()
            .zip(admission_errors(report, sites))
            .map(|(failure, error)| (error, self.is_proof_relevant(failure)))
            .collect()
    }

    /// Split a report's rejections into the ones that fail the build and the
    /// ones that are only reported, at `level`. The accumulator splits after
    /// merging carried rejections in (ADR 3.10.26r D3), so only tests split a
    /// bare report.
    #[cfg(test)]
    #[must_use]
    pub fn partition_errors(
        &self,
        report: &TerminationReport,
        sites: &DefinitionSites<'_>,
        level: Enforcement,
    ) -> AdmissionSplit {
        AdmissionSplit::from_rejections(self.rejections(report, sites), level)
    }
}

/// Rejections split by whether they fail the build.
#[derive(Debug, Default)]
pub struct AdmissionSplit {
    /// Errors that abort elaboration.
    pub gating: Vec<ElabError>,
    /// Errors demoted to warnings at the current enforcement level.
    pub reported: Vec<ElabError>,
}

/// Whether a report plus whatever a cache carried has nothing to say.
///
/// A free function so the two halves of the condition are assertable: a report
/// that is clean says nothing about rejections a *cached* module already
/// recorded, and reading only the first half is how those get dropped on every
/// run after the first.
#[must_use]
pub fn has_nothing_to_report(report: &TerminationReport, carried: &CachedTermination) -> bool {
    report.is_clean() && carried.failures.is_empty()
}

impl AdmissionSplit {
    /// Split already-rendered rejections at `level`.
    #[must_use]
    pub fn from_rejections(
        rejections: impl IntoIterator<Item = (ElabError, bool)>,
        level: Enforcement,
    ) -> Self {
        let mut split = AdmissionSplit::default();
        for (error, proof_relevant) in rejections {
            split.push(error, proof_relevant, level);
        }
        split
    }

    /// Add one already-rendered rejection at `level`.
    pub fn push(&mut self, error: ElabError, proof_relevant: bool, level: Enforcement) {
        if level.gates(proof_relevant) {
            self.gating.push(error);
        } else {
            self.reported.push(error);
        }
    }
}

/// Render a report's failures as elaboration errors, pointed at source.
///
/// `sites` supplies the file — a term span is an offset into the failing
/// definition's own file, never the entry's (ADR 3.10.26r D1) — and the
/// fallback span: a failure about a whole definition (no decreasing
/// parameter, an ambiguity, a taint chain) has no single call site to blame,
/// and pointing at the definition is better than pointing at byte 0.
#[must_use]
pub fn admission_errors(report: &TerminationReport, sites: &DefinitionSites<'_>) -> Vec<ElabError> {
    report
        .failures
        .iter()
        .map(|failure| {
            let site = sites.get(&failure.function);
            let span = failure
                .span
                .map(|term_span| Span::new(term_span.start, term_span.end))
                .or(site.map(|(span, _)| span))
                .unwrap_or_default();
            let error = build_error(failure, span);
            match site.and_then(|(_, file)| file) {
                Some(file) => error.with_file_path(file),
                None => error,
            }
        })
        .collect()
}

/// Fold the rejections cache hits carried into the recomputed ones, one per
/// proof and reason (ADR 3.10.26r D3).
///
/// A carried proof takes part in the recomputation, so a rejection its module
/// recorded when fresh comes back a second time. Only a carried/recomputed
/// pair naming the same definition with the same rendered reason is merged —
/// never two recomputed or two carried ones, and never two different proofs.
/// Of a pair, the recomputed one is kept when it has a file; otherwise the
/// carried one, which was rendered with its definition at hand, replaces it.
#[must_use]
pub fn merge_carried(
    mut recomputed: Vec<(ElabError, bool)>,
    carried: Vec<(ElabError, bool)>,
) -> Vec<(ElabError, bool)> {
    let fresh_count = recomputed.len();
    for rejection in carried {
        let key = rejection_key(&rejection.0);
        let twin = recomputed[..fresh_count]
            .iter()
            .position(|(error, _)| key.is_some() && rejection_key(error) == key);
        match twin {
            Some(index) if recomputed[index].0.file_path.is_none() => {
                recomputed[index] = rejection;
            }
            Some(_) => {}
            None => recomputed.push(rejection),
        }
    }
    recomputed
}

/// The definition a rejection names and its rendered reason — the engine's
/// `(function, reason)` as it survives rendering into a carried `ElabError`.
/// `None` for anything the termination gate does not raise.
fn rejection_key(error: &ElabError) -> Option<(&str, &str)> {
    let function = match &error.kind {
        ElabErrorKind::PartialInProof { proof, .. }
        | ElabErrorKind::ForeignInProof { proof, .. } => proof,
        ElabErrorKind::CannotProveTermination { function, .. } => function,
        _ => return None,
    };
    Some((function.as_str(), error.message.as_str()))
}

/// One failure as an `ElabError`, with the engine's notes and help attached.
fn build_error(failure: &TerminationFailure, span: Span) -> ElabError {
    let kind = match &failure.reason {
        FailureReason::PartialInProof { tainted, .. } => ElabErrorKind::PartialInProof {
            proof: failure.function.clone(),
            tainted: tainted.clone(),
        },
        // Explicit, or the fall-through below would report it as E0062
        // (ADR 3.10.26c D3).
        FailureReason::ForeignInProof { extern_symbol, via } => ElabErrorKind::ForeignInProof {
            proof: failure.function.clone(),
            via: via.clone(),
            extern_symbol: extern_symbol.clone(),
        },
        _ => ElabErrorKind::CannotProveTermination {
            function: failure.function.clone(),
            headline: failure.headline(),
        },
    };
    let mut error = ElabError::new(span, kind);
    for note in failure.notes() {
        error = error.with_note(note);
    }
    match failure.suggestion() {
        Some(help) => error.with_help(help),
        None => error,
    }
}
