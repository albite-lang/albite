//! The term-mode kernel gate's shell (ADR 2.10.26d D2–D6): each `Term`
//! obligation's kernel run, its classification by the census's own classifier
//! and probes, its register row, and [`decide`]'s verdict mapped to a
//! diagnostic. Split from `kernel/mod.rs` to keep that file under its size
//! threshold; the decision itself is the pure `verdict.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use albite_core::terms::termination::AdmissionState;
use albite_core::typecheck::{present_definition, DeltaAdmission};
use albite_core::{check_proof, Context, Type, TypeError};

use crate::elaborate::kernel::{
    attach_file, bind_with_bodies, classify, decide, delta_admission, mentioned_globals,
    mismatch_pair, present_definitions, refusal, register_class, register_key, Cause,
    DecreasingHints, DefinitionSite, ElaboratedProbes, GateVerdict, KernelMode, KernelObligation,
    KernelOutcome, RecordPresentation, Refinement, Register, RegisterClass,
};
use crate::elaborate::{CoreDef, ElabError, ElabErrorKind};

/// What a `Term` obligation is checked against (ADR 2.10.26d D2, D4).
pub(crate) struct TermGate<'g> {
    pub(crate) mode: KernelMode,
    pub(crate) register: &'g Register,
    /// The canonical repository root; `None` on a path with no register to
    /// consult, where no row can match (D1).
    pub(crate) root: Option<&'g Path>,
    /// The concrete type names the `named-type` probe reads.
    pub(crate) concrete_type_names: Option<&'g HashSet<String>>,
    /// Each definition's `#[decreasing(arg)]`, which δ's `fix_admission`
    /// re-checks (ADR 2.10.26l D4); `None` where no metadata is held.
    pub(crate) decreasing: Option<&'g DecreasingHints>,
    /// Each record's encoding, presented in place of its name (ADR 3.10.26p
    /// D2); empty on a path that holds no type metadata.
    pub(crate) presentation: &'g RecordPresentation,
}

impl TermGate<'static> {
    /// The local discharge's gate: always on, no register (D1).
    pub(crate) const LOCAL: TermGate<'static> = TermGate {
        mode: KernelMode::Gate,
        register: &Register::EMPTY,
        root: None,
        concrete_type_names: None,
        decreasing: None,
        presentation: &RecordPresentation::EMPTY,
    };
}

/// The state the obligations of one discharge share: δ's admission, decided
/// once per definition (ADR 2.10.26l D8), the probes, built on the first
/// refusal only (D9), and the exempt names of the files built.
pub(in crate::elaborate::kernel) struct TermChecks<'d> {
    gate: &'d TermGate<'d>,
    admission: &'d BTreeMap<String, AdmissionState>,
    delta: DeltaAdmission<'d>,
    probes: Option<ElaboratedProbes>,
    exempt: BTreeSet<String>,
}

impl<'d> TermChecks<'d> {
    pub(in crate::elaborate::kernel) fn new(
        defs: &'d [DefinitionSite<'d>],
        admission: &'d BTreeMap<String, AdmissionState>,
        gate: &'d TermGate<'d>,
    ) -> Self {
        let modules: BTreeSet<&Path> = defs.iter().filter_map(|site| site.module).collect();
        let built: BTreeSet<String> = modules
            .into_iter()
            .filter_map(|module| file_key(gate, module))
            .collect();
        TermChecks {
            gate,
            admission,
            delta: delta_admission(defs.iter().map(|site| site.def), admission, gate.decreasing),
            probes: None,
            exempt: gate.register.exempt_names(&built),
        }
    }

    /// One term-mode theorem through the kernel, classified and decided.
    pub(in crate::elaborate::kernel) fn check(
        &mut self,
        def: &CoreDef,
        defs_by_name: &HashMap<&str, &Type>,
        obligation: &KernelObligation,
    ) -> Result<Option<RegisterClass>, ElabError> {
        let (verdict, kernel_error) = self.verdict(def, defs_by_name, obligation.module.as_deref());
        match verdict {
            GateVerdict::Pass | GateVerdict::SorryStatus => Ok(None),
            GateVerdict::Exempt(class) => Ok(Some(class)),
            GateVerdict::Refused { cause, class } => {
                let error = kernel_error.map(|e| e.to_string()).unwrap_or_default();
                let label = GateVerdict::refusal_label(cause, class);
                Err(refusal(obligation, None, format!("{label} — {error}")))
            }
            GateVerdict::RegisterViolation(violation) => Err(attach_file(
                ElabError::new(
                    obligation.span,
                    ElabErrorKind::KernelExemptionViolation {
                        def: obligation.def.clone(),
                        why: violation.describe(),
                    },
                ),
                obligation,
            )),
        }
    }

    /// The gate's verdict on one term-mode theorem defined in `module`, and
    /// the kernel's refusal when there was one. The build's gate and the
    /// corpus census both decide through here (ADR 2.10.26o D4).
    fn verdict(
        &mut self,
        def: &CoreDef,
        defs_by_name: &HashMap<&str, &Type>,
        module: Option<&Path>,
    ) -> (GateVerdict, Option<TypeError>) {
        let term = &def.term.term;
        let mut mentioned = mentioned_globals(term, &def.ty);
        mentioned.remove(&def.name);
        let mut kernel_error = None;
        let outcome = if term.contains_sorry() {
            KernelOutcome::Hole
        } else {
            let base = self.bind(&mentioned, defs_by_name);
            let presented = present_definition(&def.name, &def.ty, term);
            match check_proof(&base, &presented, &def.ty) {
                Ok(()) => KernelOutcome::Admitted,
                Err(error) => {
                    let (cause, class) = self.classify_refusal(def, &error);
                    kernel_error = Some(error);
                    KernelOutcome::Refused { cause, class }
                }
            }
        };
        let key = module.and_then(|module| file_key(self.gate, module));
        let row = key.and_then(|key| self.gate.register.row(&key, &def.name));
        let cited: BTreeSet<String> = mentioned.intersection(&self.exempt).cloned().collect();
        let verdict = decide(self.gate.mode, outcome, row, &cited);
        (verdict, kernel_error)
    }

    /// The context a proof mentioning `mentioned` is checked in: markers,
    /// then δ's admitted bodies — the census's own builder (D8).
    pub(in crate::elaborate::kernel) fn bind(
        &mut self,
        mentioned: &BTreeSet<String>,
        defs_by_name: &HashMap<&str, &Type>,
    ) -> Context {
        bind_with_bodies(mentioned, defs_by_name, self.admission, &mut self.delta)
    }

    /// The records this discharge presents (ADR 3.10.26p D2), for a proof
    /// assembled from a tactic trace rather than read from an
    /// already-presented definition.
    pub(in crate::elaborate::kernel) fn presentation(&self) -> &'d RecordPresentation {
        self.gate.presentation
    }

    /// The census's cause and D5's class for one refusal.
    fn classify_refusal(
        &mut self,
        def: &CoreDef,
        error: &TypeError,
    ) -> (Cause, Option<RegisterClass>) {
        let (delta, names) = (&mut self.delta, self.gate.concrete_type_names);
        let probes = self.probes.get_or_insert_with(|| {
            ElaboratedProbes::new(delta, names.cloned().unwrap_or_default())
        });
        let cause = classify(error, &probes.probes_for(error));
        let converts_without_delta = (cause == Cause::NeedsConversion).then(|| {
            mismatch_pair(error).is_some_and(|(l, r)| probes.converts_without_delta(l, r))
        });
        let unbound_is_self = match error {
            TypeError::UnboundVariable(name) => Some(*name == def.name),
            _ => None,
        };
        let refinement = Refinement {
            converts_without_delta,
            unbound_is_self,
        };
        (cause, register_class(cause, refinement))
    }
}

/// The gate's verdict on each of `theorems` — term-mode theorems, each with
/// its defining file — exactly as a build's discharge decides it, so the
/// corpus census's register diff cannot disagree with the gate (ADR 2.10.26o
/// D4, ADR 2.10.26d D10). `defs` and `admission` are the whole build's.
pub(crate) fn term_verdicts(
    defs: &[DefinitionSite<'_>],
    admission: &BTreeMap<String, AdmissionState>,
    gate: &TermGate<'_>,
    theorems: &[(&CoreDef, &Path)],
) -> Vec<GateVerdict> {
    let presented = present_definitions(defs.iter().map(|site| site.def), gate.presentation);
    let defs: Vec<DefinitionSite<'_>> = defs
        .iter()
        .zip(&presented)
        .map(|(site, def)| DefinitionSite {
            module: site.module,
            def,
        })
        .collect();
    let theorem_defs = present_definitions(theorems.iter().map(|(def, _)| *def), gate.presentation);
    let defs_by_name: HashMap<&str, &Type> = defs
        .iter()
        .map(|site| (site.def.name.as_str(), &site.def.ty))
        .collect();
    let mut checks = TermChecks::new(&defs, admission, gate);
    theorem_defs
        .iter()
        .zip(theorems)
        .map(|(def, (_, module))| checks.verdict(def, &defs_by_name, Some(module)).0)
        .collect()
}

/// `module`'s register key under the gate's root, canonicalising the path
/// first; `None` when the gate has no root or the file lies outside it (D4).
fn file_key(gate: &TermGate<'_>, module: &Path) -> Option<String> {
    let root = gate.root?;
    let canonical = std::fs::canonicalize(module).unwrap_or_else(|_| module.to_path_buf());
    register_key(&canonical, root)
}
