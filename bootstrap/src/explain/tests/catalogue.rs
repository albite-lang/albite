//! Error-catalogue coverage and the code lookup (ADR 8.8.26a).
//!
//! The oracle here reads the real `ElabErrorKind::code()` arms rather than a
//! list of variant names typed beside it — see [`kind_code_pairs`].
use super::unlisted::DELIBERATELY_UNLISTED;
use crate::explain::error_catalogue;

// ─────────────────────────────────────────────────────────────────────────────
// Error catalogue tests
// ─────────────────────────────────────────────────────────────────────────────
/// Verify that every known error kind name has a corresponding explanation.
#[test]
fn all_error_kinds_have_explanations() {
    let names = error_catalogue::all_known_names();
    assert!(
        !names.is_empty(),
        "CATEGORIES should contain at least one error kind"
    );
    for name in &names {
        assert!(
            error_catalogue::get_explanation_by_name(name),
            "missing explanation for error kind: {}",
            name
        );
    }
}

/// Every code `ElabErrorKind::code()` produces must resolve to an explanation.
///
/// The oracle is the **real** `code()` arms, parsed out of
/// `elaborate/error/kind.rs` — not a list of variant names typed beside this
/// test. That is what the previous version did, and a hand-maintained list
/// cannot detect a variant missing from itself: 15 of 53 kinds had a code and a
/// rendered message but no explanation, and the "exhaustive" test was green
/// (ADR 8.8.26a).
///
/// Source-reading is deliberate and guarded: `code()` is an exhaustive match,
/// so it is the one place a new variant *must* appear, and the assertions below
/// fail loudly rather than skipping if the file cannot be read or yields
/// nothing.
#[test]
fn every_error_code_resolves() {
    let pairs = kind_code_pairs();
    for (kind, code) in &pairs {
        if DELIBERATELY_UNEXPLAINED.iter().any(|(k, _)| k == kind) {
            continue;
        }
        assert!(
            error_catalogue::resolve_query_for_test(code).is_some(),
            "code `{code}` (`{kind}`) does not resolve — every code a user can \
             see in compiler output must explain itself"
        );
        assert!(
            error_catalogue::resolve_query_for_test(kind).is_some(),
            "kind `{kind}` does not resolve by name"
        );
    }
}

/// Kinds with no explanation, each with the reason it has none.
///
/// An exclusion is a decision, so it is written down. Deleting an entry here
/// must make [`every_error_code_resolves`] fail — which is what
/// [`the_exclusion_list_is_load_bearing`] pins.
const DELIBERATELY_UNEXPLAINED: &[(&str, &str)] = &[(
    "Other",
    "the catch-all for messages with no structured kind; there is nothing \
     general to say about it",
)];

#[test]
fn the_exclusion_list_is_load_bearing() {
    for (kind, reason) in DELIBERATELY_UNEXPLAINED {
        assert!(
            error_catalogue::resolve_query_for_test(kind).is_none(),
            "`{kind}` is excluded as unexplainable ({reason}) but DOES resolve — \
             drop it from the exclusion list"
        );
    }
}

/// Every catalogued entry names a kind that still exists.
///
/// The other direction: a renamed or deleted variant leaves an entry that can
/// never be reached from a diagnostic.
#[test]
fn no_catalogue_entry_names_a_kind_that_no_longer_exists() {
    let pairs = kind_code_pairs();
    for name in error_catalogue::all_known_names() {
        assert!(
            pairs.iter().any(|(kind, _)| kind == name),
            "catalogue entry `{name}` is not a live ElabErrorKind variant"
        );
    }
}

/// Every explanation carries substance — an entry stubbed to `""` is worse than
/// no entry, because the coverage tests above would call it covered.
#[test]
fn every_explanation_has_a_detail_and_an_example() {
    for name in error_catalogue::all_known_names() {
        let (detail, example, code) = error_catalogue::entry_body_for_test(name)
            .unwrap_or_else(|| panic!("`{name}` is listed but has no explanation"));
        assert!(detail.len() > 40, "`{name}` detail is a stub: {detail:?}");
        assert!(!example.is_empty(), "`{name}` has no example");
        assert!(
            code.len() == 5 && code.starts_with(['E', 'W']),
            "`{name}` has a malformed code: {code:?}"
        );
    }
}

/// `(kind name, code)` for every arm of `ElabErrorKind::code()` and of
/// `ParseErrorKind::code()` (ADR 29.9.26f D6: the second source).
pub(super) fn kind_code_pairs() -> Vec<(String, String)> {
    let elab = code_arms(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/elaborate/error/kind/codes.rs"
        ),
        "ElabErrorKind::",
    );
    // Non-vacuity: a parse that silently matched nothing would make every
    // coverage assertion above pass over an empty set.
    assert!(
        elab.len() >= 50,
        "the ElabErrorKind code() oracle parsed only {} arm(s) — the match shape changed",
        elab.len()
    );
    let tactic = super::tactic_codes::tactic_kind_code_pairs();
    let package = super::package_codes::package_kind_code_pairs();
    let parse = parse_kind_code_pairs();
    let build = super::build_codes::build_kind_code_pairs();
    let sort = super::sort_codes::sort_kind_code_pairs();
    let mutable = nested_kind_code_pairs("mutable.rs", "MutableErrorKind::", 4);
    let tail = tactic
        .into_iter()
        .chain(package)
        .chain(parse)
        .chain(build)
        .chain(sort)
        .chain(nested_kind_code_pairs("handle.rs", "HandleErrorKind::", 2))
        .chain(nested_kind_code_pairs("region.rs", "RegionErrorKind::", 3))
        .chain(mutable);
    elab.into_iter().chain(tail).collect()
}

/// `(kind name, code)` for every arm of `elaborate/error/kind/<file>`'s nested
/// `code()` table, exactly `arms` of them (ADR 7.10.26t D7, 9.10.26o D4).
/// Here rather than in a sibling: `tests/` is at its cap.
fn nested_kind_code_pairs(file: &str, prefix: &str, arms: usize) -> Vec<(String, String)> {
    let path = format!(
        "{}/src/elaborate/error/kind/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    exact_code_arms(&path, prefix, arms)
}

// 7.10.26t AC5: E0092 and E0093 resolve by code and by name to their entries.
#[test]
fn the_handle_codes_resolve_by_code_and_by_name() {
    let expected = [
        ("UsedAfterConsumed", "E0092"),
        ("CapturedByClosure", "E0093"),
    ];
    let owned = expected.map(|(kind, code)| (kind.into(), code.into()));
    assert_eq!(
        nested_kind_code_pairs("handle.rs", "HandleErrorKind::", 2),
        owned
    );
    for (kind, code) in expected {
        assert_eq!(error_catalogue::resolve_query_for_test(code), Some(kind));
        assert_eq!(error_catalogue::resolve_query_for_test(kind), Some(kind));
    }
    let (detail, _, _) =
        error_catalogue::entry_body_for_test("UsedAfterConsumed").expect("E0092's entry");
    assert!(detail.contains("E0720"), "{detail}");
}

/// `(kind name, code)` for every arm of `ParseErrorKind::code()`.
fn parse_kind_code_pairs() -> Vec<(String, String)> {
    exact_code_arms(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/error/parse_error.rs"),
        "ParseErrorKind::",
        8,
    )
}

/// [`code_arms`], asserting the table has exactly `expected` arms — the
/// non-vacuity guard for a fixed-size code block (ADR 30.9.26j D8 shares it).
pub(super) fn exact_code_arms(path: &str, prefix: &str, expected: usize) -> Vec<(String, String)> {
    let pairs = code_arms(path, prefix);
    assert_eq!(
        pairs.len(),
        expected,
        "the {prefix} code() oracle parsed {} arm(s), not {expected} — the match shape changed",
        pairs.len()
    );
    pairs
}

/// The `Kind::Variant… => "E…"` arms of one `code()` table, one per line.
fn code_arms(path: &str, prefix: &str) -> Vec<(String, String)> {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read the code() oracle at {path}: {e}"));

    let mut pairs = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix(prefix) else {
            continue;
        };
        let Some((lhs, rhs)) = rest.split_once("=>") else {
            continue;
        };
        let kind: String = lhs
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let code: String = rhs
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric())
            .into();
        if code.len() == 5 && code.starts_with(['E', 'W']) {
            pairs.push((kind, code));
        }
    }
    pairs
}

// ─────────────────────────────────────────────────────────────────────────────
// Syntax codes (ADR 29.9.26f AC2)
// ─────────────────────────────────────────────────────────────────────────────

/// Every parse code and kind name resolves, to the same entry
/// (29.9.26f AC2).
#[test]
fn every_parse_code_and_kind_name_resolve_to_one_entry() {
    for (kind, code) in parse_kind_code_pairs() {
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
        error_catalogue::resolve_query_for_test("E0113"),
        error_catalogue::resolve_query_for_test("InvalidPattern")
    );
}

/// The listing has a *Syntax* group carrying all eight codes, in its own block
/// (29.9.26f AC2).
#[test]
fn the_listing_has_a_syntax_group_with_every_parse_code() {
    let listing = error_catalogue::render_error_list();
    let group = listing
        .split("\n\n")
        .find(|block| block.starts_with("Syntax:"))
        .unwrap_or_else(|| panic!("no Syntax group in the listing:\n{listing}"));
    for (kind, code) in parse_kind_code_pairs() {
        assert!(
            group.contains(&format!("{code}   {kind}")),
            "`{code} {kind}` missing from the Syntax group:\n{group}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Code lookup (ADR 8.8.26a)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_code_resolves_to_its_kind() {
    assert_eq!(
        error_catalogue::resolve_query_for_test("E0061"),
        Some("NonStrictlyPositive")
    );
    assert_eq!(
        error_catalogue::resolve_query_for_test("E0010"),
        Some("TypeMismatch")
    );
}

#[test]
fn a_code_resolves_case_insensitively() {
    // Users retype what they saw, and shells lowercase nothing for them.
    assert_eq!(
        error_catalogue::resolve_query_for_test("e0061"),
        Some("NonStrictlyPositive")
    );
    assert_eq!(
        error_catalogue::resolve_query_for_test("E0061"),
        error_catalogue::resolve_query_for_test("e0061")
    );
}

#[test]
fn the_kind_name_path_is_not_regressed() {
    assert_eq!(
        error_catalogue::resolve_query_for_test("NonStrictlyPositive"),
        Some("NonStrictlyPositive")
    );
    assert_eq!(
        error_catalogue::resolve_query_for_test("typemismatch"),
        Some("TypeMismatch")
    );
}

#[test]
fn a_query_that_is_neither_resolves_to_nothing() {
    assert_eq!(error_catalogue::resolve_query_for_test("Nonsense"), None);
    assert_eq!(error_catalogue::resolve_query_for_test(""), None);
}

#[test]
fn code_shaped_inputs_are_told_apart_from_names() {
    // The failure paths differ: a bad code gets no fuzzy name suggestion,
    // because "did you mean `TypeMismatch`?" is useless advice for `E0099`.
    for code in ["E0010", "e0010", "W0001", "E9999"] {
        assert!(
            error_catalogue::is_code_shaped_for_test(code),
            "`{code}` should read as a code"
        );
    }
    for name in ["TypeMismatch", "E001", "E00100", "EXXXX", "", "0E010"] {
        assert!(
            !error_catalogue::is_code_shaped_for_test(name),
            "`{name}` should NOT read as a code"
        );
    }
}

#[test]
fn an_unresolvable_code_still_fails() {
    // E0099 is not assigned. The command must fail, not fall back to a
    // near-miss name — that is the whole point of `is_code_shaped`.
    assert_eq!(error_catalogue::resolve_query_for_test("E0099"), None);
    assert!(error_catalogue::is_code_shaped_for_test("E0099"));
}

#[test]
fn the_listing_prints_every_code_beside_its_kind() {
    let listing = error_catalogue::render_error_list();
    // A reader arriving from `error[E0061]` scans for the code, not the name.
    for (kind, code) in kind_code_pairs() {
        if DELIBERATELY_UNEXPLAINED.iter().any(|(k, _)| *k == kind)
            || DELIBERATELY_UNLISTED.iter().any(|(k, _)| *k == kind)
        {
            continue;
        }
        assert!(
            listing.contains(&code),
            "code `{code}` (`{kind}`) is missing from the listing"
        );
        assert!(listing.contains(&kind), "kind `{kind}` is missing");
    }
    assert!(
        listing.contains("<code|name>"),
        "the footer must advertise that codes are accepted: {listing}"
    );
}

/// ADR 28.9.26d AC 7: E0071's explanation says what conversion now unfolds —
/// total definitions — and what it never does — `#[partial]` ones.
#[test]
fn invalid_refl_explanation_names_what_conversion_unfolds() {
    let explanation = crate::explain::explanations::get_explanation("InvalidRefl")
        .expect("InvalidRefl has an explanation");
    assert!(
        explanation.detail.contains("total"),
        "{}",
        explanation.detail
    );
    assert!(
        explanation.detail.contains("`#[partial]`"),
        "{}",
        explanation.detail
    );
}

/// ADR 29.9.26c AC 6: after D4, `n + 0` reduces, so the explanation must no
/// longer send it to induction — and must name `0 + n` as the case that is.
#[test]
fn invalid_refl_explanation_names_the_addition_rule() {
    let detail = crate::explain::explanations::get_explanation("InvalidRefl")
        .expect("InvalidRefl has an explanation")
        .detail;
    assert!(
        !detail.contains("`n + 0` and `n` are not"),
        "stale induction claim: {detail}"
    );
    assert!(detail.contains("`n + 0` reduces to `n`"), "{detail}");
    assert!(detail.contains("`0 + n` and"), "{detail}");
    assert!(detail.contains("induction (`natind`)"), "{detail}");
}
