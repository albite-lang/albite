//! ADR 3.10.26o AC 2: today's verdicts do not move. The rule runs only where
//! the existing annotation rule refuses, and on its own refusal the caller
//! gets the existing rule's error, unchanged (D2).

use super::*;

/// 3.10.26o AC2: a term the old rule accepts stays accepted even where the
/// new rule, asked directly, would refuse it.
#[test]
fn a_term_the_old_rule_accepts_and_the_new_would_refuse_is_still_accepted() {
    let zero = || Term::refl(Type::Nat, Term::Zero);
    let split = Term::case(Term::var("o"), "u", zero(), "b", zero());
    let goal = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let ctx = proof_over(&[("o", opt())]);
    assert!(type_of_dependent_case(&ctx, &split, &goal).is_err());
    assert_eq!(type_of(&ctx, &Term::annot(split, goal.clone())), Ok(goal));
}

/// ADR 4.10.26d AC4 (D4): the Core a constructor of another type lowered to
/// before that ADR — `match o { PickWrong(k) => …, _ => … }` over
/// `OptWrong ≡ Unit + Nat`, where the elaborator typed `k : Nat` — is refused
/// by the kernel, which types `Case`'s untyped binder from the scrutinee as
/// `Unit`. The hole was in elaboration and at run time, never in proof.
#[test]
fn a_constructor_of_another_type_is_refused_by_the_kernel() {
    let zero = || Term::refl(Type::Nat, Term::Zero);
    let nat_fact = || Type::eq(Type::Nat, Term::var("k"), Term::var("k"));
    let wrong_arm = Term::let_in(
        "_f",
        nat_fact(),
        Term::refl(Type::Nat, Term::var("k")),
        zero(),
    );
    let split = Term::case(Term::var("o"), "k", wrong_arm, "w", zero());
    let goal = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let theorem = Term::lambda("o", opt(), split);
    let statement = Type::arrow(opt(), goal);
    assert_eq!(
        crate::typecheck::check_proof(&Context::new(), &theorem, &statement),
        Err(TypeError::IllTypedEqWitness {
            eq: nat_fact(),
            cause: Box::new(TypeError::TypeMismatch {
                expected: Type::Nat,
                got: Type::Unit,
            }),
        })
    );
}

/// 3.10.26o AC2: an annotation that is no split keeps the old mismatch.
#[test]
fn a_non_matching_shape_returns_the_old_error_unchanged() {
    let ctx = proof_over(&[("n", Type::Nat)]);
    assert_eq!(
        type_of(&ctx, &Term::annot(Term::var("n"), Type::Bool)),
        Err(TypeError::TypeMismatch {
            expected: Type::Bool,
            got: Type::Nat,
        })
    );
}

/// 3.10.26o AC2: a split the new rule refuses reports exactly the error the
/// old rule raises — here the case's own branch mismatch.
#[test]
fn a_failed_split_returns_the_old_error_unchanged() {
    let wrong = Term::case(
        Term::var("o"),
        "u",
        Term::refl(opt(), some(Term::Zero)),
        "b",
        chain(
            "b",
            &[("x", Type::Nat)],
            Term::refl(opt(), some(Term::var("x"))),
        ),
    );
    let ctx = proof_over(&[("o", opt())]);
    let goal = rebuild_goal("o");
    let old_error = type_of(&ctx, &wrong).expect_err("the old rule refuses the split");
    assert!(type_of_dependent_case(&ctx, &wrong, &goal).is_err());
    assert_eq!(type_of(&ctx, &Term::annot(wrong, goal)), Err(old_error));
}
