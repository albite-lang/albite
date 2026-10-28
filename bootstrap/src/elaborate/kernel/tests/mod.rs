//! AC 4 — the kernel boundary, over hand-built obligations (ADR 2.10.26c);
//! the term-mode gate's verdict, register and classes (ADR 2.10.26d AC 1–3).

mod attribution_tests;
mod boundary_tests;
mod gap_class_tests;
mod linking_tests;
mod probe_tests;
mod proof_fn;
mod register_tests;
mod verdict_tests;

use std::collections::BTreeMap;

use albite_core::terms::termination::AdmissionState;
use albite_core::{SpannedTerm, Term, TermSpan, Type};

use super::{DefinitionSite, KernelObligation, ObligationOrigin};
use crate::elaborate::tactics::state::{hole_name, MetaId, Partial, ProofState};
use crate::elaborate::tactics::{TacticStep, TacticTrace};
use crate::elaborate::{CoreDef, ElabError, ElabErrorKind};
use crate::span::Span;

/// `Eq Nat left right`.
pub(super) fn eq_nat(left: Term, right: Term) -> Type {
    Type::Eq(Box::new(Type::Nat), Box::new(left), Box::new(right))
}

/// Every definition in `defs`, defined in `module`.
pub(super) fn sites<'d>(
    defs: &'d [CoreDef],
    module: Option<&'d std::path::Path>,
) -> Vec<DefinitionSite<'d>> {
    defs.iter()
        .map(|def| DefinitionSite { module, def })
        .collect()
}

/// `λn. refl (n + 0) : Π n. Eq Nat (n + 0) n`, one `refl` step.
pub(super) fn add_zero_right(
    name: &str,
    span: Span,
    module: Option<&str>,
) -> (CoreDef, KernelObligation) {
    let n_plus_zero = || Term::NatAdd(Box::new(Term::var("n")), Box::new(Term::Zero));
    let target = eq_nat(n_plus_zero(), Term::var("n"));
    let mut trace = TraceBuilder::new(target.clone(), vec![("n".into(), Type::Nat)]);
    trace.step("refl", 10, Term::refl(Type::Nat, n_plus_zero()), &[]);
    let ty = Type::pi_or_arrow("n", Type::Nat, target);
    theorem(name, ty, trace.finish(), span, module)
}

/// A definition `name : ty := term`, at `span`.
pub(super) fn def(name: &str, ty: Type, term: Term, span: Span) -> CoreDef {
    CoreDef {
        name: name.into(),
        ty,
        term: SpannedTerm::new(term, TermSpan::new(span.start, span.end)),
        span,
    }
}

/// A trace being built one step at a time, over the state the steps solve.
pub(super) struct TraceBuilder {
    pub(super) state: ProofState,
    pub(super) steps: Vec<TacticStep>,
    pub(super) value_params: Vec<(String, Type)>,
}

impl TraceBuilder {
    pub(super) fn new(target: Type, value_params: Vec<(String, Type)>) -> Self {
        TraceBuilder {
            state: ProofState::new(target),
            steps: Vec::new(),
            value_params,
        }
    }

    /// A new goal under the focused goal's locals plus `extra`.
    pub(super) fn goal(&mut self, parent: MetaId, extra: &[(&str, Type)], target: Type) -> MetaId {
        let mut locals = self.state.metas[parent].locals.clone();
        locals.extend(extra.iter().map(|(n, t)| ((*n).to_string(), t.clone())));
        self.state.fresh_meta(locals, target)
    }

    /// Run one step: solve the focused goal with `template`, whose holes are
    /// `holes` in order.
    pub(super) fn step(&mut self, tactic: &'static str, at: u32, template: Term, holes: &[MetaId]) {
        let goal = self.state.focus().expect("a goal to solve");
        let exact_sorry = tactic == "exact" && template.contains_sorry();
        self.state.refine(
            goal,
            Partial {
                template,
                holes: holes.iter().map(|h| (hole_name(*h), *h)).collect(),
            },
        );
        self.steps.push(TacticStep {
            tactic,
            span: Span::new(at, at + 1),
            goal,
            exact_sorry,
            citation: None,
        });
    }

    pub(super) fn finish(self) -> TacticTrace {
        TacticTrace {
            state: self.state,
            steps: self.steps,
            value_params: self.value_params,
            type_params: Vec::new(),
            block_span: Span::new(90, 99),
        }
    }
}

/// The theorem `name : ty` whose term is `trace`'s assembly, and its
/// obligation, both at `span`, in `module`.
pub(super) fn theorem(
    name: &str,
    ty: Type,
    trace: TacticTrace,
    span: Span,
    module: Option<&str>,
) -> (CoreDef, KernelObligation) {
    let term = trace.assemble(&BTreeMap::new()).expect("a finished trace");
    let obligation = KernelObligation {
        def: name.into(),
        span,
        module: module.map(Into::into),
        origin: ObligationOrigin::Tactics(trace),
    };
    (def(name, ty, term, span), obligation)
}

/// Every name in `names` admitted `Total`.
/// The probes over `defs`, unfolding exactly what the kernel admits under
/// `admission` (ADR 2.10.26l D8), with `List` as the one concrete type name.
pub(super) fn probes_over(
    defs: &[CoreDef],
    admission: &BTreeMap<String, AdmissionState>,
) -> super::ElaboratedProbes {
    let mut delta = super::delta_admission(defs, admission, None);
    super::ElaboratedProbes::new(&mut delta, std::collections::HashSet::from(["List".into()]))
}

pub(super) fn total(names: &[&str]) -> BTreeMap<String, AdmissionState> {
    names
        .iter()
        .map(|n| ((*n).to_string(), AdmissionState::Total))
        .collect()
}

/// The single E0081's tactic and cause.
pub(super) fn refusal(errors: &[ElabError]) -> (Option<(String, Span)>, String) {
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind.code(), "E0081");
    match &errors[0].kind {
        ElabErrorKind::KernelRefusedProof { tactic, cause, .. } => (tactic.clone(), cause.clone()),
        other => panic!("expected E0081, got {other:?}"),
    }
}
