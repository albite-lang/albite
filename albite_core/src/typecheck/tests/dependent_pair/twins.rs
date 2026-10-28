//! ADR 4.10.26h AC 1's must-fail twins, each asserting its own refusal.

use super::*;
use crate::typecheck::rules_ext::type_of_dependent_case;

/// 4.10.26h AC1: a witness that does not satisfy the body is refused, and the
/// error names the instance `B[x := 1]`.
#[test]
fn a_wrong_witness_is_refused() {
    let one = Term::succ(Term::Zero);
    let verdict = type_of(&over_n(), &witnessed_by(one.clone()));
    let instance = eq_nat(Term::nat_add(Term::var("n"), one), Term::var("n"));
    assert_eq!(
        verdict,
        Err(TypeError::TypeMismatch {
            expected: instance,
            got: eq_nat(Term::var("n"), Term::var("n")),
        })
    );
}

fn binder_refusal(binder_ty: Type) -> TypeResult<()> {
    let sigma = Type::sigma("h", binder_ty, eq_nat(Term::Zero, Term::Zero));
    check_type_wf(&Context::new(), &sigma)
}

/// 4.10.26h AC1 (D3): a binder ranging over a proposition is refused.
#[test]
fn a_binder_of_a_proposition_type_is_refused() {
    let proposition = eq_nat(Term::Zero, Term::Zero);
    assert_eq!(
        binder_refusal(proposition.clone()),
        Err(TypeError::SigmaBinderNotData { got: proposition })
    );
    // Nested anywhere in a type, too.
    let nested = Type::arrow(
        Type::Nat,
        Type::sigma("h", Type::Void, eq_nat(Term::Zero, Term::Zero)),
    );
    assert_eq!(
        check_type_wf(&Context::new(), &nested),
        Err(TypeError::SigmaBinderNotData { got: Type::Void })
    );
    // Data binders pass.
    assert_eq!(binder_refusal(Type::Nat), Ok(()));
}

/// 4.10.26i AC1 (D1, re-pointing 4.10.26h's `a_binder_of_prop_is_refused`):
/// a binder over a conjunction is refused. `is_proposition` says no to it and
/// `sort_of` says `Prop`, so this is what the old stand-in could not see.
#[test]
fn a_binder_that_sorts_as_a_proposition_is_refused() {
    let conjunction = Type::product(eq_nat(Term::Zero, Term::Zero), Type::Void);
    assert!(!conjunction.is_proposition());
    assert_eq!(
        binder_refusal(conjunction.clone()),
        Err(TypeError::SigmaBinderNotData { got: conjunction })
    );
}

/// 4.10.26h AC1 (D1): a bare pair synthesises a `Product`, which no `Sigma`
/// accepts — only the annotation makes a dependent pair.
#[test]
fn an_unannotated_pair_is_a_product_not_a_sigma() {
    let bare = Term::pair(Term::Zero, Term::refl(Type::Nat, Term::Zero));
    let product = Type::product(Type::Nat, eq_nat(Term::Zero, Term::Zero));
    assert_eq!(type_of(&Context::new(), &bare), Ok(product.clone()));
    let sigma = Type::sigma("k", Type::Nat, eq_nat(Term::Zero, Term::Zero));
    let behind_a_product = Term::annot(Term::annot(bare, product.clone()), sigma.clone());
    assert_eq!(
        type_of(&Context::new(), &behind_a_product),
        Err(TypeError::TypeMismatch {
            expected: sigma,
            got: product,
        })
    );
}

/// 4.10.26h AC1 (D1): a `Sigma` whose binder is unused is not a `Product`.
#[test]
fn a_non_dependent_sigma_is_not_a_product() {
    let body = eq_nat(Term::Zero, Term::Zero);
    let sigma = Type::sigma("k", Type::Nat, body.clone());
    let product = Type::product(Type::Nat, body);
    let proof_ctx = Context::new().for_proof();
    assert!(!types_equal(&sigma, &product));
    assert!(check(
        &proof_ctx,
        &Term::annot(Term::var("p"), sigma.clone()),
        &product
    )
    .is_err());
    assert!(matches!(sigma, Type::Sigma(..)));
}

/// 4.10.26h AC1 (D4): there is no η — `(fst p, snd p)` does not convert to a
/// neutral `p`.
#[test]
fn pairing_is_not_eta() {
    let p = || Term::var("p");
    let eta = Term::pair(Term::fst(p()), Term::snd(p()));
    assert!(!converts(&eta, &p()));
    assert!(converts(&eta, &eta));
}

/// `Opt' = None | Some(Σ k:Nat. k == k)` — a constructor whose payload is an
/// `exists`.
fn exists_option() -> Type {
    Type::sum(Type::Unit, reflexive_statement("k"))
}

/// The split `case o of inl u => refl o' | inr b => <arm>` proving
/// `Eq<Opt', o, o>` would need each constructor rebuilt; the arm is `arm`.
fn exists_split(arm: Term) -> Term {
    Term::case(
        Term::var("o"),
        "u",
        Term::refl(exists_option(), Term::inl(exists_option(), Term::Unit)),
        "b",
        arm,
    )
}

/// 4.10.26h AC1 (D4): to the case split a `Sigma` payload is ONE field (k = 1)
/// — a two-field chain projecting its witness and evidence is refused, so the
/// split's surjective pairing never reaches a dependent pair.
#[test]
fn a_sigma_payload_is_one_field_to_the_case_split() {
    let ctx = Context::new().with_term("o", exists_option()).for_proof();
    let goal = Type::eq(exists_option(), Term::var("o"), Term::var("o"));
    let rebuild = |payload: Term| Term::refl(exists_option(), Term::inr(exists_option(), payload));
    let one_field = Term::app(
        Term::lambda("z", reflexive_statement("k"), rebuild(Term::var("z"))),
        Term::var("b"),
    );
    assert_eq!(
        type_of_dependent_case(&ctx, &exists_split(one_field), &goal),
        Ok(goal.clone())
    );
    let two_fields = Term::app(
        Term::app(
            Term::lambda(
                "w",
                Type::Nat,
                Term::lambda(
                    "e",
                    eq_nat(Term::var("w"), Term::var("w")),
                    rebuild(Term::var("b")),
                ),
            ),
            Term::fst(Term::var("b")),
        ),
        Term::snd(Term::var("b")),
    );
    assert_eq!(
        type_of_dependent_case(&ctx, &exists_split(two_fields), &goal),
        Err(TypeError::DependentCaseShape {
            why: "the payload has fewer components than the chain"
        })
    );
}
