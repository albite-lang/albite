//! `explain` knows the sort codes (ADR 4.10.26i D3, D5).
//!
//! `SortErrorKind` is nested inside `ElabErrorKind`, whose arm for it
//! (`Sort(s) => s.code()`) carries no literal code, so `catalogue.rs`'s parse
//! of `codes.rs` skips it. This oracle reads the nested kind's own table, as
//! `tactic_codes.rs` reads `TacticErrorKind`'s.

use super::catalogue::exact_code_arms;
use crate::explain::error_catalogue;

/// `(kind name, code)` for every arm of `SortErrorKind::code()` — exactly 7.
pub(super) fn sort_kind_code_pairs() -> Vec<(String, String)> {
    exact_code_arms(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/elaborate/error/kind/sort.rs"
        ),
        "SortErrorKind::",
        7,
    )
}

// ADR 4.10.26i D3/D5: E0066 and E0067 resolve by code and by name, and the
// refusal's entry carries the sentence both compilers share. 6.10.26b AC1:
// E0068 resolves the same way. 6.10.26d AC3: so does E0023, TermTypeConfusion.
// 6.10.26h AC2: and E0069, PredicateKindRefused. 7.10.26l AC2: and E0024,
// SumIntroNotDisjunction. 7.10.26n AC2: and E0025, DependentFunctionField.
#[test]
fn the_sort_codes_resolve_by_code_and_by_name() {
    let pairs = sort_kind_code_pairs();
    let expected = [
        ("SortUsedAsType", "E0066"),
        ("FamilySortMismatch", "E0067"),
        ("TypeParameterApplied", "E0068"),
        ("TermTypeConfusion", "E0023"),
        ("PredicateKindRefused", "E0069"),
        ("SumIntroNotDisjunction", "E0024"),
        ("DependentFunctionField", "E0025"),
    ];
    let as_strs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(kind, code)| (kind.as_str(), code.as_str()))
        .collect();
    assert_eq!(as_strs, expected);
    for (kind, code) in expected {
        assert_eq!(error_catalogue::resolve_query_for_test(code), Some(kind));
        assert_eq!(error_catalogue::resolve_query_for_test(kind), Some(kind));
    }
    let (detail, _, _) =
        error_catalogue::entry_body_for_test("SortUsedAsType").expect("E0066's entry");
    assert!(detail.contains("is a sort"), "{detail}");
}
