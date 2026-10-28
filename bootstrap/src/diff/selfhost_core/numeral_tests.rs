//! The numeral-normalised second tier (ADR 4.10.26b): pure, over injected
//! renderings, with no binary and no elaboration.

use super::numerals::normalise_numerals;
use super::render::{
    agree_up_to_numerals_text, compare, exit_code_for, CoreComparison, SelfhostTerm,
};

/// 3.10.26i's `double`, as the bootstrap rendered it.
const DOUBLE_BOOTSTRAP: &str =
    "(λn:Nat. (natind [k. (Nat → Nat)] zero (λj:Nat. (λacc:Nat. (acc + (succ (succ zero))))) n))";

/// The same definition, as the self-host rendered it.
const DOUBLE_SELFHOST: &str =
    "(λn:Nat. (natind [k. (Nat → Nat)] 0 (λj:Nat. (λacc:Nat. (acc + 2))) n))";

fn rendered(text: &str) -> SelfhostTerm {
    SelfhostTerm::Rendered(text.to_string())
}

// 4.10.26b AC1
#[test]
fn normalise_numerals_rewrites_succ_chains_and_zero() {
    assert_eq!(
        normalise_numerals("(acc + (succ (succ zero)))").as_deref(),
        Some("(acc + 2)")
    );
    assert_eq!(normalise_numerals("zero").as_deref(), Some("0"));
    assert_eq!(normalise_numerals("(f zero)").as_deref(), Some("(f 0)"));
    assert_eq!(
        normalise_numerals("(global:zero_mul x)").as_deref(),
        Some("(global:zero_mul x)"),
        "a name that merely starts with `zero` is not the constructor"
    );
    assert_eq!(normalise_numerals("zero_mul").as_deref(), Some("zero_mul"));
    assert_eq!(normalise_numerals("(succ zero)").as_deref(), Some("1"));
}

#[test]
fn a_successor_of_a_variable_is_not_a_numeral() {
    // `(succ n)` is not a chain; its inner `zero`-free text is left alone, and
    // only the inner complete chain of a mixed one is rewritten.
    assert_eq!(normalise_numerals("(succ n)").as_deref(), Some("(succ n)"));
    assert_eq!(
        normalise_numerals("(succ (f (succ zero)))").as_deref(),
        Some("(succ (f 1))")
    );
}

#[test]
fn an_unclosed_chain_is_not_a_numeral() {
    // Too few closers after `zero`: the outer links are not a chain, but the
    // inner `zero` is still a bare constructor.
    assert_eq!(
        normalise_numerals("(succ (succ zero)").as_deref(),
        Some("(succ 1")
    );
}

#[test]
fn a_zero_beside_a_bracket_or_comma_is_a_token() {
    assert_eq!(
        normalise_numerals("(zero, (succ zero))").as_deref(),
        Some("(0, 1)")
    );
    assert_eq!(
        normalise_numerals("[x => zero]").as_deref(),
        Some("[x => 0]")
    );
    assert_eq!(normalise_numerals("xzero").as_deref(), Some("xzero"));
}

#[test]
fn every_binder_spelling_of_zero_or_succ_declines() {
    for ambiguous in [
        "(λzero:Nat. zero)",
        "(λsucc:Nat. succ)",
        "(fix zero:Nat. zero)",
        "(let zero : Nat = 1 in zero)",
        "(let succ : Nat = 1 in succ)",
        "(natind [zero. Nat] 0 f n)",
        "(natind [succ. Nat] 0 f n)",
        "(case s of inl zero => zero | inr y => y)",
        "(case s of inl x => x | inr succ => succ)",
        "(adt_match s [0 zero => zero])",
    ] {
        assert_eq!(
            normalise_numerals(ambiguous),
            None,
            "{ambiguous} should decline"
        );
    }
}

// 4.10.26b AC2
#[test]
fn numeral_spelling_alone_agrees_up_to_numerals() {
    let comparison = compare(DOUBLE_BOOTSTRAP, &rendered(DOUBLE_SELFHOST));
    assert_eq!(
        comparison,
        CoreComparison::AgreeUpToNumerals {
            bootstrap: DOUBLE_BOOTSTRAP.to_string(),
            selfhost: DOUBLE_SELFHOST.to_string(),
            normalised: DOUBLE_SELFHOST.to_string(),
        }
    );
    assert_eq!(exit_code_for(&comparison), 0);
}

// 4.10.26b AC3
#[test]
fn structural_difference_still_diverges() {
    let other_binder = DOUBLE_SELFHOST.replace("[k. ", "[_. ");
    let comparison = compare(DOUBLE_BOOTSTRAP, &rendered(&other_binder));
    assert!(
        matches!(comparison, CoreComparison::Diverge { .. }),
        "{comparison:?}"
    );
    assert_eq!(exit_code_for(&comparison), 1);
}

// 4.10.26b AC4
#[test]
fn byte_equal_terms_agree_without_normalising() {
    assert_eq!(
        compare(DOUBLE_BOOTSTRAP, &rendered(DOUBLE_BOOTSTRAP)),
        CoreComparison::Agree(DOUBLE_BOOTSTRAP.to_string())
    );
}

// 4.10.26b AC4b (adr-check numbers it positionally: 4.10.26b AC5)
#[test]
fn a_bound_zero_still_diverges() {
    let comparison = compare("(λzero:Nat. zero)", &rendered("(λzero:Nat. 0)"));
    assert!(
        matches!(comparison, CoreComparison::Diverge { .. }),
        "{comparison:?}"
    );
}

// 4.10.26b AC4b (positionally 4.10.26b AC5)
#[test]
fn string_literals_are_never_normalised() {
    assert_eq!(normalise_numerals("(f \"a zero b\")"), None);
    let comparison = compare("(f \"a zero b\")", &rendered("(f \"a 0 b\")"));
    assert!(
        matches!(comparison, CoreComparison::Diverge { .. }),
        "{comparison:?}"
    );
}

#[test]
fn one_side_declining_is_enough_to_diverge() {
    // The bootstrap side normalises cleanly; the self-host's binds `zero`.
    let comparison = compare("(f zero)", &rendered("(λzero:Nat. (f 0))"));
    assert!(matches!(comparison, CoreComparison::Diverge { .. }));
    let reversed = compare("(λzero:Nat. (f 0))", &rendered("(f zero)"));
    assert!(matches!(reversed, CoreComparison::Diverge { .. }));
}

// 4.10.26b AC4c (positionally 4.10.26b AC6)
#[test]
fn agree_up_to_numerals_report_prints_all_three_terms() {
    let text = agree_up_to_numerals_text("(succ zero)", "1 ", "1");
    assert!(text.contains("up to how numerals are spelled"), "{text}");
    assert!(text.contains("bootstrap:  (succ zero)\n"), "{text}");
    assert!(text.contains("self-host:  1 \n"), "{text}");
    assert!(text.contains("normalised: 1\n"), "{text}");
}
