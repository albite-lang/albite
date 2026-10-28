use super::*;
use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

// ==========================================================================
// Phase 3-Prep Tests: Integer Comparison
// ==========================================================================

#[test]
fn test_nat_lt() {
    let ctx = Context::new();
    let term = Term::nat_lt(Term::Zero, Term::succ(Term::Zero));
    assert_eq!(type_of(&ctx, &term), Ok(Type::Bool));
}

#[test]
fn test_nat_le() {
    let ctx = Context::new();
    let term = Term::nat_le(Term::Zero, Term::Zero);
    assert_eq!(type_of(&ctx, &term), Ok(Type::Bool));
}

#[test]
fn test_nat_gt() {
    let ctx = Context::new();
    let term = Term::nat_gt(Term::succ(Term::Zero), Term::Zero);
    assert_eq!(type_of(&ctx, &term), Ok(Type::Bool));
}

#[test]
fn test_nat_ge() {
    let ctx = Context::new();
    let term = Term::nat_ge(Term::Zero, Term::Zero);
    assert_eq!(type_of(&ctx, &term), Ok(Type::Bool));
}

#[test]
fn test_nat_comparison_type_mismatch() {
    let ctx = Context::new();
    let term = Term::nat_lt(Term::True, Term::Zero);
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

// ==========================================================================
// Phase 3-Prep Tests: String char_at
// ==========================================================================

#[test]
fn test_str_char_at() {
    let ctx = Context::new();
    let term = Term::str_char_at(Term::string_lit("hello"), Term::Zero);
    assert_eq!(type_of(&ctx, &term), Ok(Type::Nat));
}

#[test]
fn test_str_char_at_type_mismatch_string() {
    let ctx = Context::new();
    let term = Term::str_char_at(Term::Zero, Term::Zero); // Nat instead of String
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

#[test]
fn test_str_char_at_type_mismatch_index() {
    let ctx = Context::new();
    let term = Term::str_char_at(Term::string_lit("hi"), Term::True); // Bool instead of Nat
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

// ==========================================================================
// Phase 3-Prep Tests: Ref Cells
// ==========================================================================

#[test]
fn test_ref_new() {
    let ctx = Context::new();
    let term = Term::ref_new(Term::NatLit(42));
    assert_eq!(type_of(&ctx, &term), Ok(Type::ref_ty(Type::Nat)));
}

#[test]
fn test_ref_get() {
    let ctx = Context::new().with_term("r", Type::ref_ty(Type::Nat));
    let term = Term::ref_get(Term::var("r"));
    assert_eq!(type_of(&ctx, &term), Ok(Type::Nat));
}

#[test]
fn test_ref_get_type_mismatch() {
    let ctx = Context::new().with_term("x", Type::Nat);
    let term = Term::ref_get(Term::var("x")); // Nat is not Ref<_>
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

#[test]
fn test_ref_set() {
    let ctx = Context::new().with_term("r", Type::ref_ty(Type::Nat));
    let term = Term::ref_set(Term::var("r"), Term::nat(10));
    assert_eq!(type_of(&ctx, &term), Ok(Type::Unit));
}

#[test]
fn test_ref_set_type_mismatch() {
    let ctx = Context::new().with_term("r", Type::ref_ty(Type::Nat));
    let term = Term::ref_set(Term::var("r"), Term::True); // Bool instead of Nat
    assert!(matches!(
        type_of(&ctx, &term),
        Err(TypeError::TypeMismatch { .. })
    ));
}

// ==========================================================================
// Phase 3-Prep Tests: Ptr and Ref Types
// ==========================================================================

#[test]
fn test_ptr_type_well_formed() {
    let ty = Type::ptr(Type::Nat);
    let type_vars = std::collections::HashSet::new();
    assert!(ty.is_well_formed(&type_vars));
}

#[test]
fn test_ref_type_well_formed() {
    let ty = Type::ref_ty(Type::String);
    let type_vars = std::collections::HashSet::new();
    assert!(ty.is_well_formed(&type_vars));
}

#[test]
fn test_ptr_type_display() {
    let ty = Type::ptr(Type::Nat);
    assert_eq!(ty.to_string(), "Ptr<Nat>");
}

#[test]
fn test_ref_type_display() {
    let ty = Type::ref_ty(Type::Bool);
    assert_eq!(ty.to_string(), "Ref<Bool>");
}

// ==========================================================================
// Primitive operand checks, both directions (ADR 29.9.26e D11 moved every
// one of these comparisons onto `types_equal_in`)
// ==========================================================================

fn boxed(term: Term) -> Box<Term> {
    Box::new(term)
}

fn text() -> Term {
    Term::StringLit("s".into())
}

/// `build(good…)` types at `expected`; replacing any one operand by `True`
/// is refused.
fn assert_operands_checked(
    arity: usize,
    build: impl Fn(Vec<Term>) -> Term,
    good: &[Term],
    expected: Type,
) {
    let ctx = Context::new();
    assert_eq!(
        type_of(&ctx, &build(good.to_vec())),
        Ok(expected),
        "all operands good"
    );
    for bad in 0..arity {
        let mut operands = good.to_vec();
        operands[bad] = Term::True;
        assert!(
            type_of(&ctx, &build(operands)).is_err(),
            "operand {bad} is Bool"
        );
    }
}

/// The helper must notice a rule that ignores its operand.
#[test]
#[should_panic(expected = "operand 0 is Bool")]
fn assert_operands_checked_fails_on_an_unchecked_operand() {
    assert_operands_checked(1, |_| Term::Zero, &[Term::Zero], Type::Nat);
}

#[test]
fn type_of_nat_binop_and_bool_ops_check_each_operand() {
    let nat_add = |o: Vec<Term>| Term::NatAdd(boxed(o[0].clone()), boxed(o[1].clone()));
    assert_operands_checked(2, nat_add, &[Term::Zero, Term::Zero], Type::Nat);
    let ctx = Context::new();
    let and = |l: Term, r: Term| Term::BoolAnd(boxed(l), boxed(r));
    assert_eq!(type_of(&ctx, &and(Term::True, Term::False)), Ok(Type::Bool));
    assert!(type_of(&ctx, &and(Term::Zero, Term::False)).is_err());
    assert!(type_of(&ctx, &and(Term::True, Term::Zero)).is_err());
    assert_eq!(
        type_of(&ctx, &Term::BoolNot(boxed(Term::True))),
        Ok(Type::Bool)
    );
    assert!(type_of(&ctx, &Term::BoolNot(boxed(Term::Zero))).is_err());
}

#[test]
fn type_of_string_ops_check_each_operand() {
    let concat = |o: Vec<Term>| Term::StrConcat(boxed(o[0].clone()), boxed(o[1].clone()));
    assert_operands_checked(2, concat, &[text(), text()], Type::String);
    let length = |o: Vec<Term>| Term::StrLen(boxed(o[0].clone()));
    assert_operands_checked(1, length, &[text()], Type::Nat);
    let equal = |o: Vec<Term>| Term::StrEq(boxed(o[0].clone()), boxed(o[1].clone()));
    assert_operands_checked(2, equal, &[text(), text()], Type::Bool);
    let substring = |o: Vec<Term>| {
        Term::StrSubstring(
            boxed(o[0].clone()),
            boxed(o[1].clone()),
            boxed(o[2].clone()),
        )
    };
    assert_operands_checked(
        3,
        substring,
        &[text(), Term::Zero, Term::Zero],
        Type::String,
    );
}

#[test]
fn type_of_adt_match_requires_every_arm_to_agree() {
    let ctx = Context::new();
    let pick = Type::Adt(
        "Pick".into(),
        vec![],
        vec![("PA".into(), Type::Unit), ("PB".into(), Type::Nat)],
    );
    let matching = |right_arm: Term| {
        Term::adt_match(
            Term::adt_construct(pick.clone(), 1, Term::Zero),
            vec![
                (0, "u".into(), boxed(Term::Zero)),
                (1, "n".into(), boxed(right_arm)),
            ],
        )
    };
    assert_eq!(type_of(&ctx, &matching(Term::var("n"))), Ok(Type::Nat));
    assert!(matches!(
        type_of(&ctx, &matching(Term::True)),
        Err(TypeError::BranchTypeMismatch { .. })
    ));
}

#[test]
fn type_of_subst_requires_the_equality_over_the_stated_type() {
    let ctx = Context::new().with_term("p", Type::Unit);
    let motive = Type::arrow(Type::Nat, Type::Unit);
    let over_bool = Term::subst(
        Type::Nat,
        motive.clone(),
        Term::refl(Type::Bool, Term::True),
        Term::var("p"),
    );
    assert!(matches!(
        type_of(&ctx, &over_bool),
        Err(TypeError::NotEquality { .. })
    ));
    let over_nat = Term::subst(
        Type::Nat,
        motive,
        Term::refl(Type::Nat, Term::Zero),
        Term::var("p"),
    );
    assert_eq!(type_of(&ctx, &over_nat), Ok(Type::Unit));
}
