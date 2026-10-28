//! ADR 1.10.26c AC 4: the terms inside an `Eq` are typed in proof mode, under
//! every binder in scope.

use super::*;

/// `Eq Nat true 0`
fn ill_typed() -> Type {
    eq_nat(Term::True, Term::Zero)
}

/// `λh: ty. h`
fn identity_at(ty: Type) -> Term {
    Term::lambda("h", ty, var("h"))
}

fn bool_for_nat() -> Box<TypeError> {
    Box::new(TypeError::TypeMismatch {
        expected: Type::Nat,
        got: Type::Bool,
    })
}

// 1.10.26c AC4
#[test]
fn an_ill_typed_witness_is_refused_in_proof_mode() {
    assert_eq!(
        type_of_proof(&Context::new(), &identity_at(ill_typed())),
        Err(TypeError::IllTypedEqWitness {
            eq: ill_typed(),
            cause: bool_for_nat(),
        })
    );
}

// 1.10.26c AC4: a `Pi`'s binder, and a `∀`'s type variable, are in scope.
#[test]
fn a_witness_is_typed_under_its_pi_binder() {
    let proof_ctx = Context::new().for_proof();
    let statement = Type::pi("n", Type::Nat, eq_nat(plus(var("n"), Term::Zero), var("n")));
    assert_eq!(check_type_wf(&proof_ctx, &statement), Ok(()));
    let polymorphic = Type::forall(
        "a",
        Type::pi(
            "x",
            Type::TyVar("a".into()),
            Type::eq(Type::TyVar("a".into()), var("x"), var("x")),
        ),
    );
    assert_eq!(check_type_wf(&proof_ctx, &polymorphic), Ok(()));
    // Outside its binder, the name is unbound.
    let escaped = Type::arrow(Type::Nat, eq_nat(var("n"), var("n")));
    assert_eq!(
        check_type_wf(&proof_ctx, &escaped),
        Err(TypeError::IllTypedEqWitness {
            eq: eq_nat(var("n"), var("n")),
            cause: Box::new(TypeError::UnboundVariable("n".into())),
        })
    );
}

// 1.10.26c AC4: the motive names `k`, which the context does not bind.
#[test]
fn a_natind_motive_types_its_witnesses_under_its_binder() {
    let verdict = type_of_proof(&over_n(), &right_zero_induction());
    assert_eq!(verdict, Ok(eq_nat(plus(var("n"), Term::Zero), var("n"))));
}

// 1.10.26c AC4: a proposition may not depend on a `#[partial]` global.
#[test]
fn a_witness_naming_an_opaque_global_is_refused() {
    let ctx = Context::new().with_global("loop", Type::Nat, GlobalAdmission::Opaque);
    let statement = eq_nat(Term::Global("loop".into()), Term::Zero);
    assert_eq!(
        type_of_proof(&ctx, &identity_at(statement.clone())),
        Err(TypeError::IllTypedEqWitness {
            eq: statement,
            cause: Box::new(TypeError::GlobalNotAdmitted {
                name: "loop".into()
            }),
        })
    );
}

// 1.10.26c AC4: the statement is screened too, not only the term.
#[test]
fn check_proof_checks_the_expected_type() {
    let proof = refl_nat(Term::Zero);
    assert_eq!(
        check_proof(&Context::new(), &proof, &ill_typed()),
        Err(TypeError::IllTypedEqWitness {
            eq: ill_typed(),
            cause: bool_for_nat(),
        })
    );
    let holed = eq_nat(Term::Annot(Box::new(Term::Sorry), Type::Nat), Term::Zero);
    assert_eq!(
        check_proof(&Context::new(), &proof, &holed),
        Err(TypeError::SorryInProof)
    );
}

// 1.10.26c AC4: the term's refusal comes first, so the census's first causes
// do not move.
#[test]
fn check_proof_keeps_the_terms_refusal_first() {
    assert_eq!(
        check_proof(&Context::new(), &Term::Sorry, &ill_typed()),
        Err(TypeError::SorryInProof)
    );
}

// 1.10.26c AC4: executable mode is unchanged (and 1.10.26c AC5: the witness
// walk adds nothing to the judgment the existing suites run).
#[test]
fn executable_mode_still_accepts_an_ill_typed_witness() {
    assert_eq!(
        type_of(&Context::new(), &identity_at(ill_typed())),
        Ok(Type::arrow(ill_typed(), ill_typed()))
    );
}

// 1.10.26c AC4
#[test]
fn each_witness_refusal_renders_its_cause() {
    let rendered = TypeError::IllTypedEqWitness {
        eq: ill_typed(),
        cause: bool_for_nat(),
    }
    .to_string();
    assert!(rendered.contains(&ill_typed().to_string()), "{rendered}");
    assert!(
        rendered.contains("Type mismatch: expected Nat, got Bool"),
        "{rendered}"
    );
}
