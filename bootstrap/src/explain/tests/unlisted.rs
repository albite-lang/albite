//! Kinds `explain error` resolves but the no-argument listing withholds
//! (ADR 15.8.26b), split from `catalogue.rs` when a second, different kind of
//! withheld entry joined the first (ADR 3.10.26c D3).

use super::catalogue::kind_code_pairs;
use crate::explain::error_catalogue;

/// Kinds that resolve in `explain error <code|name>` but are deliberately
/// absent from the no-argument listing, each with the reason (ADR 15.8.26b).
///
/// Distinct from `DELIBERATELY_UNEXPLAINED`: these HAVE explanations — a
/// user holding the code can look it up — the listing just does not advertise
/// them among codes a user might encounter.
pub(super) const DELIBERATELY_UNLISTED: &[(&str, &str)] = &[
    (
        "InternalError",
        "the compiler's own broken invariant; not user-actionable, so the listing \
         does not advertise it",
    ),
    (
        "ForeignInProof",
        "shares E0063 and its page with PartialInProof (ADR 3.10.26c D3), so a \
         second listing row would print the same code twice",
    ),
];

/// Unlisted kinds whose code a listed kind also carries, so the code IS in
/// the listing — once, under the listed kind (ADR 3.10.26c D3).
const SHARES_A_LISTED_CODE: &[&str] = &["ForeignInProof"];

#[test]
fn unlisted_kinds_resolve_but_are_not_listed() {
    let listing = error_catalogue::render_error_list();
    let pairs = kind_code_pairs();
    for (kind, reason) in DELIBERATELY_UNLISTED {
        assert!(
            error_catalogue::resolve_query_for_test(kind).is_some(),
            "`{kind}` is unlisted ({reason}) but must still resolve by name"
        );
        let code = &pairs
            .iter()
            .find(|(k, _)| k == kind)
            .unwrap_or_else(|| panic!("`{kind}` is not a live ElabErrorKind variant"))
            .1;
        assert!(
            error_catalogue::resolve_query_for_test(code).is_some(),
            "`{kind}`'s code `{code}` must still resolve"
        );
        let expected_rows = usize::from(SHARES_A_LISTED_CODE.contains(kind));
        assert_eq!(
            listing.matches(code.as_str()).count(),
            expected_rows,
            "`{kind}`'s code `{code}` must appear in the no-argument listing \
             {expected_rows} time(s): only under a listed kind that shares it"
        );
        assert!(
            !listing.contains(kind),
            "`{kind}` itself must NOT appear in the no-argument listing"
        );
    }
}

/// 3.10.26c AC7: `explain error E0063` and `explain error ForeignInProof`
/// print the same page, it covers the foreign call, and the listing shows
/// E0063 once.
#[test]
fn foreign_in_proof_shares_the_e0063_page() {
    assert_eq!(
        error_catalogue::resolve_query_for_test("E0063"),
        Some("PartialInProof")
    );
    assert_eq!(
        error_catalogue::resolve_query_for_test("ForeignInProof"),
        Some("ForeignInProof")
    );
    let by_code = error_catalogue::entry_body_for_test("PartialInProof");
    let by_name = error_catalogue::entry_body_for_test("ForeignInProof");
    assert!(by_code.is_some(), "E0063 must have a page");
    assert_eq!(by_code, by_name, "the two queries must print one page");

    let (detail, example, code) = by_name.unwrap_or_default();
    assert_eq!(code, "E0063");
    assert!(detail.contains("foreign code"), "detail: {detail}");
    assert!(
        example.contains("extern \"C\" fn tg_type_nat"),
        "example: {example}"
    );

    let listing = error_catalogue::render_error_list();
    assert_eq!(listing.matches("E0063").count(), 1, "{listing}");
}
