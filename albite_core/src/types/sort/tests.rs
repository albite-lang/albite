//! ADR 4.10.26i AC1: one test per rule of D1's sort judgment.

use super::Sort;
use crate::terms::Term;
use crate::types::Type;

fn zero_eq_zero() -> Type {
    Type::eq(Type::Nat, Term::Zero, Term::Zero)
}

/// 4.10.26i AC1: `Eq` and `Void` are propositions.
#[test]
fn eq_and_void_are_prop() {
    assert_eq!(zero_eq_zero().sort_of(), Some(Sort::Prop));
    assert_eq!(Type::Void.sort_of(), Some(Sort::Prop));
}

/// 4.10.26i AC1: ∧ over propositions is `Prop`; with one data side it is data.
#[test]
fn a_conjunction_of_propositions_is_prop() {
    let both = Type::product(zero_eq_zero(), Type::Void);
    assert_eq!(both.sort_of(), Some(Sort::Prop));
    assert_eq!(
        Type::product(zero_eq_zero(), Type::Nat).sort_of(),
        Some(Sort::Type0)
    );
    assert_eq!(
        Type::product(Type::Nat, zero_eq_zero()).sort_of(),
        Some(Sort::Type0)
    );
}

/// 4.10.26i AC1: ∨ over propositions is `Prop`; with one data side it is data.
#[test]
fn a_disjunction_of_propositions_is_prop() {
    assert_eq!(
        Type::sum(zero_eq_zero(), Type::Void).sort_of(),
        Some(Sort::Prop)
    );
    assert_eq!(
        Type::sum(Type::Bool, Type::Void).sort_of(),
        Some(Sort::Type0)
    );
    assert_eq!(
        Type::sum(Type::Void, Type::Bool).sort_of(),
        Some(Sort::Type0)
    );
}

/// 4.10.26i AC1: → and Π sort as their codomain, ¬P = `P → Void` included.
#[test]
fn an_implication_sorts_as_its_codomain() {
    let negation = Type::arrow(zero_eq_zero(), Type::Void);
    assert_eq!(negation.sort_of(), Some(Sort::Prop));
    assert_eq!(
        Type::arrow(Type::Nat, zero_eq_zero()).sort_of(),
        Some(Sort::Prop)
    );
    assert_eq!(
        Type::arrow(zero_eq_zero(), Type::Nat).sort_of(),
        Some(Sort::Type0)
    );
    let dependent = Type::pi(
        "n",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
    );
    assert_eq!(dependent.sort_of(), Some(Sort::Prop));
    assert_eq!(
        Type::pi("n", Type::Nat, Type::Nat).sort_of(),
        Some(Sort::Type0)
    );
}

/// 4.10.26i AC1: ∃ sorts as its body.
#[test]
fn an_existential_sorts_as_its_body() {
    let exists = Type::sigma(
        "k",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("k"), Term::Zero),
    );
    assert_eq!(exists.sort_of(), Some(Sort::Prop));
    assert_eq!(
        Type::sigma("k", Type::Nat, Type::Nat).sort_of(),
        Some(Sort::Type0)
    );
}

/// 4.10.26i AC1: `Forall(α, B)` sorts as `B` — impredicative over data.
#[test]
fn forall_sorts_as_its_body() {
    let refl_at = Type::forall("T", Type::arrow(Type::TyVar("T".into()), zero_eq_zero()));
    assert_eq!(refl_at.sort_of(), Some(Sort::Prop));
    assert_eq!(
        Type::forall("T", Type::TyVar("T".into())).sort_of(),
        Some(Sort::Type0)
    );
}

/// 4.10.26i AC1: ⊤ is `0 == 0`, and `Unit` is data (3.10.26h D4 overturn).
#[test]
fn unit_is_data_and_top_is_zero_eq_zero() {
    assert_eq!(Type::Unit.sort_of(), Some(Sort::Type0));
    assert_eq!(zero_eq_zero().sort_of(), Some(Sort::Prop));
}

/// 4.10.26i AC1: `type Dir = L | R` must not sort as a proposition.
#[test]
fn a_two_constructor_enum_is_data() {
    assert_eq!(
        Type::sum(Type::Unit, Type::Unit).sort_of(),
        Some(Sort::Type0)
    );
}

/// 4.10.26i AC1: every type variable is `Type₀` until `1b.3`.
#[test]
fn a_type_variable_is_data() {
    assert_eq!(Type::TyVar("α".into()).sort_of(), Some(Sort::Type0));
}

/// 4.10.26i AC1: every remaining former is data.
#[test]
fn the_data_formers_are_type0() {
    let data = [
        Type::Bool,
        Type::Nat,
        Type::Int,
        Type::String,
        Type::mu("α", Type::sum(Type::Unit, Type::TyVar("α".into()))),
        Type::adt("Dir", vec![], vec![("L".into(), Type::Unit)]),
        Type::ptr(Type::Nat),
        Type::ref_ty(Type::Nat),
    ];
    for ty in data {
        assert_eq!(ty.sort_of(), Some(Sort::Type0), "{ty}");
    }
}

/// 4.10.26i AC1: a poison and a deferred application have no sort, and
/// neither does a connective that contains one.
#[test]
fn error_and_app_have_no_sort() {
    assert_eq!(Type::Error.sort_of(), None);
    assert_eq!(Type::app("F", vec![]).sort_of(), None);
    assert_eq!(Type::product(Type::Error, Type::Void).sort_of(), None);
    assert_eq!(Type::sum(Type::Void, Type::Error).sort_of(), None);
    assert_eq!(Type::arrow(Type::Nat, Type::Error).sort_of(), None);
}

/// 7.10.26l AC4 (D4): a disjunction sorts `Prop` but is NOT an erasable
/// proposition — which side holds is information, and a `match` on it reads
/// a real tag. Widening `is_proposition` to `sort_of` must break this test.
#[test]
fn a_disjunction_is_prop_but_never_erased() {
    let one_eq_one = Type::eq(Type::Nat, Term::succ(Term::Zero), Term::succ(Term::Zero));
    let equations = Type::sum(zero_eq_zero(), one_eq_one);
    let le = |n: Term| Type::sigma("k", Type::Nat, Type::eq(Type::Nat, n, Term::var("m")));
    let orders = Type::sum(le(Term::Zero), le(Term::var("n")));
    for disjunction in [equations, orders] {
        assert_eq!(disjunction.sort_of(), Some(Sort::Prop), "{disjunction}");
        assert!(!disjunction.is_proposition(), "{disjunction}");
    }
    // Its sides each are: the narrow key answers per former, not per sort.
    assert!(zero_eq_zero().is_proposition());
}
