//! The `check` verdict line (ADR 18.9.26g AC2, ADR 2.10.26d D11, ADR 13.10.26g AC3).

use albite_core::terms::analysis::SorryCounts;

use super::check_verdict_line;
use crate::elaborate::kernel::{ExemptTally, RegisterClass};

fn none_exempt() -> ExemptTally {
    ExemptTally::default()
}

fn three_needing_delta() -> ExemptTally {
    let mut tally = ExemptTally::default();
    for _ in 0..3 {
        tally.record(RegisterClass::NeedsDelta);
    }
    tally
}

fn one_authored() -> SorryCounts {
    SorryCounts {
        authored: 1,
        ..SorryCounts::default()
    }
}

// 18.9.26g AC2: a program with no hole reads all OK.
#[test]
fn a_hole_free_program_reads_all_ok() {
    assert_eq!(
        check_verdict_line("a.tg", 3, &SorryCounts::default(), 0, &none_exempt()),
        "✓ a.tg: 3 definition(s), all OK"
    );
}

// 18.9.26g AC2: both counts print, and unclassified folds into synthesised.
#[test]
fn the_verdict_line_carries_both_counts() {
    let sorry = SorryCounts {
        authored: 2,
        synthesised: 3,
        unclassified: 1,
    };
    assert_eq!(
        check_verdict_line("a.tg", 6, &sorry, 0, &none_exempt()),
        "⚠ a.tg: 6 definition(s), contains sorry (2 authored, 4 synthesised)"
    );
}

// 18.9.26g AC2: one hole of any class is enough to leave all OK.
#[test]
fn a_single_hole_of_any_class_is_reported() {
    let one_of_each = [
        one_authored(),
        SorryCounts {
            synthesised: 1,
            ..SorryCounts::default()
        },
        SorryCounts {
            unclassified: 1,
            ..SorryCounts::default()
        },
    ];
    for sorry in one_of_each {
        let line = check_verdict_line("a.tg", 1, &sorry, 0, &none_exempt());
        assert!(line.contains("contains sorry"), "{line}");
    }
}

// 2.10.26d D11: `check` names the theorems it did not kernel-check, by class.
#[test]
fn an_exempt_theorem_leaves_all_ok_and_is_named_by_class() {
    assert_eq!(
        check_verdict_line(
            "k.tg",
            8,
            &SorryCounts::default(),
            0,
            &three_needing_delta()
        ),
        "⚠ k.tg: 8 definition(s), 3 theorem(s) kernel-exempt (needs-delta 3)"
    );
}

// 2.10.26d D11: with a hole too, both clauses print and `contains sorry` stays.
#[test]
fn an_exempt_theorem_beside_a_hole_keeps_the_sorry_clause() {
    let mut exempt = three_needing_delta();
    exempt.record(RegisterClass::CitesExempt);
    assert_eq!(
        check_verdict_line("k.tg", 9, &one_authored(), 0, &exempt),
        "⚠ k.tg: 9 definition(s), contains sorry (1 authored, 0 synthesised), \
         4 theorem(s) kernel-exempt (needs-delta 3, cites-exempt 1)"
    );
}

// 13.10.26g AC3: the resting clause follows the sorry clause.
#[test]
fn resting_theorems_are_counted_after_the_sorry_clause() {
    assert_eq!(
        check_verdict_line("ax.tg", 4, &one_authored(), 3, &none_exempt()),
        "⚠ ax.tg: 4 definition(s), contains sorry (1 authored, 0 synthesised), \
         3 theorem(s) rest on an axiom or sorry"
    );
}

// 13.10.26g AC3: an unused axiom keeps today's line.
#[test]
fn an_unused_axiom_keeps_the_sorry_line_alone() {
    assert_eq!(
        check_verdict_line("ax.tg", 1, &one_authored(), 0, &none_exempt()),
        "⚠ ax.tg: 1 definition(s), contains sorry (1 authored, 0 synthesised)"
    );
}

// 13.10.26g AC3: the resting clause sits between the sorry and exempt clauses,
// and alone it is enough to leave all OK.
#[test]
fn resting_theorems_sit_between_the_sorry_and_exempt_clauses() {
    assert_eq!(
        check_verdict_line("k.tg", 9, &one_authored(), 1, &three_needing_delta()),
        "⚠ k.tg: 9 definition(s), contains sorry (1 authored, 0 synthesised), \
         1 theorem(s) rest on an axiom or sorry, 3 theorem(s) kernel-exempt (needs-delta 3)"
    );
    assert_eq!(
        check_verdict_line("k.tg", 2, &SorryCounts::default(), 2, &none_exempt()),
        "⚠ k.tg: 2 definition(s), 2 theorem(s) rest on an axiom or sorry"
    );
}
