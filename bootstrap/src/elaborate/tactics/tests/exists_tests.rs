//! ADR 6.10.26e AC 1 — `apply`'s premise rule, one case per D2 row — and
//! AC 2/AC 3 — `exists w` (D4) and the codes the must-fail twin pins.

use albite_core::{Term, Type};

use super::eq_nat;
use crate::elaborate::tactics::is_tactic_proposition;
use crate::elaborate::tactics::steps::premise_refusal;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::CoreDef;

const LE: &str = "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n";

fn zero_eq() -> Type {
    eq_nat(Term::Zero, Term::Zero)
}

/// `exists k: Nat. 0 == 0`.
fn exists_zero_eq() -> Type {
    Type::sigma("k", Type::Nat, zero_eq())
}

const DATA_PREMISE_REFUSAL: &str = "cannot infer an argument of type";
const GENERIC_PREMISE_REFUSAL: &str = "a generic premise cannot become a goal in S1";

// ADR 6.10.26e AC1: ∧, ∨, ∃, a family's expansion and an arrow ending in ∃
// sort `Prop`, so each becomes a goal.
#[test]
fn a_premise_that_sorts_prop_becomes_a_goal() {
    let family = Type::sigma(
        "k",
        Type::Nat,
        eq_nat(
            Term::nat_add(Term::var("a"), Term::var("k")),
            Term::var("b"),
        ),
    );
    for premise in [
        zero_eq(),
        Type::product(zero_eq(), zero_eq()),
        Type::sum(zero_eq(), zero_eq()),
        exists_zero_eq(),
        family,
        Type::arrow(Type::Nat, exists_zero_eq()),
        Type::pi("n", Type::Nat, exists_zero_eq()),
    ] {
        assert!(is_tactic_proposition(&premise), "{premise}");
        assert_eq!(premise_refusal(&premise), None, "{premise}");
    }
}

// ADR 6.10.26e AC1: a `Forall` over a proposition sorts `Prop` and is still
// refused, alone or at the end of an arrow.
#[test]
fn a_generic_premise_is_refused() {
    let generic = Type::Forall("T".to_string(), Box::new(zero_eq()));
    assert!(is_tactic_proposition(&generic));
    for premise in [
        generic.clone(),
        Type::arrow(Type::Nat, generic.clone()),
        Type::pi("n", Type::Nat, generic),
    ] {
        assert_eq!(
            premise_refusal(&premise).as_deref(),
            Some(GENERIC_PREMISE_REFUSAL),
            "{premise}"
        );
    }
}

// ADR 6.10.26e AC1: data — `Nat`, `Unit`, a user ADT, a data product — and a
// deferred `App` are refused with today's message.
#[test]
fn a_data_premise_is_refused_with_todays_message() {
    let adt = Type::adt("Opt", vec![], vec![("None".to_string(), Type::Unit)]);
    let app = Type::App("Le".to_string(), vec![Type::Nat]);
    for premise in [
        Type::Nat,
        Type::Unit,
        adt,
        Type::product(zero_eq(), Type::Nat),
        Type::Forall("T".to_string(), Box::new(Type::Nat)),
        app,
    ] {
        assert!(!is_tactic_proposition(&premise), "{premise}");
        let why = premise_refusal(&premise).unwrap_or_else(|| panic!("{premise} became a goal"));
        assert_eq!(why, format!("{DATA_PREMISE_REFUSAL} `{premise}`"));
    }
}

fn core_of(defs: &[CoreDef], name: &str) -> Term {
    defs.iter()
        .find(|def| def.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
        .term
        .term
        .strip_spans()
}

// ADR 6.10.26e AC2: `exists w` emits the term the tuple check builds,
// `Annot(Pair(w, proof), Σ…)`, and `by_term` and `by_tactic` agree. An
// unrelated definition comes first, so a lookup that found the wrong
// definition could not pass by finding the other proof.
#[test]
fn exists_emits_the_annotated_pair() {
    let defs = elab_ok(&format!(
        "{LE}theorem unrelated(n: Nat) -> n == n {{ refl }}
         theorem by_term(n: Nat) -> Le<n, n + 2> {{ (2, refl) }}
         theorem by_tactic(n: Nat) -> Le<n, n + 2> by {{ exists 2; refl }}"
    ));
    let by_tactic = core_of(&defs, "by_tactic");
    assert_eq!(core_of(&defs, "by_term"), by_tactic);
    let Term::Lambda(_, _, body) = &by_tactic else {
        panic!("expected the parameter's λ, got {by_tactic}");
    };
    let Term::Annot(pair, ty) = &**body else {
        panic!("expected an annotation, got {by_tactic}");
    };
    assert!(matches!(ty, Type::Sigma(..)), "{ty}");
    assert!(matches!(**pair, Term::Pair(..)), "{pair}");
}

// ADR 6.10.26e AC2: the rest is a goal — `apply` of a family premise, then
// `exists` under it.
#[test]
fn exists_leaves_the_body_as_a_goal_for_later_steps() {
    elab_ok(&format!(
        "{LE}theorem le_keep(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> {{ h }}
         theorem t(a: Nat) -> Le<a, a> by {{ apply le_keep(a, a); exists 0; refl }}"
    ));
}

/// The code of the one error `source` raises after `LE`.
fn code_of(source: &str) -> &'static str {
    let errors = elab_err(&format!("{LE}{source}"));
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors[0].kind.code()
}

// ADR 6.10.26e AC3: each must-fail twin's code, pinned (D8).
#[test]
fn the_must_fail_twins_are_refused_with_their_own_codes() {
    assert_eq!(
        code_of("theorem t(n: Nat) -> n == n by { exists 1 }"),
        "E0082"
    );
    assert_eq!(
        code_of("theorem t(n: Nat) -> Le<n, n> by { exists true }"),
        "E0010"
    );
    assert_eq!(
        code_of("theorem t(n: Nat) -> Le<n, n> by { exists 1; refl }"),
        "E0071"
    );
    assert_eq!(
        code_of("theorem t(n: Nat) -> Le<n, n> by { cases n { (k, e) => exact (0, refl) } }"),
        "E0082"
    );
    assert_eq!(
        code_of(
            "theorem nat_premise(a: Nat, z: Nat) -> Le<a, a> { (0, refl) }
             theorem t(a: Nat) -> Le<a, a> by { apply nat_premise }"
        ),
        "E0082"
    );
}

// ADR 6.10.26e AC3: `exists` on an equation names why.
#[test]
fn exists_on_an_equation_says_it_needs_an_exists_goal() {
    let why = super::tactic_error(&format!(
        "{LE}theorem t(n: Nat) -> n == n by {{ exists 1 }}"
    ));
    assert!(
        format!("{why:?}").contains("`exists` needs an `exists` goal"),
        "{why:?}"
    );
}
