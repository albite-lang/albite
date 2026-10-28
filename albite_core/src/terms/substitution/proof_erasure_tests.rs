//! Proofs are erased before codegen, nothing else is (ADR 3.10.26e AC 4).

use crate::terms::Term;
use crate::types::Type;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `0 + i = i`, the motive body of `add_zero_left`.
fn zero_add_body() -> Type {
    eq_nat(
        Term::NatAdd(Box::new(Term::Zero), Box::new(Term::var("i"))),
        Term::var("i"),
    )
}

/// `natind [i. Nat → body] base step n`, with a step that is a plain `λ`.
fn natind_over(body: Type, base: Term, step: Term) -> Term {
    Term::natind(
        "i",
        Type::arrow(Type::Nat, body),
        base,
        step,
        Term::var("n"),
    )
}

fn proof_step() -> Term {
    Term::lambda(
        "j",
        Type::Nat,
        Term::lambda("ih", Type::Unit, Term::var("ih")),
    )
}

/// 3.10.26e AC 4: an effect-free `natind` over an `Eq` is the unit value.
#[test]
fn a_proof_valued_natind_erases_to_unit() {
    let proof = natind_over(
        zero_add_body(),
        Term::refl(Type::Nat, Term::Zero),
        proof_step(),
    );
    assert_eq!(proof.erase_proofs(), Term::Unit);
}

/// 3.10.26e AC 4: a `natind` computing a `Nat` is data, and is kept whole.
#[test]
fn a_data_valued_natind_is_kept() {
    let data = natind_over(Type::Nat, Term::Zero, proof_step());
    assert_eq!(data.erase_proofs(), data);
}

/// 3.10.26e AC 4: a proof bound in a `let` under a `λ` is reached, and the
/// surrounding term is left as it was.
#[test]
fn erasure_reaches_under_lambdas_and_lets() {
    let proof = natind_over(
        zero_add_body(),
        Term::refl(Type::Nat, Term::Zero),
        proof_step(),
    );
    let wrap = |inner: Term| {
        Term::lambda(
            "n",
            Type::Nat,
            Term::let_in("p", zero_add_body(), inner, Term::var("n")),
        )
    };
    assert_eq!(wrap(proof).erase_proofs(), wrap(Term::Unit));
}

/// 3.10.26e AC 4: a term with no `natind` comes back unchanged.
#[test]
fn a_term_without_natind_is_unchanged() {
    let term = Term::lambda(
        "n",
        Type::Nat,
        Term::app(Term::global("f"), Term::refl(Type::Nat, Term::var("n"))),
    );
    assert_eq!(term.erase_proofs(), term);
}

/// 3.10.26e AC 4: a generalised motive erases to a `λ` per argument, with the
/// argument's own domain, so an application around it still type-checks.
#[test]
fn a_function_valued_proof_natind_keeps_its_arity() {
    let generalised = Type::pi(
        "m",
        Type::Nat,
        Type::arrow(Type::Bool, eq_nat(Term::var("m"), Term::var("i"))),
    );
    let proof = natind_over(generalised, Term::var("base"), proof_step());
    let Term::Lambda(_, first, rest) = proof.erase_proofs() else {
        panic!("a generalised proof erases to a λ");
    };
    assert_eq!(first, Type::Nat);
    let Term::Lambda(_, second, body) = *rest else {
        panic!("one λ per argument");
    };
    assert_eq!(second, Type::Bool);
    assert_eq!(*body, Term::Unit);
}

/// 3.10.26e AC 4: an extern call in any of the three parts keeps the whole
/// `natind`, so erasure drops no effect.
#[test]
fn an_effectful_proof_natind_is_kept() {
    let effect = Term::ExternCall("tg_print".to_owned(), vec![]);
    let in_step = natind_over(
        zero_add_body(),
        Term::refl(Type::Nat, Term::Zero),
        Term::lambda("j", Type::Nat, effect.clone()),
    );
    assert_eq!(in_step.erase_proofs(), in_step);
    let in_base = natind_over(zero_add_body(), effect.clone(), proof_step());
    assert_eq!(in_base.erase_proofs(), in_base);
    let in_scrutinee = Term::natind(
        "i",
        Type::arrow(Type::Nat, zero_add_body()),
        Term::refl(Type::Nat, Term::Zero),
        proof_step(),
        effect,
    );
    assert_eq!(in_scrutinee.erase_proofs(), in_scrutinee);
}

/// 3.10.26e AC 4: a motive that is not stored as a function is kept: there is
/// no body to read a proposition from.
#[test]
fn a_natind_whose_motive_is_not_a_function_is_kept() {
    let odd = Term::natind(
        "i",
        zero_add_body(),
        Term::Zero,
        proof_step(),
        Term::var("n"),
    );
    assert_eq!(odd.erase_proofs(), odd);
}

/// 3.10.26e AC 4: `Eq` and `Void` are propositions, as is any arrow or `Π`
/// ending in one; `Nat`, an ADT and an arrow ending in data are not.
#[test]
fn is_proposition_reads_through_arrows_and_pis() {
    let eq = eq_nat(Term::Zero, Term::Zero);
    assert!(eq.is_proposition());
    assert!(Type::Void.is_proposition());
    assert!(Type::arrow(Type::Nat, eq.clone()).is_proposition());
    assert!(Type::pi("x", Type::Nat, Type::Void).is_proposition());
    assert!(!Type::Nat.is_proposition());
    assert!(!Type::adt("Option", vec![], vec![]).is_proposition());
    assert!(!Type::arrow(eq.clone(), Type::Nat).is_proposition());
    assert!(!Type::pi("x", eq, Type::Bool).is_proposition());
}

/// 4.10.26h AC2 (D6): a `Sigma` is never a proposition, even over an `Eq` —
/// its witness is data a body may project at runtime, so it must not erase.
#[test]
fn a_sigma_over_an_equality_is_not_a_proposition() {
    let sigma = Type::sigma("k", Type::Nat, eq_nat(Term::var("k"), Term::Zero));
    assert!(!sigma.is_proposition());
    assert!(!Type::arrow(Type::Nat, sigma).is_proposition());
}

/// 4.10.26i AC5 (D8): `sort_of` and `is_proposition` disagree on purpose.
/// `Le<n, m>`'s expansion and a conjunction each sort `Prop`, and neither is
/// erased: erasure stays keyed on `is_proposition` alone.
#[test]
fn erasure_stays_keyed_on_is_proposition_not_sort_of() {
    use crate::types::Sort;
    let le = Type::sigma(
        "k",
        Type::Nat,
        eq_nat(
            Term::NatAdd(Box::new(Term::var("n")), Box::new(Term::var("k"))),
            Term::var("m"),
        ),
    );
    let both = Type::product(eq_nat(Term::Zero, Term::Zero), Type::Void);
    for ty in [le, both] {
        assert_eq!(ty.sort_of(), Some(Sort::Prop), "{ty}");
        assert!(!ty.is_proposition(), "{ty}");
    }
}
