//! The trusted boundary's admission step (ADR 29.6.26e §2.5): termination,
//! then the deferred conversions, then the kernel. Moved from
//! `accumulator/mod.rs` by ADR 2.10.26d (§2.3), to keep that file under its
//! size threshold as the term-mode kernel gate joins it.

use std::collections::{BTreeMap, HashMap, HashSet};

use albite_core::terms::termination::AdmissionState;
use albite_core::Term;

use super::ModuleTreeAccumulator;
use crate::driver::modules::ParsedModule;
use crate::elaborate::conversion::{self, ConversionObligation};
use crate::elaborate::handles;
use crate::elaborate::kernel::{
    self, repository_root, DefinitionSite, KernelMode, ObligationOrigin, RecordPresentation,
    Register, TermGate,
};
use crate::elaborate::termination;
use crate::elaborate::ElabError;

impl ModuleTreeAccumulator {
    /// Run the termination gate over everything the tree produced
    /// (ADR 29.6.26e §2.5).
    ///
    /// A method on the accumulator because the accumulator *is* the assembled
    /// trusted environment at this point — every module's definitions,
    /// annotations, and whatever the elaboration cache carried forward.
    ///
    /// Failures the current enforcement level does not gate become warnings
    /// rather than being dropped: a definition the checker could not certify is
    /// worth saying out loud even while the corpus is being brought up to the
    /// rule, and both callers already render `warnings`.
    ///
    /// Deferred conversions are discharged here too, against the verdicts just
    /// computed (ADR 28.9.26d D11): their failures follow the gating errors.
    /// Then the kernel, whose term-mode gate `kernel_mode` decides, read once
    /// on the caller thread (ADR 2.10.26d D2).
    pub(in crate::driver::per_module) fn admit_or_reject(
        &mut self,
        verbose: bool,
        kernel_mode: KernelMode,
    ) -> Result<(), Vec<ElabError>> {
        let carried = self.carried_termination.as_carried();
        let input = termination::TerminationInput::from_meta(&self.termination_meta);
        let report = input.check_with_carried(&self.defs, &carried);
        let conversion_failures = conversion::discharge_after_admission(
            &self.defs,
            &report.admission,
            std::mem::take(&mut self.conversion_obligations),
            std::mem::take(&mut self.conversion_deferred),
            verbose,
        );
        let mut errors = Vec::new();
        if !termination::has_nothing_to_report(&report, &self.carried_termination) {
            let level = termination::enforcement();
            // A module served from cache contributed no terms, so its rejections
            // come back rendered rather than recomputed — the alternative is that
            // they are reported once and never again. One that is ALSO
            // recomputed is reported once (ADR 3.10.26r D3).
            let carried_failures = std::mem::take(&mut self.carried_termination.failures);
            let recomputed = input.rejections(&report, &self.definition_sites());
            let rejections = termination::merge_carried(recomputed, carried_failures);
            let split = termination::AdmissionSplit::from_rejections(rejections, level);
            self.warnings.extend(split.reported);
            errors = split.gating;
        }
        self.conversion_failure_count = conversion_failures.len();
        let held = (conversion_failures.is_empty(), errors.is_empty());
        let kernel_failures = self.discharge_kernel(&report.admission, held, kernel_mode);
        self.kernel_failure_count = kernel_failures.len();
        errors.extend(conversion_failures);
        errors.extend(kernel_failures);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// [`Self::admit_or_reject`], then the affine handle check over the parsed
    /// `tree` (ADR 7.10.26t D6), its refusals after admission's own errors. It
    /// reads the parsed tree because every module has one here, while a module
    /// the elaboration cache served contributed no Core terms to `defs`.
    pub(in crate::driver::per_module) fn admit_with_handles(
        &mut self,
        verbose: bool,
        kernel_mode: KernelMode,
        tree: &ParsedModule,
    ) -> Result<(), Vec<ElabError>> {
        let admitted = self.admit_or_reject(verbose, kernel_mode);
        handles::with_handle_errors(admitted, handles::check_module_tree(tree))
    }

    /// The deferred conversions of a run whose walk already failed (ADR
    /// 4.10.26e D6): termination over the definitions that elaborated, for
    /// their verdicts only, then the discharge against those verdicts. Its
    /// failures follow the elaboration errors; termination's own rejections
    /// and the kernel are not reported, since the run fails regardless.
    ///
    /// An obligation that reaches a global with no body here — one that
    /// failed to elaborate, or a bodyless cache hit's — is set aside: its
    /// refusal would be a cascade of an error already reported, not a fault
    /// of its own. So the change only adds refusals a clean module would
    /// raise, and never accepts a proof.
    pub(in crate::driver::per_module) fn discharge_beside_errors(
        &mut self,
        verbose: bool,
    ) -> Vec<ElabError> {
        let carried = self.carried_termination.as_carried();
        let input = termination::TerminationInput::from_meta(&self.termination_meta);
        let report = input.check_with_carried(&self.defs, &carried);
        let bodies = crate::comparator::eval::defs_as_globals(&self.defs);
        let obligations: Vec<_> = std::mem::take(&mut self.conversion_obligations)
            .into_iter()
            .filter(|obligation| reaches_only_bodies(obligation, &bodies))
            .collect();
        let deferred = obligations.len();
        self.conversion_deferred = 0;
        conversion::discharge_after_admission(
            &self.defs,
            &report.admission,
            obligations,
            deferred,
            verbose,
        )
    }

    /// Where every definition the gate can name was defined (ADR 3.10.26r):
    /// the assembled set for spans, each module's group for its file (D1),
    /// and the sites cache hits carried for the proofs they served (D2).
    fn definition_sites(&self) -> termination::DefinitionSites<'_> {
        let mut sites = termination::DefinitionSites::of_defs(&self.defs);
        for (_, file, defs) in &self.module_defs {
            sites.add_module(file, defs);
        }
        sites.add_carried(&self.carried_termination);
        sites
    }

    /// Every proof through the kernel, against the admission verdicts just
    /// computed (ADR 2.10.26c D5). Skipped when a conversion obligation
    /// failed: the run already fails, and refusals over the same terms would
    /// be noise. Term-mode theorems are also skipped when termination already
    /// rejected something — one report per theorem (ADR 2.10.26d D1, D2).
    /// Each definition is linked through the file that defined it, which is
    /// also how a term-mode theorem finds its register row (D4).
    fn discharge_kernel(
        &mut self,
        admission: &BTreeMap<String, AdmissionState>,
        (conversions_held, termination_held): (bool, bool),
        kernel_mode: KernelMode,
    ) -> Vec<ElabError> {
        let mut obligations = std::mem::take(&mut self.kernel_obligations);
        if !termination_held {
            obligations.retain(|obligation| obligation.origin != ObligationOrigin::Term);
        }
        if !conversions_held || obligations.is_empty() {
            return Vec::new();
        }
        let sites: Vec<DefinitionSite<'_>> = self
            .module_defs
            .iter()
            .flat_map(|(_, file, defs)| {
                defs.iter().map(move |def| DefinitionSite {
                    module: Some(file.as_path()),
                    def,
                })
            })
            .collect();
        let root = std::fs::canonicalize(repository_root()).unwrap_or_else(|_| repository_root());
        let meta = &self.type_meta;
        let concrete: HashSet<String> = meta
            .adt_types
            .keys()
            .chain(meta.record_types.keys())
            .cloned()
            .collect();
        let hints = kernel::decreasing_hints(&self.termination_meta);
        let presentation = RecordPresentation::new(&meta.encoded_types, meta.record_types.keys());
        let gate = TermGate {
            mode: kernel_mode,
            register: Register::active(),
            root: Some(&root),
            concrete_type_names: Some(&concrete),
            decreasing: Some(&hints),
            presentation: &presentation,
        };
        let discharged = kernel::discharge_gated(&sites, admission, obligations, &gate);
        self.kernel_exempt.absorb(discharged.exempt);
        discharged.errors
    }
}

/// Whether every global `obligation`'s sides reach, through the bodies in
/// `bodies`, has a body there (D6's cascade filter).
pub(in crate::driver::per_module) fn reaches_only_bodies(
    obligation: &ConversionObligation,
    bodies: &HashMap<String, Term>,
) -> bool {
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    collect_globals(&obligation.left, &mut pending);
    collect_globals(&obligation.right, &mut pending);
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        match bodies.get(&name) {
            Some(body) => collect_globals(body, &mut pending),
            None => return false,
        }
    }
    true
}

fn collect_globals(term: &Term, found: &mut Vec<String>) {
    if let Term::Global(name) = term {
        found.push(name.clone());
    }
    term.for_each_subterm(|child| collect_globals(child, found));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use albite_core::{Term, Type};

    use super::{reaches_only_bodies, ConversionObligation};
    use crate::elaborate::ElabError;
    use crate::span::Span;

    fn obligation_over(global: &str) -> ConversionObligation {
        let left = Term::Global(global.into());
        ConversionObligation {
            error: ElabError::invalid_refl(Span::new(0, 4), left.clone(), Term::nat(2)),
            left,
            right: Term::nat(2),
            ty: Type::Nat,
        }
    }

    // ADR 4.10.26e D6: the cascade filter follows bodies transitively, ends
    // on a recursive one, and sets aside any obligation that reaches a
    // global with no body.
    #[test]
    fn an_obligation_is_kept_only_when_every_reached_global_has_a_body() {
        let mut bodies = HashMap::new();
        bodies.insert("two".to_string(), Term::nat(2));
        // `loops` calls itself and `two`: revisiting it must not spin.
        let recursive = Term::app(Term::Global("loops".into()), Term::Global("two".into()));
        bodies.insert("loops".to_string(), recursive);
        bodies.insert(
            "calls_broken".to_string(),
            Term::app(Term::Global("broken".into()), Term::Zero),
        );
        assert!(reaches_only_bodies(&obligation_over("two"), &bodies));
        assert!(reaches_only_bodies(&obligation_over("loops"), &bodies));
        assert!(!reaches_only_bodies(&obligation_over("broken"), &bodies));
        assert!(!reaches_only_bodies(
            &obligation_over("calls_broken"),
            &bodies
        ));
        let mut on_the_right = obligation_over("two");
        on_the_right.right = Term::Global("broken".into());
        assert!(!reaches_only_bodies(&on_the_right, &bodies));
    }
}
