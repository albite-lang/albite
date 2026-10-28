//! AC 4 — the kernel's verdicts on tactic proofs, and D8's Holes rule. How an
//! obligation finds its definition is in `linking_tests.rs`.

use std::collections::BTreeMap;

use albite_core::terms::termination::AdmissionState;
use albite_core::{check_proof, Context, Term, Type, TypeError};

use super::{add_zero_right, def, eq_nat, refusal, sites, theorem, total, TraceBuilder};
use crate::elaborate::kernel::{abstract_sorries, discharge, KernelObligation, ObligationOrigin};
use crate::elaborate::tactics::state::hole_name;
use crate::elaborate::CoreDef;
use crate::span::Span;

// ADR 2.10.26c AC4
#[test]
fn an_admitted_tactic_proof_passes() {
    let (thm, obligation) = add_zero_right("t", Span::new(0, 50), None);
    let defs = [thm];
    assert!(discharge(&sites(&defs, None), &BTreeMap::new(), vec![obligation]).is_empty());
}

/// `two = 2 by { refl }`, where `two` is in fact `3`: the kernel unfolds it
/// (ADR 2.10.26l) and still refuses, so the step is a genuine fault.
fn two_is_two() -> (Vec<CoreDef>, KernelObligation) {
    let two = def("two", Type::Nat, Term::NatLit(3), Span::new(0, 5));
    let target = eq_nat(Term::Global("two".into()), Term::NatLit(2));
    let mut trace = TraceBuilder::new(target.clone(), vec![]);
    trace.step(
        "refl",
        20,
        Term::refl(Type::Nat, Term::Global("two".into())),
        &[],
    );
    let (thm, obligation) = theorem(
        "two_is_two",
        target,
        trace.finish(),
        Span::new(10, 30),
        None,
    );
    (vec![two, thm], obligation)
}

// ADR 2.10.26c AC4
#[test]
fn a_refused_tactic_proof_is_e0081_naming_the_tactic() {
    let (defs, obligation) = two_is_two();
    let errors = discharge(&sites(&defs, None), &total(&["two"]), vec![obligation]);
    let (tactic, cause) = refusal(&errors);
    assert_eq!(tactic, Some(("refl".to_string(), Span::new(20, 21))));
    assert_eq!(errors[0].span, Span::new(20, 21), "reported at the tactic");
    let expected = check_proof(
        &Context::new().with_global(
            "two",
            Type::Nat,
            albite_core::context::GlobalAdmission::Admitted,
        ),
        &defs[1].term.term,
        &defs[1].ty,
    )
    .expect_err("a marker without a body refuses, and so does `3` unfolded");
    assert!(
        matches!(expected, TypeError::TypeMismatch { .. }),
        "{expected:?}"
    );
    assert_eq!(cause, expected.to_string());
}

// ADR 2.10.26c AC4
#[test]
fn an_opaque_citation_is_refused() {
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
    let defs = [lemma, thm];
    let not_total = BTreeMap::from([("lem".to_string(), AdmissionState::Rejected)]);
    let (tactic, cause) = refusal(&discharge(
        &sites(&defs, None),
        &not_total,
        vec![obligation],
    ));
    assert_eq!(tactic.map(|t| t.0), Some("exact".to_string()));
    assert_eq!(
        cause,
        TypeError::GlobalNotAdmitted { name: "lem".into() }.to_string()
    );
}

/// One `exact` step whose term is `term : ty`.
fn exact_obligation(name: &str, term: Term, ty: Type) -> (CoreDef, KernelObligation) {
    let mut trace = TraceBuilder::new(ty.clone(), vec![]);
    trace.step("exact", 20, term, &[]);
    theorem(name, ty, trace.finish(), Span::new(10, 30), None)
}

// ADR 2.10.26c AC4 (D9): the parked 28.9.26c routes meet the gates here.
#[test]
fn the_parked_routes_are_refused_at_the_boundary() {
    let negative_mu = Type::mu("a", Type::arrow(Type::TyVar("a".into()), Type::Void));
    let omega = Term::lambda(
        "x",
        negative_mu.clone(),
        Term::app(
            Term::unfold(negative_mu.clone(), Term::var("x")),
            Term::var("x"),
        ),
    );
    let curry = Term::app(omega.clone(), Term::fold(negative_mu, omega));
    let routes: [(Term, Type, fn(&TypeError) -> bool); 3] = [
        (
            Term::fix("f", Type::Void, Term::var("f")),
            Type::Void,
            |e| matches!(e, TypeError::FixNotStructural { .. }),
        ),
        (curry, Type::Void, |e| {
            matches!(e, TypeError::NonStrictlyPositiveMu { .. })
        }),
        (Term::early_return(Term::Unit), Type::Unit, |e| {
            matches!(e, TypeError::ReturnInProof)
        }),
    ];
    for (term, ty, is_gate) in routes {
        let gate = check_proof(&Context::new(), &term, &ty).expect_err("a gate refuses");
        assert!(is_gate(&gate), "unexpected gate error {gate:?}");
        let (thm, obligation) = exact_obligation("route", term, ty);
        let defs = [thm];
        let (_, cause) = refusal(&discharge(
            &sites(&defs, None),
            &BTreeMap::new(),
            vec![obligation],
        ));
        assert_eq!(cause, gate.to_string());
    }
}

/// `t(m) : Eq m m → Eq m m by { intro h; apply both; refl; exact sorry }`,
/// where `both : Π n. P n → Eq n n → Eq n n` and the `refl` closes `P m`.
fn sorry_beside_refl(
    premise: impl Fn(Term) -> Type,
    refl_of: Term,
) -> (Vec<CoreDef>, KernelObligation) {
    let m = || Term::var("m");
    let eq_m = eq_nat(m(), m());
    let mut trace = TraceBuilder::new(
        Type::arrow(eq_m.clone(), eq_m.clone()),
        vec![("m".into(), Type::Nat)],
    );
    let body = trace.goal(0, &[("h", eq_m.clone())], eq_m.clone());
    trace.step(
        "intro",
        10,
        Term::lambda("h", eq_m.clone(), Term::var(hole_name(body))),
        &[body],
    );
    let first = trace.goal(body, &[], premise(m()));
    let second = trace.goal(body, &[], eq_m.clone());
    let applied = Term::app(
        Term::app(
            Term::app(Term::Global("both".into()), m()),
            Term::var(hole_name(first)),
        ),
        Term::var(hole_name(second)),
    );
    trace.step("apply", 20, applied, &[first, second]);
    trace.step("refl", 30, Term::refl(Type::Nat, refl_of), &[]);
    trace.step("exact", 40, Term::Sorry, &[]);
    let both_ty = Type::pi_or_arrow(
        "n",
        Type::Nat,
        Type::arrow(
            premise(Term::var("n")),
            Type::arrow(
                eq_nat(Term::var("n"), Term::var("n")),
                eq_nat(Term::var("n"), Term::var("n")),
            ),
        ),
    );
    let both = def("both", both_ty, Term::Sorry, Span::new(0, 5));
    let ty = Type::pi_or_arrow("m", Type::Nat, Type::arrow(eq_m.clone(), eq_m));
    let (thm, obligation) = theorem("t", ty, trace.finish(), Span::new(10, 60), None);
    (vec![both, thm], obligation)
}

// ADR 2.10.26c AC4 (D8): the `sorry` premise is abstracted over the full
// chain, `Π(params)(all intro locals). target`, and the rest is checked.
#[test]
fn an_abstracted_sorry_lets_a_sound_proof_through() {
    let plus_zero = |n: Term| eq_nat(Term::NatAdd(Box::new(n.clone()), Box::new(Term::Zero)), n);
    let m_plus_zero = Term::NatAdd(Box::new(Term::var("m")), Box::new(Term::Zero));
    let (defs, obligation) = sorry_beside_refl(plus_zero, m_plus_zero);
    let ObligationOrigin::Tactics(trace) = &obligation.origin else {
        panic!("a tactic obligation");
    };
    let (overrides, hypotheses) = abstract_sorries(trace);
    let eq_m = eq_nat(Term::var("m"), Term::var("m"));
    assert_eq!(
        hypotheses,
        vec![(
            "%hyp0".to_string(),
            Type::pi_or_arrow("m", Type::Nat, Type::arrow(eq_m.clone(), eq_m))
        )]
    );
    assert_eq!(
        overrides.values().next(),
        Some(&Term::app(
            Term::app(Term::var("%hyp0"), Term::var("m")),
            Term::var("h")
        ))
    );
    assert!(discharge(&sites(&defs, None), &total(&["both"]), vec![obligation]).is_empty());
}

// ADR 2.10.26c AC4 (D8): a hole does not hide a faulty step beside it.
#[test]
fn a_sorry_does_not_hide_a_faulty_step() {
    let two_eq = |_: Term| eq_nat(Term::Global("two".into()), Term::NatLit(2));
    let (mut defs, obligation) = sorry_beside_refl(two_eq, Term::Global("two".into()));
    // `two` is `3`, so the `refl` step is false even with δ (ADR 2.10.26l).
    defs.push(def("two", Type::Nat, Term::NatLit(3), Span::new(70, 75)));
    let errors = discharge(
        &sites(&defs, None),
        &total(&["both", "two"]),
        vec![obligation],
    );
    let (tactic, _) = refusal(&errors);
    assert_eq!(tactic, Some(("refl".to_string(), Span::new(30, 31))));
}

// ADR 2.10.26c AC4: the 2.10.26d hand-off — no steps, nothing to abstract.
#[test]
fn a_term_origin_sorry_keeps_the_plain_exemption() {
    let target = eq_nat(Term::Zero, Term::Zero);
    let mut trace = TraceBuilder::new(target.clone(), vec![]);
    trace.state.refine(
        0,
        crate::elaborate::tactics::state::Partial::closed(Term::Sorry),
    );
    let (thm, obligation) = theorem("t", target, trace.finish(), Span::new(0, 9), None);
    let defs = [thm];
    assert!(discharge(&sites(&defs, None), &BTreeMap::new(), vec![obligation]).is_empty());
}

/// The plain exemption needs BOTH halves: no steps and a `SorryInProof`. A
/// `sorry` a step assembled itself (here an `apply` head) is not abstracted,
/// so it is a refusal; and a step-less origin's other refusals still report.
#[test]
fn the_plain_exemption_needs_no_steps_and_a_sorry() {
    let target = eq_nat(Term::Zero, Term::Zero);
    let mut stepped = TraceBuilder::new(target.clone(), vec![]);
    stepped.step("apply", 20, Term::Sorry, &[]);
    let (thm, obligation) = theorem(
        "t",
        target.clone(),
        stepped.finish(),
        Span::new(10, 30),
        None,
    );
    let defs = [thm];
    let (tactic, cause) = refusal(&discharge(
        &sites(&defs, None),
        &BTreeMap::new(),
        vec![obligation],
    ));
    assert_eq!(cause, TypeError::SorryInProof.to_string());
    assert_eq!(tactic.map(|t| t.0), Some("apply".to_string()));

    let (mut defs, _) = two_is_two();
    let target = eq_nat(Term::Global("two".into()), Term::NatLit(2));
    let mut stepless = TraceBuilder::new(target.clone(), vec![]);
    stepless.state.refine(
        0,
        crate::elaborate::tactics::state::Partial::closed(Term::refl(
            Type::Nat,
            Term::Global("two".into()),
        )),
    );
    let (thm, obligation) = theorem("plain", target, stepless.finish(), Span::new(40, 50), None);
    defs.push(thm);
    let errors = discharge(&sites(&defs, None), &total(&["two"]), vec![obligation]);
    let (_, cause) = refusal(&errors);
    assert!(cause.contains("mismatch"), "{cause}");
}
