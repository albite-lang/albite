//! The E0071 explanation states the open-term rules conversion applies.

fn invalid_refl_detail() -> &'static str {
    crate::explain::explanations::get_explanation("InvalidRefl")
        .expect("InvalidRefl has an explanation")
        .detail
}

/// ADR 30.9.26e AC 7: after D2, `n * 0` reduces, and the explanation names
/// `0 * n` as the case that still needs induction.
#[test]
fn invalid_refl_explanation_names_the_multiplication_rule() {
    let detail = invalid_refl_detail();
    assert!(detail.contains("`n * 0` reduces to `0`"), "{detail}");
    assert!(detail.contains("`n * (m + 1)` to `n * m + n`"), "{detail}");
    assert!(
        detail.contains("`0 * n` and `0` need induction"),
        "{detail}"
    );
}
