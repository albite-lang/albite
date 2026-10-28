//! `explain` knows the package codes (ADR 3.10.26m D8, AC 7).
//!
//! `PackageErrorKind` is nested inside `ElabErrorKind`, whose arm for it
//! (`Package(p) => p.code()`) carries no literal code, so `catalogue.rs`'s
//! parse of `codes.rs` skips it. This oracle reads the nested kind's own
//! table, as `tactic_codes.rs` reads `TacticErrorKind`'s.

use super::catalogue::exact_code_arms;
use crate::explain::error_catalogue;

/// `(kind name, code)` for every arm of `PackageErrorKind::code()` — exactly 5.
pub(super) fn package_kind_code_pairs() -> Vec<(String, String)> {
    exact_code_arms(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/elaborate/error/kind/package.rs"
        ),
        "PackageErrorKind::",
        5,
    )
}

// ADR 3.10.26m AC7: the four moved kinds keep their codes, the cycle takes
// E0124, and each resolves by code and by name to one entry.
#[test]
fn the_package_codes_resolve_by_code_and_by_name() {
    let pairs = package_kind_code_pairs();
    let expected = [
        ("ManifestInvalid", "E0120"),
        ("DependencyNotFound", "E0121"),
        ("PackageNameConflict", "E0122"),
        ("CrossPackageNameCollision", "E0123"),
        ("DependencyCycle", "E0124"),
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
}

// ADR 3.10.26m D3/D7: the cycle's entry has a worked example, and the
// manifest entry no longer promises nested dependencies for a later slice.
#[test]
fn the_cycle_is_explained_and_the_slice_2_sentence_is_gone() {
    let (detail, example, code) =
        error_catalogue::entry_body_for_test("DependencyCycle").expect("E0124's entry");
    assert_eq!(code, "E0124");
    assert!(detail.contains("cycle"), "{detail}");
    assert!(
        example.contains("lib_a") && example.contains("lib_b"),
        "{example}"
    );
    let (manifest, _, _) =
        error_catalogue::entry_body_for_test("ManifestInvalid").expect("E0120's entry");
    assert!(!manifest.contains("slice 2"), "{manifest}");
}
