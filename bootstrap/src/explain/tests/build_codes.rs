//! `explain` knows every build code (ADR 30.9.26j AC4).
//!
//! The oracle reads the real `BuildErrorKind::code()` arms, as the elaboration
//! and syntax oracles in `catalogue.rs` read theirs (29.9.26f D6), so a new
//! build kind with no explanation fails here.

use super::catalogue::exact_code_arms;
use crate::explain::error_catalogue;

/// `(kind name, code)` for every arm of `BuildErrorKind::code()` — exactly 7.
pub(super) fn build_kind_code_pairs() -> Vec<(String, String)> {
    exact_code_arms(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/driver/diagnostics/compile_failure/mod.rs"
        ),
        "BuildErrorKind::",
        7,
    )
}

// 30.9.26j AC4
/// Every build code and kind name resolves, to the same entry.
#[test]
fn every_build_code_and_kind_name_resolve_to_one_entry() {
    for (kind, code) in build_kind_code_pairs() {
        assert_eq!(
            error_catalogue::resolve_query_for_test(&code),
            Some(kind.as_str()),
            "code `{code}` must resolve to `{kind}`"
        );
        assert_eq!(
            error_catalogue::resolve_query_for_test(&kind),
            Some(kind.as_str()),
            "kind `{kind}` must resolve by name"
        );
    }
    assert_eq!(
        error_catalogue::resolve_query_for_test("E0144"),
        error_catalogue::resolve_query_for_test("ToolFailed")
    );
}

// 30.9.26j AC4
/// The listing has a *Build* group carrying all seven codes, and the moved
/// syntax entries still resolve.
#[test]
fn the_listing_has_a_build_group_with_every_build_code() {
    let listing = error_catalogue::render_error_list();
    let group = listing
        .split("\n\n")
        .find(|block| block.starts_with("Build:"))
        .unwrap_or_else(|| panic!("no Build group in the listing:\n{listing}"));
    for (kind, code) in build_kind_code_pairs() {
        assert!(
            group.contains(&format!("{code}   {kind}")),
            "`{code} {kind}` missing from the Build group:\n{group}"
        );
    }
    assert_eq!(group.lines().count(), 1 + 7, "Build group:\n{group}");
    assert_eq!(
        error_catalogue::resolve_query_for_test("E0117"),
        Some("Lexical")
    );
}
