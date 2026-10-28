//! ADR 2.10.26d AC 1 — the gate's decision table, one test per arm of §2.1,
//! over injected outcomes.

use std::collections::BTreeSet;

use crate::elaborate::kernel::{
    decide, Cause, ExemptTally, Exemption, GateVerdict, KernelMode, KernelOutcome, RegisterClass,
    Violation,
};

fn row(class: RegisterClass, cites: &[&str]) -> Exemption {
    Exemption {
        file: "t.tg".into(),
        theorem: "t".into(),
        class,
        removed_by: "2.10.26l".into(),
        cites: cites.iter().map(|c| (*c).to_string()).collect(),
    }
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|n| (*n).to_string()).collect()
}

fn needs_delta_refusal() -> KernelOutcome {
    KernelOutcome::Refused {
        cause: Cause::NeedsConversion,
        class: Some(RegisterClass::NeedsDelta),
    }
}

fn gate(outcome: KernelOutcome, row: Option<&Exemption>, cited: &[&str]) -> GateVerdict {
    decide(KernelMode::Gate, outcome, row, &names(cited))
}

// 2.10.26d AC1
#[test]
fn an_admitted_theorem_with_no_row_passes() {
    assert_eq!(gate(KernelOutcome::Admitted, None, &[]), GateVerdict::Pass);
}

// 2.10.26d AC1
#[test]
fn a_refused_theorem_with_no_row_is_refused() {
    assert_eq!(
        gate(needs_delta_refusal(), None, &[]),
        GateVerdict::Refused {
            cause: Cause::NeedsConversion,
            class: Some(RegisterClass::NeedsDelta),
        }
    );
}

// 2.10.26d AC1
#[test]
fn a_refusal_matching_its_row_is_exempt() {
    let exemption = row(RegisterClass::NeedsDelta, &[]);
    assert_eq!(
        gate(needs_delta_refusal(), Some(&exemption), &[]),
        GateVerdict::Exempt(RegisterClass::NeedsDelta)
    );
}

// 2.10.26d AC1: a refused theorem under a cites-exempt row, and a
// needs-delta row over a named-type refusal, are both drift.
#[test]
fn a_row_whose_cause_drifted_is_a_violation() {
    let cites = row(RegisterClass::CitesExempt, &["two_is_two"]);
    assert_eq!(
        gate(needs_delta_refusal(), Some(&cites), &[]),
        GateVerdict::RegisterViolation(Violation::Drifted {
            row: RegisterClass::CitesExempt,
            observed: "needs-delta".into(),
        })
    );
    let named = KernelOutcome::Refused {
        cause: Cause::NamedTypeReference,
        class: Some(RegisterClass::NamedType),
    };
    let delta = row(RegisterClass::NeedsDelta, &[]);
    assert_eq!(
        gate(named, Some(&delta), &[]),
        GateVerdict::RegisterViolation(Violation::Drifted {
            row: RegisterClass::NeedsDelta,
            observed: "named-type".into(),
        })
    );
}

// 2.10.26d AC1
#[test]
fn a_needs_delta_row_for_an_admitted_theorem_is_stale() {
    let exemption = row(RegisterClass::NeedsDelta, &[]);
    assert_eq!(
        gate(KernelOutcome::Admitted, Some(&exemption), &[]),
        GateVerdict::RegisterViolation(Violation::Stale)
    );
}

// 2.10.26d AC1
#[test]
fn a_cites_exempt_row_for_an_admitted_theorem_is_not_stale() {
    let exemption = row(RegisterClass::CitesExempt, &["b", "a"]);
    assert_eq!(
        gate(KernelOutcome::Admitted, Some(&exemption), &["a", "b"]),
        GateVerdict::Exempt(RegisterClass::CitesExempt)
    );
}

// 2.10.26d AC1: nothing cited is exempt any more, or the list disagrees.
#[test]
fn a_cites_exempt_row_whose_cited_names_cleared_is_stale() {
    let exemption = row(RegisterClass::CitesExempt, &["a"]);
    assert_eq!(
        gate(KernelOutcome::Admitted, Some(&exemption), &[]),
        GateVerdict::RegisterViolation(Violation::CitesCleared)
    );
    assert_eq!(
        gate(KernelOutcome::Admitted, Some(&exemption), &["a", "b"]),
        GateVerdict::RegisterViolation(Violation::CitesMismatch {
            listed: vec!["a".into()],
            cited: vec!["a".into(), "b".into()],
        })
    );
}

// 2.10.26d AC1
#[test]
fn an_admitted_theorem_citing_an_exempt_one_needs_a_row() {
    assert_eq!(
        gate(KernelOutcome::Admitted, None, &["two_is_two"]),
        GateVerdict::RegisterViolation(Violation::CitesWithoutRow {
            cited: vec!["two_is_two".into()],
        })
    );
}

// 2.10.26d AC1: a hole keeps "contains sorry", whatever the row (D7).
#[test]
fn a_hole_keeps_its_sorry_status() {
    let exemption = row(RegisterClass::NeedsDelta, &[]);
    assert_eq!(
        gate(KernelOutcome::Hole, None, &[]),
        GateVerdict::SorryStatus
    );
    assert_eq!(
        gate(KernelOutcome::Hole, Some(&exemption), &["x"]),
        GateVerdict::SorryStatus
    );
}

// 2.10.26d AC1: under the census's window nothing is reported, refused or not.
#[test]
fn report_mode_reports_nothing() {
    let exemption = row(RegisterClass::NeedsDelta, &[]);
    for (outcome, row) in [
        (needs_delta_refusal(), None),
        (KernelOutcome::Admitted, Some(&exemption)),
        (KernelOutcome::Admitted, None),
    ] {
        assert_eq!(
            decide(KernelMode::Report, outcome, row, &names(&["x"])),
            GateVerdict::Pass
        );
    }
    assert_eq!(KernelMode::from_window(true), KernelMode::Report);
    assert_eq!(KernelMode::from_window(false), KernelMode::Gate);
}

// 2.10.26d AC1: a row cannot silence a refusal no class explains (D5).
#[test]
fn an_unregistrable_cause_cannot_be_exempted() {
    let subst = KernelOutcome::Refused {
        cause: Cause::DependentSubst,
        class: None,
    };
    let exemption = row(RegisterClass::NeedsDelta, &[]);
    assert_eq!(
        gate(subst, Some(&exemption), &[]),
        GateVerdict::Refused {
            cause: Cause::DependentSubst,
            class: None,
        }
    );
    assert_eq!(
        GateVerdict::refusal_label(Cause::DependentSubst, None),
        "dependent-subst"
    );
    assert_eq!(
        GateVerdict::refusal_label(Cause::NeedsConversion, Some(RegisterClass::NeedsDelta)),
        "needs-delta"
    );
}

/// Each violation's clause names what a reader must change.
#[test]
fn each_violation_describes_itself() {
    let cases = [
        (Violation::Stale, "needs-delta row is stale"),
        (
            Violation::Drifted {
                row: RegisterClass::NeedsDelta,
                observed: "named-type".into(),
            },
            "its row says needs-delta but the kernel's refusal is named-type",
        ),
        (
            Violation::CitesWithoutRow {
                cited: vec!["a".into(), "b".into()],
            },
            "it cites the exempt theorem(s) a, b, so it needs a cites-exempt row",
        ),
        (
            Violation::CitesMismatch {
                listed: vec!["a".into()],
                cited: vec!["b".into()],
            },
            "its cites-exempt row lists a but it cites b",
        ),
        (Violation::CitesCleared, "is exempt any more"),
    ];
    for (violation, expected) in cases {
        let text = violation.describe();
        assert!(text.contains(expected), "{text}");
    }
}

/// D11's tally counts by class and renders in D5's order.
#[test]
fn the_exempt_tally_counts_by_class() {
    let mut tally = ExemptTally::default();
    assert_eq!((tally.total(), tally.render()), (0, String::new()));
    tally.record(RegisterClass::CitesExempt);
    tally.record(RegisterClass::NeedsDelta);
    let mut more = ExemptTally::default();
    more.record(RegisterClass::NeedsDelta);
    tally.absorb(more);
    assert_eq!(tally.total(), 3);
    assert_eq!(tally.render(), "needs-delta 2, cites-exempt 1");
}
