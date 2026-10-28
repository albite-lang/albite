//! ADR 7.10.26l AC 1 — `left` and `right` (D2): the goal each leaves, the
//! template each installs, and the graft through `Inl`/`Inr` that zonks it.

use albite_core::{Term, Type};

use super::{eq_nat, tactic_error};
use crate::elaborate::error::TacticErrorKind;
use crate::elaborate::tactics::state::{hole_name, Partial, ProofState};
use crate::elaborate::tests::elab_ok;

fn zero_or_one() -> Type {
    Type::sum(
        eq_nat(Term::Zero, Term::Zero),
        eq_nat(Term::succ(Term::Zero), Term::succ(Term::Zero)),
    )
}

// ADR 7.10.26l AC1 (D2): a hole under `Inl` and under `Inr` is grafted — the
// walk that left `%goal1` unbound before it learned the two injections.
#[test]
fn zonk_grafts_under_inl_and_inr() {
    for inject in [Term::inl, Term::inr] {
        let mut state = ProofState::new(zero_or_one());
        let side = state.fresh_meta(vec![], eq_nat(Term::Zero, Term::Zero));
        state.refine(
            0,
            Partial {
                template: inject(zero_or_one(), Term::var(hole_name(side))),
                holes: vec![(hole_name(side), side)],
            },
        );
        let proof = Term::Refl(Type::Nat, Box::new(Term::Zero));
        state.refine(side, Partial::closed(proof.clone()));
        let zonked = state.zonk(0).expect("every meta is solved");
        assert_eq!(zonked, inject(zero_or_one(), proof));
    }
}

/// The definition `name`'s term in `source`, spans and parameter `λ`s peeled.
fn proof_of(source: &str, name: &str) -> Term {
    let defs = elab_ok(source);
    let mut term = defs
        .iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no `{name}`"))
        .term
        .term
        .strip_spans();
    while let Term::Lambda(_, _, body) = term {
        term = *body;
    }
    term
}

// ADR 7.10.26l AC1 (D2): `left` installs `Inl(A + B, ?)` and its goal is `A`,
// which `refl` closes; `right` the same with `Inr` and `B`.
#[test]
fn left_and_right_install_the_injection() {
    let left = proof_of(
        "theorem l(n: Nat) -> Eq<Nat, n, n> + Eq<Nat, 0, 1> by { left; refl }",
        "l",
    );
    let Term::Inl(Type::Sum(chosen, _), proof) = &left else {
        panic!("expected Inl, got {left}");
    };
    assert!(matches!(**chosen, Type::Eq(..)), "{chosen}");
    assert!(matches!(**proof, Term::Refl(..)), "{proof}");
    let right = proof_of(
        "theorem r(n: Nat) -> Eq<Nat, 0, 1> + Eq<Nat, n, n> by { right; refl }",
        "r",
    );
    assert!(matches!(right, Term::Inr(Type::Sum(..), _)), "{right}");
}

// ADR 7.10.26l AC1 (D2): `left` leaves the left side open — the wrong side's
// equation is what `refl` then refuses, and an unfinished block names it.
#[test]
fn the_side_left_open_is_the_chosen_one() {
    let TacticErrorKind::UnsolvedGoals { goals } =
        tactic_error("theorem l() -> Eq<Nat, 0, 1> + Eq<Nat, 2, 2> by { right }")
    else {
        panic!("expected an unsolved goal");
    };
    let two = Term::succ(Term::succ(Term::Zero));
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].1, eq_nat(two.clone(), two));
}
