//! AC 4 — how an obligation finds its definition, and where it is discharged
//! (ADR 2.10.26c D5). Split from `boundary_tests.rs`, which keeps the
//! kernel's verdicts and D8's Holes rule.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use albite_core::terms::termination::AdmissionState;
use albite_core::{Context, Term, Type, TypeError};

use super::{add_zero_right, def, eq_nat, refusal, sites, theorem, TraceBuilder};
use crate::elaborate::kernel::{
    discharge, discharge_local, verdicts_agree, DefinitionSite, KernelObligation,
    RecordPresentation, RegisterClass, COPIES_DISAGREE,
};
use crate::elaborate::{CoreDef, DefTerminationMeta, ElabError};
use crate::span::Span;

/// A pass that already failed discharges nothing: the refusal would be noise
/// over a run that fails regardless (D10).
#[test]
fn a_failed_pass_discharges_nothing() {
    let source = "#[partial]
        fn spin(n: Nat) -> Eq<Nat, n, n> { spin(n) }
        theorem t(n: Nat) -> Eq<Nat, n, n> by { exact spin(n) }
        fn bad() -> Nat { true }";
    let errors = crate::elaborate::tests::elab_err(source);
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0010"),
        "{errors:?}"
    );
    assert!(
        !errors.iter().any(|e| e.kind.code() == "E0081"),
        "{errors:?}"
    );
}

// ADR 2.10.26c AC4 (D5): never a skip.
#[test]
fn an_obligation_without_its_definition_is_refused() {
    let (_, obligation) = add_zero_right("t", Span::new(0, 50), None);
    let (_, cause) = refusal(&discharge(&[], &BTreeMap::new(), vec![obligation]));
    assert!(cause.contains("could not be identified"), "{cause}");
}

// ADR 2.10.26c AC4 (D5): two name+span+module matches refuse too.
#[test]
fn an_obligation_matching_two_definitions_is_refused() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), None);
    let defs = [thm.clone(), thm];
    let (_, cause) = refusal(&discharge(
        &sites(&defs, None),
        &BTreeMap::new(),
        vec![obligation],
    ));
    assert!(cause.contains("could not be identified"), "{cause}");
}

/// `thm` with its term replaced by one the kernel refuses at `ty`.
fn ill_typed(thm: &CoreDef, ty: Type) -> CoreDef {
    def(&thm.name, ty, Term::Zero, thm.span)
}

fn a_tg() -> Option<&'static Path> {
    Some(Path::new("a.tg"))
}

// 9.10.26p AC1 (D2): a file reached by two import paths contributes two
// copies of each definition, both stamped with that file; each is checked
// and the obligation passes once both pass.
#[test]
fn copies_of_one_files_definition_are_checked_each() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), Some("a.tg"));
    let defs = [thm.clone(), thm.clone()];
    let errors = discharge(
        &sites(&defs, a_tg()),
        &BTreeMap::new(),
        vec![obligation.clone()],
    );
    assert!(errors.is_empty(), "{errors:?}");
    // Both copies refused agree: the refusal is the first copy's, the kernel's.
    let refused = [
        ill_typed(&thm, thm.ty.clone()),
        ill_typed(&thm, thm.ty.clone()),
    ];
    let (_, cause) = refusal(&discharge(
        &sites(&refused, a_tg()),
        &BTreeMap::new(),
        vec![obligation],
    ));
    assert!(!cause.contains("could not be identified"), "{cause}");
    assert_ne!(cause, COPIES_DISAGREE);
}

// 9.10.26p AC1 (D2) must-fail twins: a second copy the kernel refuses, or
// one whose `ty` differs while both are refused, is the disagreement — never
// "could not be identified", and never whichever copy happened to come first.
#[test]
fn copies_that_disagree_are_refused() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), Some("a.tg"));
    let other_ty = eq_nat(Term::Zero, Term::Zero);
    let pairs = [
        [thm.clone(), ill_typed(&thm, thm.ty.clone())],
        [ill_typed(&thm, thm.ty.clone()), thm.clone()],
        [ill_typed(&thm, thm.ty.clone()), ill_typed(&thm, other_ty)],
    ];
    for defs in pairs {
        let errors = discharge(
            &sites(&defs, a_tg()),
            &BTreeMap::new(),
            vec![obligation.clone()],
        );
        let (_, cause) = refusal(&errors);
        assert_eq!(cause, COPIES_DISAGREE, "{defs:?}");
        assert_eq!(errors[0].file_path.as_deref(), a_tg());
    }
}

// 9.10.26p AC1 (D2): a pass beside an exemption disagrees, as does a pass
// beside a refusal; the same class twice, or two refusals, agree.
#[test]
fn verdicts_agree_on_pass_class_or_refusal() {
    let (_, obligation) = add_zero_right("t", Span::new(0, 50), None);
    let refused = || Err(discharge(&[], &BTreeMap::new(), vec![obligation.clone()]).remove(0));
    let exempt = || Ok(Some(RegisterClass::CitesExempt));
    assert!(!verdicts_agree(&[Ok(None), exempt()]));
    assert!(!verdicts_agree(&[exempt(), Ok(None)]));
    assert!(!verdicts_agree(&[Ok(None), refused()]));
    assert!(!verdicts_agree(&[refused(), Ok(None)]));
    assert!(verdicts_agree(&[exempt(), exempt()]));
    assert!(verdicts_agree(&[Ok(None), Ok(None)]));
    assert!(verdicts_agree(&[refused(), refused()]));
}

// 9.10.26p AC1 (D2): copies are one file's only when EVERY match names the
// file; a match with no file beside one with it is still unidentifiable.
#[test]
fn a_fileless_match_beside_a_copy_is_refused() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), Some("a.tg"));
    let defs = [thm];
    let mut mixed = sites(&defs, a_tg());
    mixed.extend(sites(&defs, None));
    let (_, cause) = refusal(&discharge(&mixed, &BTreeMap::new(), vec![obligation]));
    assert!(cause.contains("could not be identified"), "{cause}");
}

// 9.10.26p AC1 (D3): each visit stamps an equal obligation; it is
// discharged once, so a refusal is reported once.
#[test]
fn equal_obligations_are_discharged_once() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), Some("a.tg"));
    let defs = [ill_typed(&thm, thm.ty.clone())];
    let errors = discharge(
        &sites(&defs, a_tg()),
        &BTreeMap::new(),
        vec![obligation.clone(), obligation],
    );
    refusal(&errors);
}

// 9.10.26p AC1 (D3): the drop compares the whole obligation — two equal but
// for their file are two definitions, and each is discharged.
#[test]
fn obligations_from_two_files_are_each_discharged() {
    let (thm, in_a) = add_zero_right("t", Span::new(0, 50), Some("a.tg"));
    let in_b = KernelObligation {
        module: Some("b.tg".into()),
        ..in_a.clone()
    };
    let defs = [ill_typed(&thm, thm.ty.clone())];
    let mut both = sites(&defs, a_tg());
    both.extend(sites(&defs, Some(Path::new("b.tg"))));
    let errors = discharge(&both, &BTreeMap::new(), vec![in_a, in_b]);
    assert_eq!(errors.len(), 2, "{errors:?}");
    let files: Vec<_> = errors.iter().map(|e| e.file_path.clone()).collect();
    assert_eq!(files, [Some("a.tg".into()), Some("b.tg".into())]);
}

/// Name and span link together: a definition that agrees on only one of them
/// is not the obligation's, whichever one it is (D5).
#[test]
fn an_obligation_links_on_name_and_span_together() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), None);
    let mut renamed = thm.clone();
    renamed.name = "other".into();
    let mut moved = thm;
    moved.span = Span::new(60, 90);
    for def in [renamed, moved] {
        let defs = [def];
        let (_, cause) = refusal(&discharge(
            &sites(&defs, None),
            &BTreeMap::new(),
            vec![obligation.clone()],
        ));
        assert!(cause.contains("could not be identified"), "{cause}");
    }
}

/// The module is part of the link: the same name and span in another file
/// is not the definition, and the refusal carries the obligation's file.
#[test]
fn an_obligation_links_through_its_module() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), Some("a.tg"));
    let defs = [thm];
    let elsewhere = sites(&defs, Some(std::path::Path::new("b.tg")));
    let errors = discharge(&elsewhere, &BTreeMap::new(), vec![obligation.clone()]);
    let (_, cause) = refusal(&errors);
    assert!(cause.contains("could not be identified"), "{cause}");
    assert_eq!(
        errors[0].file_path.as_deref(),
        Some(std::path::Path::new("a.tg"))
    );
    let home = sites(&defs, Some(std::path::Path::new("a.tg")));
    assert!(discharge(&home, &BTreeMap::new(), vec![obligation]).is_empty());
}

// ADR 2.10.26c AC4 (D5): a compile-time pin — no enforcement level appears
// in the signature; 2.10.26b's `debug_assert!` covers a pooled reader.
#[test]
fn discharge_takes_no_level() {
    let signature: fn(
        &[DefinitionSite<'_>],
        &BTreeMap<String, AdmissionState>,
        Vec<KernelObligation>,
    ) -> Vec<ElabError> = discharge;
    assert!(signature(&[], &BTreeMap::new(), Vec::new()).is_empty());
}

// ADR 2.10.26c AC4 (D5): off the walk, the pass discharges at once and
// leaves nothing behind; with the flag on, the obligation waits.
#[test]
fn only_the_walk_defers() {
    let source = "#[partial]
        fn spin(n: Nat) -> Eq<Nat, n, n> { spin(n) }
        theorem t(n: Nat) -> Eq<Nat, n, n> by { exact spin(n) }";
    let (ast, parse_errors) = crate::parse(source);
    assert!(parse_errors.is_empty(), "{parse_errors:?}");
    for defer in [false, true] {
        let mut ctx = Context::new();
        let mut elaborator = crate::elaborate::Elaborator::new(&mut ctx);
        elaborator.defer_kernel = defer;
        let result = elaborator.elaborate_file(&ast);
        if defer {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(elaborator.kernel_obligations.len(), 1);
            assert_eq!(elaborator.kernel_discharged, 0);
        } else {
            let errors = result.expect_err("E0081 at once");
            assert!(
                errors.iter().any(|e| e.kind.code() == "E0081"),
                "{errors:?}"
            );
            assert!(elaborator.kernel_obligations.is_empty());
            assert_eq!(elaborator.kernel_discharged, 1);
        }
    }
}

// ADR 2.10.26c AC4: admission is computed over what the pass holds.
#[test]
fn discharge_local_binds_what_it_holds() {
    let lemma_ty = eq_nat(Term::Zero, Term::Zero);
    let lemma = def(
        "lem",
        lemma_ty.clone(),
        Term::refl(Type::Nat, Term::Zero),
        Span::new(0, 5),
    );
    let mut trace = TraceBuilder::new(lemma_ty.clone(), vec![]);
    trace.step("exact", 20, Term::Global("lem".into()), &[]);
    let (thm, obligation) = theorem("t", lemma_ty, trace.finish(), Span::new(10, 30), None);
    let meta: HashMap<String, DefTerminationMeta> = HashMap::new();
    let held = [lemma, thm.clone()];
    let none = RecordPresentation::EMPTY;
    assert!(discharge_local(&held, &meta, &none, vec![obligation.clone()]).is_empty());
    let (_, cause) = refusal(&discharge_local(&[thm], &meta, &none, vec![obligation]));
    assert_eq!(cause, TypeError::UnboundVariable("lem".into()).to_string());
}
