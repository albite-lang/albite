use super::*;
use crate::eval::eval;
use crate::types::Type;

// ==========================================================================
// Phase 2A Tests: Strings
// ==========================================================================

#[test]
fn test_string_literal_is_value() {
    let term = Term::string_lit("hello");
    let result = step(&term);
    assert_eq!(result, StepResult::Value);
    assert!(term.is_value());
}

#[test]
fn test_str_concat() {
    let term = Term::str_concat(Term::string_lit("hello"), Term::string_lit(" world"));
    let result = eval(&term);
    assert_eq!(result, Term::string_lit("hello world"));
}

#[test]
fn test_str_concat_empty() {
    let term = Term::str_concat(Term::string_lit(""), Term::string_lit("test"));
    let result = eval(&term);
    assert_eq!(result, Term::string_lit("test"));
}

#[test]
fn test_str_len_empty() {
    let term = Term::str_len(Term::string_lit(""));
    let result = eval(&term);
    assert_eq!(result, Term::Zero);
}

#[test]
fn test_str_len_nonempty() {
    let term = Term::str_len(Term::string_lit("hello"));
    let result = eval(&term);
    assert_eq!(result, Term::nat(5));
}

#[test]
fn test_str_eq_true() {
    let term = Term::str_eq(Term::string_lit("hello"), Term::string_lit("hello"));
    let result = eval(&term);
    assert_eq!(result, Term::True);
}

#[test]
fn test_str_eq_false() {
    let term = Term::str_eq(Term::string_lit("hello"), Term::string_lit("world"));
    let result = eval(&term);
    assert_eq!(result, Term::False);
}

#[test]
fn test_str_concat_nested() {
    let term = Term::str_concat(
        Term::str_concat(Term::string_lit("a"), Term::string_lit("b")),
        Term::string_lit("c"),
    );
    let result = eval(&term);
    assert_eq!(result, Term::string_lit("abc"));
}

// ADR 13.10.26j: `naïve café` is ten characters and twelve bytes; `ï` is 2..4.
const NAIVE_CAFE: &str = "naïve café";

fn substring_closed(text: &str, start: u64, len: u64) -> StepResult {
    // `nat_lit`, not `nat`: `nat(u64::MAX)` would build a unary chain.
    step(&Term::str_substring(
        Term::string_lit(text),
        Term::nat_lit(start),
        Term::nat_lit(len),
    ))
}

// ADR 13.10.26j AC2: the closed stepper gives the environment stepper's bytes.
#[test]
fn substring_and_char_at_count_bytes_on_the_closed_stepper() {
    assert_eq!(
        substring_closed(NAIVE_CAFE, 0, 4),
        StepResult::Stepped(Term::string_lit("naï"))
    );
    assert_eq!(
        substring_closed(NAIVE_CAFE, 4, 3),
        StepResult::Stepped(Term::string_lit("ve "))
    );
    let char_at = Term::str_char_at(Term::string_lit(NAIVE_CAFE), Term::nat(2));
    assert_eq!(crate::eval::term_to_nat(&eval(&char_at)), Some(195));
}

// ADR 13.10.26j AC3: a cut inside a character is `Stuck`, not a panic.
#[test]
fn substring_cut_inside_a_character_is_stuck_on_the_closed_stepper() {
    assert_eq!(substring_closed(NAIVE_CAFE, 0, 3), StepResult::Stuck);
    assert_eq!(substring_closed(NAIVE_CAFE, 3, 2), StepResult::Stuck);
}

// ADR 13.10.26j AC3: `start + len` saturates instead of overflowing.
#[test]
fn substring_with_a_huge_length_saturates_on_the_closed_stepper() {
    assert_eq!(
        substring_closed("ab", 1, u64::MAX),
        StepResult::Stepped(Term::string_lit("b"))
    );
    assert_eq!(
        substring_closed("ab", 7, 1),
        StepResult::Stepped(Term::string_lit(""))
    );
}

fn char_at_on_both_steppers(text: &str, index: u64) -> (StepResult, StepResult) {
    let term = Term::str_char_at(Term::string_lit(text), Term::nat_lit(index));
    let env = crate::eval::EvalEnv::empty();
    (step(&term), crate::eval::step_with_env(&term, &env))
}

// ADR 16.10.26i AC2: past the end is `0` on the closed and the environment
// stepper alike, at the length, beyond it and at `u64::MAX`.
#[test]
fn char_at_past_the_end_is_zero_on_both_steppers() {
    let len = NAIVE_CAFE.len() as u64;
    for index in [len, len + 3, u64::MAX] {
        let (closed, with_env) = char_at_on_both_steppers(NAIVE_CAFE, index);
        assert_eq!(closed, StepResult::Stepped(Term::nat_smart(0)), "{index}");
        assert_eq!(with_env, closed, "{index}");
    }
}

// ADR 16.10.26i AC2: in range, both steppers give the byte.
#[test]
fn char_at_in_range_is_the_byte_on_both_steppers() {
    let (closed, with_env) = char_at_on_both_steppers(NAIVE_CAFE, 2);
    assert_eq!(closed, StepResult::Stepped(Term::nat_smart(195)));
    assert_eq!(with_env, closed);
}

// ==========================================================================
// Phase 2A Tests: Fix Combinator
// ==========================================================================

#[test]
fn test_fix_identity() {
    let fix_term = Term::fix(
        "f",
        Type::arrow(Type::Nat, Type::Nat),
        Term::lambda("n", Type::Nat, Term::var("n")),
    );
    let term = Term::app(fix_term, Term::nat(5));
    let result = eval(&term);
    assert_eq!(result, Term::nat(5));
}

#[test]
fn test_fix_double() {
    let fix_term = Term::fix(
        "f",
        Type::arrow(Type::Nat, Type::Nat),
        Term::lambda("n", Type::Nat, Term::succ(Term::var("n"))),
    );
    let term = Term::app(fix_term, Term::nat(3));
    let result = eval(&term);
    assert_eq!(result, Term::nat(4));
}

// ==========================================================================
// Phase 2A Tests: μ-types (Iso-recursive)
// ==========================================================================

#[test]
fn test_fold_is_value() {
    let mu_ty = Type::mu("α", Type::sum(Type::Unit, Type::TyVar("α".into())));
    let term = Term::fold(
        mu_ty.clone(),
        Term::inl(Type::sum(Type::Unit, mu_ty), Term::Unit),
    );
    assert!(term.is_value());
    let result = step(&term);
    assert_eq!(result, StepResult::Value);
}

#[test]
fn test_unfold_fold() {
    let mu_ty = Type::mu("α", Type::sum(Type::Unit, Type::TyVar("α".into())));
    let inner = Term::inl(Type::sum(Type::Unit, mu_ty.clone()), Term::Unit);
    let folded = Term::fold(mu_ty.clone(), inner.clone());
    let term = Term::unfold(mu_ty, folded);
    let result = eval(&term);
    assert_eq!(result, inner);
}

#[test]
fn test_mu_peano_zero() {
    let peano = Type::mu("α", Type::sum(Type::Unit, Type::TyVar("α".into())));
    let unfolded_ty = Type::sum(Type::Unit, peano.clone());
    let zero = Term::fold(peano.clone(), Term::inl(unfolded_ty.clone(), Term::Unit));

    let unfolded = Term::unfold(peano, zero);
    let result = eval(&unfolded);

    match result {
        Term::Inl(_, inner) => assert_eq!(*inner, Term::Unit),
        _ => panic!("Expected Inl"),
    }
}

#[test]
fn test_mu_peano_succ() {
    let peano = Type::mu("α", Type::sum(Type::Unit, Type::TyVar("α".into())));
    let unfolded_ty = Type::sum(Type::Unit, peano.clone());

    let zero = Term::fold(peano.clone(), Term::inl(unfolded_ty.clone(), Term::Unit));
    let one = Term::fold(peano.clone(), Term::inr(unfolded_ty.clone(), zero));

    let unfolded = Term::unfold(peano.clone(), one);
    let result = eval(&unfolded);

    let Term::Inr(_, inner) = result else {
        panic!("Expected Inr");
    };
    let Term::Fold(_, inner_inner) = inner.as_ref() else {
        panic!("Expected Fold inside Inr");
    };
    let Term::Inl(_, u) = inner_inner.as_ref() else {
        panic!("Expected Inl inside Fold");
    };
    assert_eq!(u.as_ref(), &Term::Unit);
}
