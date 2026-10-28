//! `explain` knows the tactic codes (ADR 2.10.26c D8, AC 6).
//!
//! `TacticErrorKind` is nested inside `ElabErrorKind`, whose arm for it
//! (`Tactic(t) => t.code()`) carries no literal code, so `catalogue.rs`'s parse
//! of `codes.rs` skips it. This oracle reads the nested kind's own table, as
//! `build_codes.rs` reads `BuildErrorKind`'s.

use super::catalogue::exact_code_arms;
use crate::explain::error_catalogue;

/// `(kind name, code)` for every arm of `TacticErrorKind::code()` — exactly 6.
pub(super) fn tactic_kind_code_pairs() -> Vec<(String, String)> {
    exact_code_arms(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/elaborate/error/kind/tactic.rs"
        ),
        "TacticErrorKind::",
        6,
    )
}

// ADR 2.10.26c AC6: each new code and kind name resolves, to one entry —
// E0081 from `codes.rs`, E0082/E0083 from the nested table, and `rewrite`'s
// E0085 from the same table (ADR 2.10.26e D7).
#[test]
fn the_tactic_codes_resolve_by_code_and_by_name() {
    for (code, kind) in [
        ("E0081", "KernelRefusedProof"),
        ("E0082", "TacticNotApplicable"),
        ("E0083", "UnsolvedGoals"),
        ("E0085", "RewriteNoOccurrence"),
        ("E0086", "SimpStepBound"),
        ("E0087", "RingUnequal"),
        ("E0088", "OmegaUnproved"),
    ] {
        assert_eq!(error_catalogue::resolve_query_for_test(code), Some(kind));
        assert_eq!(error_catalogue::resolve_query_for_test(kind), Some(kind));
    }
    let nested: Vec<String> = tactic_kind_code_pairs()
        .into_iter()
        .map(|(_, code)| code)
        .collect();
    assert_eq!(
        nested,
        ["E0082", "E0083", "E0085", "E0086", "E0087", "E0088"]
    );
}

// ADR 4.10.26e AC3: E0086 is explained, and E0082's entry names `simp`'s
// two refusals.
#[test]
fn e0086_is_explained_and_e0082_names_simp() {
    let (detail, _, code) =
        error_catalogue::entry_body_for_test("SimpStepBound").expect("E0086's entry");
    assert_eq!(code, "E0086");
    assert!(detail.contains("64"), "the bound is named");
    assert!(detail.contains("undo each other"));
    let (detail, _, _) =
        error_catalogue::entry_body_for_test("TacticNotApplicable").expect("E0082's entry");
    assert!(detail.contains("`simp [r, …]`"));
    assert!(detail.contains("swap its variables"));
}

// ADR 4.10.26g AC3: E0087 is explained — the false equation, the atoms it
// cannot see inside, and the library it cites.
#[test]
fn e0087_is_explained() {
    let (detail, example, code) =
        error_catalogue::entry_body_for_test("RingUnequal").expect("E0087's entry");
    assert_eq!(code, "E0087");
    assert!(detail.contains("different polynomials"));
    assert!(detail.contains("`(n - 1) + 1 = n` is refused"));
    assert!(detail.contains("`mod nat_arith;`"));
    assert!(detail.contains("`pow`"));
    assert!(example.contains("E0087"));
}

// ADR 6.10.26f AC3: E0088 is explained — its three messages, the fragment
// and the library it cites.
#[test]
fn e0088_is_explained() {
    let (detail, example, code) =
        error_catalogue::entry_body_for_test("OmegaUnproved").expect("E0088's entry");
    assert_eq!(code, "E0088");
    assert!(detail.contains("three messages"));
    assert!(detail.contains("no witness over the oriented hypotheses"));
    assert!(detail.contains("`mod nat_arith;`"));
    assert!(example.contains("E0088"));
}

// ADR 3.10.26f D5: E0082's entry, now its own sibling function, carries a
// sentence for every tactic that can refuse a goal, `cases` included.
#[test]
fn e0082_explains_each_refusing_tactic() {
    let (detail, _, code) =
        error_catalogue::entry_body_for_test("TacticNotApplicable").expect("E0082's entry");
    assert_eq!(code, "E0082");
    for tactic in [
        "`intro x`",
        "`apply e`",
        "`rewrite [r]`",
        "`induction x`",
        "`cases x { … }`",
        "`ring`",
        "`omega`",
    ] {
        assert!(detail.contains(tactic), "{tactic} is not explained");
    }
    assert!(detail.contains("split it by `induction`"));
}
