//! Tests for the two text predicates.
//!
//! Every case is a literal string, so these assert the rules themselves rather
//! than the checkout they happen to run against.

use super::*;

// ---------------------------------------------------------------------------
// Guard (a)'s scan
// ---------------------------------------------------------------------------

#[test]
fn a_runtime_assertion_call_is_detected_whatever_the_wrapper() {
    assert!(calls_a_runtime_assertion("    assert_eq_string(x, \"a\")"));
    assert!(calls_a_runtime_assertion("assert_eq_nat(n, 3)"));
    assert!(calls_a_runtime_assertion("let _ = assert_ne(a, b);"));
    assert!(calls_a_runtime_assertion("assert(cond)"));
    assert!(calls_a_runtime_assertion("assert_some(opt)"));
}

/// The elaboration-time assertions a tier-3 file is *made of* must not trip
/// guard (a) — otherwise no file could be declared tier 3 at all and the guard
/// would have eaten the feature it protects.
#[test]
fn elaboration_time_assertions_do_not_count_as_runtime_ones() {
    let compare_result = "\
pub fn test_compare_result_equal_typechecks() -> Unit {
    let r: CompareResult = Equal;
    expect_type(r, \"CompareResult\")
}
";
    assert!(!calls_a_runtime_assertion(compare_result));
    assert!(!calls_a_runtime_assertion("expect_error(e, \"E0001\")"));

    let list_ops = "\
pub fn test_list_len_three() -> Unit {
    let xs: List<Nat> = Cons(1, Nil);
    let n: Nat = list_len(xs);
    let _ = n;
}
";
    assert!(!calls_a_runtime_assertion(list_ops));
}

/// Importing an assertion without calling it must not force tier 5: the scan
/// looks for an *application*, so a `use` line alone is not a call.
#[test]
fn importing_an_assertion_without_calling_it_is_not_a_call() {
    assert!(!calls_a_runtime_assertion(
        "use driver::ffi::test::{assert_eq_string, assert_eq_nat};"
    ));
}

/// An identifier that merely *ends* in `assert` is not one, or a helper named
/// `reassert_all(..)` would silently force its file to tier 5.
#[test]
fn a_name_that_only_ends_in_assert_is_not_an_assertion_call() {
    assert!(!calls_a_runtime_assertion("reassert_all(x)"));
    assert!(!calls_a_runtime_assertion("my_assert(x)"));
}

/// A bare mention with no application is not a call — the scan must look at
/// what follows the identifier, not just find the substring.
#[test]
fn a_mention_without_an_application_is_not_a_call() {
    assert!(!calls_a_runtime_assertion(
        "// this file deliberately has no assert of any kind"
    ));
    assert!(!calls_a_runtime_assertion("let assert_count = 0;"));
}

// ---------------------------------------------------------------------------
// Guard (a) over an entry's test companions (ADR 2.10.26j D8)
// ---------------------------------------------------------------------------

/// Guard (a)'s verdict on a tier-3 entry whose tree yields `companions`.
fn tier_3_verdict(entry: &str, companions: &[String]) -> bool {
    let asserts = calls_a_runtime_assertion(&guarded_source(entry, companions));
    super::super::decide_tier("k.tg", Some(3), false, asserts).is_ok()
}

const TYPES_ONLY_ENTRY: &str = "pub mod widget;\nfn test_t() -> Unit { expect_type(1, \"Nat\") }\n";

// ADR 2.10.26j AC7: a companion's runtime assertion is the entry's.
#[test]
fn a_tier_3_entry_whose_companion_asserts_is_refused() {
    let companion = "fn test_w() -> Unit { assert_eq_nat(1, 1) }\n".to_string();
    assert!(
        tier_3_verdict(TYPES_ONLY_ENTRY, &[]),
        "the entry alone is tier 3"
    );
    assert!(!tier_3_verdict(TYPES_ONLY_ENTRY, &[companion]));
}

// ADR 2.10.26j AC7: a companion that only `expect_type`s keeps tier 3.
#[test]
fn a_tier_3_entry_whose_companion_only_expects_types_is_not() {
    let companion = "fn test_w() -> Unit { expect_type(true, \"Bool\") }\n".to_string();
    assert!(tier_3_verdict(TYPES_ONLY_ENTRY, &[companion]));
}

// ADR 2.10.26j AC7: only the companions of the entry's own tree are scanned —
// an asserting companion of a module the entry never declares is not.
#[test]
fn a_tier_3_entry_is_not_refused_for_a_companion_outside_its_tree() {
    use albite_bootstrap::driver::{companion_index, companions_in_tree};
    use std::path::PathBuf;
    let files: Vec<PathBuf> = ["d/main.tg", "d/widget.tg", "d/far.tg", "d/far_test.tg"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let tree = [PathBuf::from("d/main.tg"), PathBuf::from("d/widget.tg")];
    let in_tree = companions_in_tree(&tree, &companion_index(&files));
    assert!(
        in_tree.is_empty(),
        "far_test.tg's sibling is not in the tree"
    );
    let sources: Vec<String> = in_tree
        .iter()
        .map(|_| "assert_eq_nat(1, 2)".to_string())
        .collect();
    assert!(tier_3_verdict(TYPES_ONLY_ENTRY, &sources));
}

#[test]
fn guarded_source_joins_the_entry_and_each_companion_on_a_newline() {
    let joined = guarded_source("a", &["b".to_string(), "c".to_string()]);
    assert_eq!(joined, "a\nb\nc");
    assert_eq!(guarded_source("a", &[]), "a");
}

// ---------------------------------------------------------------------------
// The `must_declare` glob
// ---------------------------------------------------------------------------

#[test]
fn the_glob_matches_within_a_segment_and_not_across_one() {
    assert!(glob_matches(
        "src/compiler/test_*.tg",
        "src/compiler/test_strmap.tg"
    ));
    assert!(!glob_matches(
        "src/compiler/test_*.tg",
        "src/compiler/driver/test_x.tg"
    ));
    assert!(!glob_matches("src/compiler/test_*.tg", "tests/test_x.tg"));
    assert!(!glob_matches(
        "src/compiler/test_*.tg",
        "src/compiler/mustfail_ast_compare.tg"
    ));
}

/// The prefix and suffix both have to hold — a matcher checking only one would
/// pass the test above for the wrong reason.
#[test]
fn the_glob_requires_both_the_prefix_and_the_suffix() {
    assert!(!glob_matches("src/test_*.tg", "src/other_x.tg"));
    assert!(!glob_matches("src/test_*.tg", "src/test_x.rs"));
    // A segment shorter than prefix+suffix must not match by overlapping them.
    assert!(!glob_matches("src/ab*ba.tg", "src/aba.tg"));
}

#[test]
fn a_pattern_with_no_star_is_an_exact_match() {
    assert!(glob_matches("tests/try_block.tg", "tests/try_block.tg"));
    assert!(!glob_matches("tests/try_block.tg", "tests/type_alias.tg"));
}
