//! `substring` and `char_at` on the environment stepper count bytes
//! (ADR 13.10.26j D1–D3), and a cut inside a character traps (ADR 16.10.26h).
//!
//! The environment stepper is what `tungsten run`, `tungsten test`, the
//! evaluator side of `diff exec` and the kernel's closed-primitive conversion
//! use, so it must give the answer native code and the closed stepper give.
//! `naïve café` is ten characters and twelve bytes: `ï` is bytes 2..4.
//!
//! Tests: albite_core/src/eval/env/handlers_string.rs

use crate::eval::env::*;
use crate::eval::term_to_nat;

const NAIVE_CAFE: &str = "naïve café";

fn substring_env(text: &str, start: u64, len: u64) -> StepResult {
    // `nat_lit`, not `nat`: `nat(u64::MAX)` would build a unary chain.
    let term = Term::str_substring(
        Term::string_lit(text),
        Term::nat_lit(start),
        Term::nat_lit(len),
    );
    step_with_env(&term, &EvalEnv::empty())
}

fn char_at_env(text: &str, idx: u64) -> Option<usize> {
    let term = Term::str_char_at(Term::string_lit(text), Term::nat_lit(idx));
    let StepResult::Stepped(result) = step_with_env(&term, &EvalEnv::empty()) else {
        panic!("char_at on a literal and a Nat must step");
    };
    term_to_nat(&result)
}

// ADR 13.10.26j AC2: offsets and lengths are bytes, as native code counts them.
#[test]
fn substring_counts_bytes_on_the_environment_stepper() {
    assert_eq!(
        substring_env(NAIVE_CAFE, 0, 4),
        StepResult::Stepped(Term::string_lit("naï"))
    );
    assert_eq!(
        substring_env(NAIVE_CAFE, 4, 3),
        StepResult::Stepped(Term::string_lit("ve "))
    );
}

// ADR 13.10.26j AC2: `char_at` is the byte at an index, not the code point.
#[test]
fn char_at_is_the_byte_on_the_environment_stepper() {
    assert_eq!(char_at_env(NAIVE_CAFE, 2), Some(195));
    assert_eq!(char_at_env(NAIVE_CAFE, 3), Some(175));
    assert_eq!(char_at_env(NAIVE_CAFE, 0), Some(usize::from(b'n')));
}

// ADR 13.10.26j: out of range stays `0`, as `tg_string_char_at_internal` gives.
#[test]
fn char_at_out_of_range_is_zero_on_the_environment_stepper() {
    assert_eq!(char_at_env("ab", 5), Some(0));
    assert_eq!(char_at_env("ab", 2), Some(0));
}

/// The step result and the stop it recorded, for a cut on a fresh env.
fn substring_env_stop(text: &str, start: u64, len: u64) -> (StepResult, Option<EvalStopped>) {
    let term = Term::str_substring(
        Term::string_lit(text),
        Term::nat_lit(start),
        Term::nat_lit(len),
    );
    let env = EvalEnv::empty();
    let result = step_with_env(&term, &env);
    (result, env.recorded_stop())
}

// ADR 16.10.26h AC2: a cut inside `ï` — the end, the start, or both — is the
// substring trap on the environment stepper, not a silent `Stuck`.
#[test]
fn substring_cut_inside_a_character_records_the_trap_on_the_environment_stepper() {
    let trap = Some(EvalStopped::IntTrap {
        kind: IntTrapKind::SubstringInsideCharacter,
    });
    for (start, len) in [(0, 3), (3, 2), (3, 8)] {
        assert_eq!(
            substring_env_stop(NAIVE_CAFE, start, len),
            (StepResult::Stuck, trap.clone()),
            "({start}, {len})"
        );
    }
}

// ADR 16.10.26h AC2: a boundary cut and a clamped cut record no stop.
#[test]
fn substring_on_a_boundary_or_clamped_records_no_stop() {
    assert_eq!(
        substring_env_stop(NAIVE_CAFE, 0, 4),
        (StepResult::Stepped(Term::string_lit("naï")), None)
    );
    assert_eq!(
        substring_env_stop("ab", 1, u64::MAX),
        (StepResult::Stepped(Term::string_lit("b")), None)
    );
}

// ADR 13.10.26j AC3: a length past the end saturates rather than overflowing.
#[test]
fn substring_with_a_huge_length_saturates_on_the_environment_stepper() {
    assert_eq!(
        substring_env("ab", 1, u64::MAX),
        StepResult::Stepped(Term::string_lit("b"))
    );
    assert_eq!(
        substring_env("ab", 7, 1),
        StepResult::Stepped(Term::string_lit(""))
    );
    assert_eq!(
        substring_env("ab", 1, 0),
        StepResult::Stepped(Term::string_lit(""))
    );
}
