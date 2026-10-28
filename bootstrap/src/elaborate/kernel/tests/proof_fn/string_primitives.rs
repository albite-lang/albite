//! ADR 13.10.26j AC 4 — the kernel proves what the binary computes for
//! `substring` and `char_at`: bytes, as native code counts them.
//!
//! `naïve café` is ten characters and twelve bytes; `ï` is bytes 2..4, so
//! `(0, 4)` is `naï` in bytes and `naïv` in characters, and byte 2 is `195`
//! where the code point is `239`. Before the ADR the kernel admitted the
//! character answers and refused the byte ones.
//!
//! The refusals are E0071, raised by the elaborator's own conversion before
//! any obligation reaches the kernel: both steppers now give the byte answer,
//! so `refl` over the character answer never type-checks at all.

use super::project_error;

/// `statement` as a one-line `refl` theorem named `t`, and its first error.
fn refl_theorem(statement: &str) -> Option<String> {
    project_error(&format!("theorem t : {statement} = refl\n"))
}

/// The refusal every non-byte statement gets: `refl`'s sides differ.
fn assert_refl_refused(statement: &str) {
    let error = refl_theorem(statement).expect("the statement is refused");
    assert!(
        error.contains("error[E0071]: `refl` requires both sides to be equal"),
        "{error}"
    );
}

/// The helper's own contract: an admitted statement is not a refusal. Without
/// this, a helper that asserted nothing would let every refusal test pass.
#[test]
#[should_panic(expected = "the statement is refused")]
fn assert_refl_refused_fails_on_an_admitted_statement() {
    assert_refl_refused(r#"char_at("naïve", 2) == 195"#);
}

// ADR 13.10.26j AC4 — the byte answers are admitted.
#[test]
fn the_kernel_admits_the_byte_substring() {
    assert_eq!(
        refl_theorem(r#"substring("naïve café", 0, 4) == "naï""#),
        None
    );
}

#[test]
fn the_kernel_admits_the_byte_char_at() {
    assert_eq!(refl_theorem(r#"char_at("naïve", 2) == 195"#), None);
}

// ADR 13.10.26j AC4 — the character answers are refused, with E0071.
#[test]
fn the_kernel_refuses_the_character_substring() {
    assert_refl_refused(r#"substring("naïve café", 0, 4) == "naïv""#);
}

#[test]
fn the_kernel_refuses_the_character_char_at() {
    assert_refl_refused(r#"char_at("naïve", 2) == 239"#);
}

/// ADR 13.10.26j AC4 — §1.3: a cut inside `ï` panicked the closed stepper inside `check`. It is
/// now `Stuck`, so the `refl` is refused with a diagnostic.
#[test]
fn a_cut_inside_a_character_is_refused_not_a_panic() {
    assert_refl_refused(r#"substring("naïve café", 0, 3) == "na""#);
}

/// ADR 16.10.26h AC 6 — the proof checker computes nothing for a cut inside a
/// character: no value is admitted for any of the three cuts, neither side's
/// snap (`na` down, `naï` up) included, and each is E0071, never a panic.
#[test]
fn no_value_is_admitted_for_a_cut_inside_a_character() {
    for statement in [
        r#"substring("naïve café", 0, 3) == "naï""#,
        r#"substring("naïve café", 3, 2) == "v""#,
        r#"substring("naïve café", 3, 2) == "ve""#,
        r#"substring("naïve café", 3, 8) == "ve caf""#,
    ] {
        assert_refl_refused(statement);
    }
}

/// ADR 16.10.26h D4 — a statement that needs no value for the cut stays
/// provable: it is true of a program that aborts, as for an overflowing `Int`.
#[test]
fn a_cut_inside_a_character_equals_itself() {
    assert_eq!(
        refl_theorem(r#"substring("naïve café", 0, 3) == substring("naïve café", 0, 3)"#),
        None
    );
}

/// ADR 16.10.26i AC 5 — a pin: `char_at` past the end is `0`, as the kernel
/// already proved and the binary now computes, and no other value is admitted.
#[test]
fn the_kernel_admits_zero_past_the_end_of_char_at() {
    assert_eq!(refl_theorem(r#"char_at("ab", 5) == 0"#), None);
    assert_refl_refused(r#"char_at("ab", 5) == 1"#);
}
