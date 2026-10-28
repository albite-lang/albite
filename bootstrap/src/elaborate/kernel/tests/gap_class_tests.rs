//! ADR 2.10.26d AC 3 — the register classes are what D5 says, pure and
//! through the gate's shell (`discharge_gated`) over hand-built definitions.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use albite_core::{Term, Type, TypeError};

use super::{def, eq_nat, probes_over, sites, total};
use crate::elaborate::kernel::{
    classify, discharge_gated, register_class, Cause, ElaboratedProbes, KernelMode,
    KernelObligation, ObligationOrigin, Probes, RecordPresentation, Refinement, Register,
    RegisterClass, TermGate,
};
use crate::elaborate::{CoreDef, ElabErrorKind};
use crate::span::Span;

fn two() -> Term {
    Term::succ(Term::succ(Term::Zero))
}

/// The probes over `two := 2`, admitted `Total`.
fn probes_with_two() -> ElaboratedProbes {
    let two = def("two", Type::Nat, two(), Span::new(0, 1));
    probes_over(&[two], &total(&["two"]))
}

fn mismatch(expected: Type, got: Type) -> TypeError {
    TypeError::TypeMismatch { expected, got }
}

// 2.10.26d AC3: `Eq two 2` against `Eq two two` converts with δ and not without.
#[test]
fn a_pair_that_converts_only_with_delta_is_needs_delta() {
    let probes = probes_with_two();
    let stated = eq_nat(Term::global("two"), two());
    let proved = eq_nat(Term::global("two"), Term::global("two"));
    let refusal = mismatch(stated.clone(), proved.clone());
    assert_eq!(
        classify(&refusal, &probes.probes_for(&refusal)),
        Cause::NeedsConversion
    );
    assert!(probes.converts(&stated, &proved));
    assert!(!probes.converts_without_delta(&stated, &proved));
    let refinement = Refinement {
        converts_without_delta: Some(false),
        ..Refinement::default()
    };
    assert_eq!(
        register_class(Cause::NeedsConversion, refinement),
        Some(RegisterClass::NeedsDelta)
    );
}

// 2.10.26d AC3: converting without δ while the kernel refuses is a
// kernel-conversion disagreement, not δ — and "not asked" is no class either.
#[test]
fn a_pair_that_converts_without_delta_is_unregistrable() {
    let zero_plus_zero = Term::nat_add(Term::Zero, Term::Zero);
    let stated = eq_nat(zero_plus_zero.clone(), Term::Zero);
    let proved = eq_nat(zero_plus_zero.clone(), zero_plus_zero);
    assert!(probes_with_two().converts_without_delta(&stated, &proved));
    let disagrees = Refinement {
        converts_without_delta: Some(true),
        ..Refinement::default()
    };
    assert_eq!(register_class(Cause::NeedsConversion, disagrees), None);
    assert_eq!(
        register_class(Cause::NeedsConversion, Refinement::default()),
        None
    );
}

// 2.10.26d AC3
#[test]
fn an_unbound_self_name_is_self_reference() {
    let own = Refinement {
        unbound_is_self: Some(true),
        ..Refinement::default()
    };
    assert_eq!(
        register_class(Cause::MissingGlobalContext, own),
        Some(RegisterClass::SelfReference)
    );
    // Through the shell: the context builder drops the theorem's own name.
    let errors = gate_errors(vec![def("t", Type::Nat, Term::var("t"), Span::new(0, 9))]);
    assert_refused(&errors, "self-reference");
}

// 2.10.26d AC3
#[test]
fn an_unbound_foreign_name_is_unregistrable() {
    let foreign = Refinement {
        unbound_is_self: Some(false),
        ..Refinement::default()
    };
    assert_eq!(register_class(Cause::MissingGlobalContext, foreign), None);
    let errors = gate_errors(vec![def(
        "t",
        Type::Nat,
        Term::var("ghost"),
        Span::new(0, 9),
    )]);
    assert_refused(&errors, "missing-global-context");
}

// 2.10.26d AC3
#[test]
fn a_named_type_reference_maps_to_named_type() {
    let refusal = TypeError::UnboundTypeVar("List".into());
    let probes = probes_with_two().probes_for(&refusal);
    let cause = classify(&refusal, &probes);
    assert_eq!(cause, Cause::NamedTypeReference);
    assert_eq!(
        register_class(cause, Refinement::default()),
        Some(RegisterClass::NamedType)
    );
    for other in [
        Cause::DependentSubst,
        Cause::StrictEquality,
        Cause::Unclassified,
    ] {
        assert_eq!(
            register_class(other, Refinement::default()),
            None,
            "{other:?}"
        );
    }
    assert_eq!(
        Probes::default(),
        probes_with_two().probes_for(&TypeError::EmptyMatch)
    );
}

// ── The gate's shell over hand-built definitions ──

const ROOT: &str = "/repo";
const FILE: &str = "/repo/k.tg";

/// `two := 2` and `t : Eq Nat two 2 := refl two` — δ's standing example.
fn delta_defs() -> Vec<CoreDef> {
    vec![
        def("two", Type::Nat, two(), Span::new(0, 5)),
        def(
            "t",
            eq_nat(Term::global("two"), two()),
            Term::refl(Type::Nat, Term::global("two")),
            Span::new(10, 30),
        ),
    ]
}

fn term_obligation(def: &CoreDef) -> KernelObligation {
    KernelObligation {
        def: def.name.clone(),
        span: def.span,
        module: Some(FILE.into()),
        origin: ObligationOrigin::Term,
    }
}

/// `t : List → List := λx:List. x`, where `List` reaches the kernel as a bare
/// name: a `named-type` refusal, which the kernel still makes after K-δ
/// (ADR 2.10.26l moved the gate's standing example off `needs-delta`, which a
/// consistent probe can no longer produce).
fn named_type_defs() -> Vec<CoreDef> {
    let list = Type::TyVar("List".into());
    vec![def(
        "t",
        Type::arrow(list.clone(), list.clone()),
        Term::lambda("x", list, Term::var("x")),
        Span::new(10, 30),
    )]
}

fn run_gate(
    defs: &[CoreDef],
    register: &Register,
    mode: KernelMode,
) -> crate::elaborate::kernel::KernelDischarge {
    let obligation = term_obligation(defs.last().expect("a theorem"));
    let names = HashSet::from(["List".to_string()]);
    let gate = TermGate {
        mode,
        register,
        root: Some(Path::new(ROOT)),
        concrete_type_names: Some(&names),
        decreasing: None,
        presentation: &RecordPresentation::EMPTY,
    };
    discharge_gated(
        &sites(defs, Some(Path::new(FILE))),
        &total(&["two"]),
        vec![obligation],
        &gate,
    )
}

fn gate_errors(defs: Vec<CoreDef>) -> Vec<crate::elaborate::ElabError> {
    run_gate(&defs, &Register::EMPTY, KernelMode::Gate).errors
}

fn assert_refused(errors: &[crate::elaborate::ElabError], label: &str) {
    assert_eq!(errors.len(), 1, "{errors:?}");
    match &errors[0].kind {
        ElabErrorKind::KernelRefusedProof {
            tactic: None,
            cause,
            ..
        } => {
            assert!(cause.starts_with(&format!("{label} — ")), "{cause}");
        }
        other => panic!("expected a term-mode E0081, got {other:?}"),
    }
}

fn register_row(file: &str, cause: &str, removed_by: &str) -> Register {
    Register::parse(&format!(
        "schema = 1\n[[exemption]]\nfile = \"{file}\"\ntheorem = \"t\"\n\
         cause = \"{cause}\"\nremoved_by = \"{removed_by}\"\n"
    ))
    .expect("a well-formed register")
}

fn delta_register(file: &str) -> Register {
    register_row(file, "needs-delta", "2.10.26l")
}

// 2.10.26d AC1/AC3 through the shell: no row refuses, a row of the wrong
// class is a violation, and a row for a file outside the root matches
// nothing. Since ADR 2.10.26l no real refusal is of a class the register
// admits today (`needs-delta` is what K-δ removed, and `named-type` rows
// wait on 3.10.26p), so the exempting half is `decide`'s, in verdict_tests.
#[test]
fn the_shell_refuses_flags_drift_and_keys_by_file() {
    let defs = named_type_defs();
    assert_refused(&gate_errors(defs.clone()), "named-type");
    let drifted = run_gate(&defs, &delta_register("k.tg"), KernelMode::Gate).errors;
    assert_eq!(drifted.len(), 1, "{drifted:?}");
    assert_eq!(drifted[0].kind.code(), "E0084");
    let elsewhere = run_gate(&defs, &delta_register("other.tg"), KernelMode::Gate);
    assert_refused(&elsewhere.errors, "named-type");
}

// ADR 2.10.26l AC 4 through the shell: δ's standing example now passes the
// gate with no register row at all, and its old `needs-delta` row is stale.
#[test]
fn the_shell_admits_the_delta_example_with_no_row() {
    let defs = delta_defs();
    assert!(gate_errors(defs.clone()).is_empty());
    let stale = run_gate(&defs, &delta_register("k.tg"), KernelMode::Gate).errors;
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!(stale[0].kind.code(), "E0084");
}

// 2.10.26d AC1 through the shell: report mode drops the refusal; a stale row
// is E0084 naming the theorem.
#[test]
fn the_shell_reports_nothing_in_report_mode_and_flags_a_stale_row() {
    let defs = delta_defs();
    let reported = run_gate(&defs, &Register::EMPTY, KernelMode::Report);
    assert!(reported.errors.is_empty() && reported.exempt.total() == 0);
    let sound = vec![def(
        "t",
        eq_nat(two(), two()),
        Term::refl(Type::Nat, two()),
        Span::new(10, 30),
    )];
    let stale = run_gate(&sound, &delta_register("k.tg"), KernelMode::Gate).errors;
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!(stale[0].kind.code(), "E0084");
    assert!(matches!(
        &stale[0].kind,
        ElabErrorKind::KernelExemptionViolation { def, why } if def == "t" && why.contains("stale")
    ));
    assert_eq!(stale[0].file_path.as_deref(), Some(Path::new(FILE)));
}

// 2.10.26d D7 through the shell: a term containing `sorry` is not reported.
#[test]
fn the_shell_leaves_a_hole_to_its_sorry_status() {
    let holed = vec![def(
        "t",
        eq_nat(Term::Zero, two()),
        Term::Sorry,
        Span::new(0, 9),
    )];
    assert!(gate_errors(holed).is_empty());
}
