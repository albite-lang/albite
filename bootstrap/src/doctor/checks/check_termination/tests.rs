//! Tests for the termination report's tally and rendering.
//!
//! The reports are built by hand rather than by elaborating a file: the tally
//! and the rendering are what this module owns, and a fixture that had to
//! elaborate would test the engine again instead.

use std::collections::BTreeMap;

use albite_core::terms::termination::{
    AdmissionState, FailureReason, TerminationFailure, TerminationReport,
};

use super::{render_report, TerminationTally};

fn report_with(
    states: &[(&str, AdmissionState)],
    failures: Vec<TerminationFailure>,
) -> TerminationReport {
    TerminationReport {
        admission: states
            .iter()
            .map(|(name, state)| ((*name).to_string(), *state))
            .collect::<BTreeMap<_, _>>(),
        recursive_groups: vec![vec!["spin".to_string()]],
        tainted: Vec::new(),
        foreign_seeds: BTreeMap::new(),
        decreasing: BTreeMap::new(),
        failures,
    }
}

fn no_descent(function: &str) -> TerminationFailure {
    TerminationFailure {
        function: function.to_string(),
        group: vec![function.to_string()],
        span: None,
        reason: FailureReason::NoDescent {
            parameter: "l".to_string(),
            callee: function.to_string(),
            argument: "l".to_string(),
        },
    }
}

fn partial_in_proof(function: &str) -> TerminationFailure {
    TerminationFailure {
        function: function.to_string(),
        group: Vec::new(),
        span: None,
        reason: FailureReason::PartialInProof {
            tainted: "spin".to_string(),
            via: Vec::new(),
        },
    }
}

#[test]
fn a_clean_report_counts_admissions_and_exits_zero() {
    let report = report_with(
        &[
            ("len", AdmissionState::Total),
            ("spin", AdmissionState::Partial),
        ],
        Vec::new(),
    );
    let tally = TerminationTally::from_report(&report);

    assert_eq!(tally.definitions, 2);
    assert_eq!(tally.total, 1);
    assert_eq!(tally.partial, 1);
    assert_eq!(tally.rejected, 0);
    assert_eq!(tally.recursive_groups, 1);
    assert_eq!(
        format!("{:?}", tally.exit()),
        format!("{:?}", std::process::ExitCode::SUCCESS)
    );

    let text = render_report(&report, &tally, &[], false);
    assert!(text.contains("✓ 2 definition(s) admitted"), "{text}");
    assert!(text.contains("1 total, 1 partial, 0 rejected"), "{text}");
}

#[test]
fn a_rejection_is_listed_with_its_reason_and_exits_non_zero() {
    let report = report_with(
        &[("spin", AdmissionState::Rejected)],
        vec![no_descent("spin")],
    );
    let tally = TerminationTally::from_report(&report);

    assert_eq!(tally.rejected, 1);
    assert_eq!(tally.total, 0);
    assert_ne!(
        format!("{:?}", tally.exit()),
        format!("{:?}", std::process::ExitCode::SUCCESS)
    );

    let text = render_report(&report, &tally, &[], false);
    assert!(
        text.contains("✗ 1 definition(s) not admitted of 1"),
        "{text}"
    );
    assert!(
        text.contains("cannot prove termination of `spin`"),
        "{text}"
    );
    assert!(text.contains("not a known strict subterm"), "{text}");
}

#[test]
fn each_admission_state_is_counted_under_its_own_heading() {
    // Asymmetric on purpose: with one of each state, swapping any two of the
    // three predicates would produce the same three numbers.
    let report = report_with(
        &[
            ("a", AdmissionState::Total),
            ("b", AdmissionState::Total),
            ("c", AdmissionState::Total),
            ("d", AdmissionState::Partial),
            ("e", AdmissionState::Partial),
            ("f", AdmissionState::Rejected),
        ],
        vec![no_descent("f")],
    );
    let tally = TerminationTally::from_report(&report);

    assert_eq!(tally.definitions, 6);
    assert_eq!(tally.total, 3);
    assert_eq!(tally.partial, 2);
    assert_eq!(tally.rejected, 1);
}

#[test]
fn a_proof_failure_alone_still_exits_non_zero() {
    // The proof is not itself rejected — its dependency is partial — so the
    // rejected count is zero and only the proof-failure count is not.
    let report = report_with(
        &[
            ("thm", AdmissionState::Partial),
            ("spin", AdmissionState::Partial),
        ],
        vec![partial_in_proof("thm")],
    );
    let tally = TerminationTally::from_report(&report);

    assert_eq!(tally.rejected, 0);
    assert_eq!(tally.proof_failures, 1);
    assert_ne!(
        format!("{:?}", tally.exit()),
        format!("{:?}", std::process::ExitCode::SUCCESS)
    );
}

#[test]
fn verbose_adds_the_group_listing_and_the_taint_census() {
    let mut report = report_with(&[("spin", AdmissionState::Partial)], Vec::new());
    report.tainted = vec!["spin".to_string(), "wrapper".to_string()];

    let tally = TerminationTally::from_report(&report);
    let quiet = render_report(&report, &tally, &[], false);
    let loud = render_report(&report, &tally, &[], true);

    assert!(!quiet.contains("Recursive groups:"), "{quiet}");
    assert!(loud.contains("Recursive groups:\n  spin"), "{loud}");
    assert!(loud.contains("Tainted: spin, wrapper"), "{loud}");
}

#[test]
fn an_empty_taint_census_is_omitted_rather_than_printed_blank() {
    let report = report_with(&[("len", AdmissionState::Total)], Vec::new());
    let tally = TerminationTally::from_report(&report);

    let loud = render_report(&report, &tally, &[], true);
    assert!(!loud.contains("Tainted:"), "{loud}");
    assert!(!loud.contains("Taint seeds:"), "{loud}");
}

// 3.10.26c AC8: `--verbose` labels each seed `partial` or `foreign`, the
// foreign one with its extern symbol; the quiet form prints neither.
#[test]
fn verbose_labels_each_taint_seed_partial_or_foreign() {
    let mut report = report_with(&[("spin", AdmissionState::Partial)], Vec::new());
    report.tainted = vec!["g".to_string(), "spin".to_string(), "wrapper".to_string()];
    report
        .foreign_seeds
        .insert("g".to_string(), "tg_type_nat".to_string());
    let tally = TerminationTally::from_report(&report);
    let annotated = vec!["spin".to_string()];

    let loud = render_report(&report, &tally, &annotated, true);
    assert!(
        loud.contains("Taint seeds:\n  g — foreign (tg_type_nat)\n  spin — partial\n"),
        "{loud}"
    );
    assert!(
        !loud.contains("wrapper —"),
        "an inherited taint is not a seed: {loud}"
    );
    let quiet = render_report(&report, &tally, &annotated, false);
    assert!(!quiet.contains("Taint seeds:"), "{quiet}");
}

#[test]
fn an_annotated_seed_that_also_calls_foreign_code_is_labelled_once() {
    let mut report = report_with(&[("spin", AdmissionState::Partial)], Vec::new());
    report
        .foreign_seeds
        .insert("spin".to_string(), "g".to_string());
    let tally = TerminationTally::from_report(&report);

    let loud = render_report(&report, &tally, &["spin".to_string()], true);
    assert!(loud.contains("  spin — partial\n"), "{loud}");
    assert!(!loud.contains("spin — foreign"), "{loud}");
}

// 3.10.26c AC9: a foreign rejection is a proof failure to the census, so the
// tool exits non-zero on it exactly as on a `#[partial]` one.
#[test]
fn a_foreign_proof_failure_alone_still_exits_non_zero() {
    let foreign = TerminationFailure {
        function: "thm".to_string(),
        group: Vec::new(),
        span: None,
        reason: FailureReason::ForeignInProof {
            extern_symbol: "g".to_string(),
            via: vec!["g".to_string()],
        },
    };
    let report = report_with(&[("thm", AdmissionState::Partial)], vec![foreign]);
    let tally = TerminationTally::from_report(&report);

    assert_eq!(tally.proof_failures, 1);
    assert_ne!(
        format!("{:?}", tally.exit()),
        format!("{:?}", std::process::ExitCode::SUCCESS)
    );
}
