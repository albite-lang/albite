//! 28.9.26f AC3 — the rule is the Decisions' rule, at its exact boundaries.
//! Each arm: admitted with both boundaries met with equality, parked one below
//! the floor, parked one short of the share.

use super::decide::*;
use super::scan::{BlobShapeCounts, SmallAdtCounts};

/// Counts with `small` memory-buffer sites at N = 0 (4-byte blobs) and
/// `large` at N = 16 (20-byte blobs, not small).
fn with_buffers(small: u64, large: u64) -> SmallAdtCounts {
    let mut counts = SmallAdtCounts::default();
    counts.shapes.insert(
        0,
        BlobShapeCounts {
            sret_returns: small,
            ..Default::default()
        },
    );
    counts.shapes.insert(
        16,
        BlobShapeCounts {
            indirect_params: large,
            ..Default::default()
        },
    );
    counts
}

/// Counts with `pointer` pointer-payload constructions out of `total` at N = 8.
fn with_constructions(pointer: u64, total: u64) -> SmallAdtCounts {
    let mut counts = SmallAdtCounts::default();
    counts.pointer_payload_constructions = pointer;
    counts.shapes.insert(
        8,
        BlobShapeCounts {
            constructions: total,
            ..Default::default()
        },
    );
    counts
}

fn with_nullary(nullary: u64, total: u64) -> SmallAdtCounts {
    let mut counts = SmallAdtCounts::default();
    counts.nullary_mu_alloc_sites = nullary;
    counts.mu_alloc_sites = total;
    counts
}

#[test]
fn the_named_thresholds_are_the_decisions_numbers() {
    assert_eq!(FLOOR, 20);
    assert_eq!(NULLARY_SHARE_DEN, 10);
    assert_eq!(SMALL_SUM_SHARE_DEN, 20);
    assert_eq!(NICHE_SHARE_DEN, 20);
}

/// 28.9.26f AC3 — kills `>=` → `>` on both comparisons.
#[test]
fn admitted_holds_at_both_boundaries_and_fails_one_below_each() {
    assert!(
        admitted(20, 200, 10),
        "part == FLOOR and part × den == whole"
    );
    assert!(!admitted(19, 190, 10), "one below the floor, share met");
    assert!(!admitted(20, 201, 10), "floor met, part × den == whole − 1");
}

#[test]
fn nullary_cells_arm_boundaries() {
    assert_eq!(
        decide(&with_nullary(20, 200)).nullary_cells,
        ArmVerdict::Admit
    );
    assert_eq!(
        decide(&with_nullary(19, 190)).nullary_cells,
        ArmVerdict::Park
    );
    assert_eq!(
        decide(&with_nullary(20, 201)).nullary_cells,
        ArmVerdict::Park
    );
}

#[test]
fn small_sum_registers_arm_boundaries() {
    // whole = small + large: 20 × 20 == 400 exactly.
    assert_eq!(
        decide(&with_buffers(20, 380)).small_sum_registers,
        ArmVerdict::Admit
    );
    assert_eq!(
        decide(&with_buffers(19, 361)).small_sum_registers,
        ArmVerdict::Park
    );
    assert_eq!(
        decide(&with_buffers(20, 381)).small_sum_registers,
        ArmVerdict::Park
    );
}

#[test]
fn pointer_niche_arm_boundaries() {
    assert_eq!(
        decide(&with_constructions(20, 400)).pointer_niche,
        ArmVerdict::Admit
    );
    assert_eq!(
        decide(&with_constructions(19, 380)).pointer_niche,
        ArmVerdict::Park
    );
    assert_eq!(
        decide(&with_constructions(20, 401)).pointer_niche,
        ArmVerdict::Park
    );
}

#[test]
fn each_arm_reads_only_its_own_counts() {
    let nullary = decide(&with_nullary(20, 200));
    assert_eq!(nullary.small_sum_registers, ArmVerdict::Park);
    assert_eq!(nullary.pointer_niche, ArmVerdict::Park);
    let buffers = decide(&with_buffers(20, 0));
    assert_eq!(buffers.nullary_cells, ArmVerdict::Park);
    assert_eq!(buffers.pointer_niche, ArmVerdict::Park);
}

#[test]
fn b19_closes_with_no_arm_and_rescopes_with_any_one() {
    assert_eq!(decide(&SmallAdtCounts::default()).b19, B19Verdict::Close);
    assert_eq!(decide(&with_nullary(20, 200)).b19, B19Verdict::Rescope);
    assert_eq!(decide(&with_buffers(20, 0)).b19, B19Verdict::Rescope);
    assert_eq!(decide(&with_constructions(20, 20)).b19, B19Verdict::Rescope);
}

#[test]
fn fallbacks_map_and_never_move_b19() {
    let mut counts = SmallAdtCounts::default();
    assert_eq!(decide(&counts).fallbacks, FallbackVerdict::Unknown);
    counts.pointer_fallbacks = Some(0);
    assert_eq!(decide(&counts).fallbacks, FallbackVerdict::None);
    counts.pointer_fallbacks = Some(1);
    let verdict = decide(&counts);
    assert_eq!(verdict.fallbacks, FallbackVerdict::RouteToCorrectness);
    assert_eq!(verdict.b19, B19Verdict::Close, "fallbacks never change b19");
}

#[test]
fn verdict_spellings_are_the_json_values() {
    assert_eq!(ArmVerdict::Admit.as_str(), "admit");
    assert_eq!(ArmVerdict::Park.as_str(), "park");
    assert_eq!(FallbackVerdict::None.as_str(), "none");
    assert_eq!(
        FallbackVerdict::RouteToCorrectness.as_str(),
        "route-to-correctness"
    );
    assert_eq!(FallbackVerdict::Unknown.as_str(), "unknown");
    assert_eq!(B19Verdict::Rescope.as_str(), "rescope");
    assert_eq!(B19Verdict::Close.as_str(), "close");
}
