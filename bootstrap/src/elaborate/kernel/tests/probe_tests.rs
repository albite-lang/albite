//! The probes' pure parts (ADR 30.9.26f P0.4, D6), moved here with the
//! probes from the census by ADR 2.10.26d D10.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use albite_core::terms::termination::AdmissionState;
use albite_core::typecheck::AdmittedUnfoldings;
use albite_core::{Term, Type, TypeError};

use super::{def, eq_nat, probes_over, total};
use crate::elaborate::kernel::{delta_admission, delta_candidates, ElaboratedProbes, Probes};
use crate::span::Span;

fn mismatch(expected: Type, got: Type) -> TypeError {
    TypeError::TypeMismatch { expected, got }
}

/// 30.9.26f D6, ADR 2.10.26l D3: δ unfolds only admitted-`Total`,
/// effect-free bodies — and, since 2.10.26l, only those the kernel types.
#[test]
fn delta_candidates_are_total_and_effect_free() {
    let effect = Term::extern_call("tg_print".to_string(), vec![]);
    let span = Span::new(0, 1);
    let defs = [
        def("pure_total", Type::Nat, Term::Zero, span),
        def("effectful_total", Type::Unit, effect.clone(), span),
        def("pure_partial", Type::Nat, Term::Zero, span),
        def("effectful_partial", Type::Unit, effect, span),
        def("ill_typed_total", Type::Nat, Term::True, span),
    ];
    let mut admission = BTreeMap::from([
        ("pure_total".to_string(), AdmissionState::Total),
        ("effectful_total".to_string(), AdmissionState::Total),
        ("pure_partial".to_string(), AdmissionState::Partial),
        ("effectful_partial".to_string(), AdmissionState::Partial),
    ]);
    admission.insert("ill_typed_total".to_string(), AdmissionState::Total);
    let mut delta = delta_admission(&defs, &admission, None);
    let kept = delta_candidates(&mut delta);
    assert_eq!(kept.names(), BTreeSet::from(["pure_total"]));
}

fn no_globals() -> ElaboratedProbes {
    ElaboratedProbes {
        concrete_type_names: HashSet::from(["List".to_string()]),
        unfoldable: AdmittedUnfoldings::default(),
    }
}

/// 30.9.26f P0.4: the conversion probe equates what `+` computes, under a
/// matching binder, and nothing else.
#[test]
fn the_conversion_probe_equates_only_convertible_equalities() {
    let probes = no_globals();
    let zero_plus_zero = Term::nat_add(Term::Zero, Term::Zero);
    let inferred = eq_nat(zero_plus_zero.clone(), zero_plus_zero.clone());
    let expected = eq_nat(zero_plus_zero, Term::Zero);
    assert!(probes.converts(&expected, &inferred));
    let one = Term::succ(Term::Zero);
    assert!(!probes.converts(&eq_nat(Term::Zero, one.clone()), &eq_nat(one.clone(), one)));
    let two = Term::succ(Term::succ(Term::Zero));
    assert!(!probes.converts(&eq_nat(Term::Zero, two), &eq_nat(Term::Zero, Term::Zero)));
    let plus_zero = Term::nat_add(Term::var("n"), Term::Zero);
    let stated = Type::pi("n", Type::Nat, eq_nat(plus_zero.clone(), Term::var("n")));
    let proved = Type::pi("n", Type::Nat, eq_nat(plus_zero.clone(), plus_zero));
    assert!(probes.converts(&stated, &proved));
    let other_binder = Type::pi("m", Type::Nat, eq_nat(Term::var("n"), Term::var("n")));
    assert!(!probes.converts(&stated, &other_binder));
    let other_domain = Type::pi("n", Type::Bool, eq_nat(Term::var("n"), Term::var("n")));
    assert!(!probes.converts(&stated, &other_domain));
    let arrow = |dom: Type| Type::arrow(dom, expected.clone());
    assert!(probes.converts(&arrow(Type::Nat), &Type::arrow(Type::Nat, inferred.clone())));
    assert!(!probes.converts(
        &arrow(Type::Bool),
        &Type::arrow(Type::Nat, inferred.clone())
    ));
    assert!(!probes.converts(
        &Type::arrow(Type::Nat, Type::Nat),
        &Type::arrow(Type::Nat, Type::Nat)
    ));
    assert!(!probes.converts(&Type::Nat, &Type::Nat));
    assert!(!probes.converts(&Type::eq(Type::Bool, Term::Zero, Term::Zero), &inferred));
}

/// 30.9.26f P0.4: δ over the admitted environment — `two` unfolds when it is
/// in the engine's globals, and only then; the δ-free probe never unfolds it
/// (ADR 2.10.26d D5).
#[test]
fn the_conversion_probe_unfolds_only_its_globals() {
    let two = Term::succ(Term::succ(Term::Zero));
    let stated = eq_nat(Term::global("two"), two.clone());
    let proved = eq_nat(Term::global("two"), Term::global("two"));
    assert!(!no_globals().converts(&stated, &proved));
    let with_two = probes_over(
        &[def("two", Type::Nat, two, Span::new(0, 1))],
        &total(&["two"]),
    );
    assert!(with_two.converts(&stated, &proved));
    assert!(!with_two.converts_without_delta(&stated, &proved));
}

/// 30.9.26f D6: which probes each refusal gets.
#[test]
fn probes_for_asks_only_what_the_error_can_use() {
    let probes = no_globals();
    let marker = probes.probes_for(&TypeError::UnboundTypeVar("@List".into()));
    assert_eq!(
        (marker.marker, marker.names_concrete_type),
        (Some(true), Some(false))
    );
    assert_eq!((marker.lenient_equal, marker.converts), (None, None));
    let alpha = probes.probes_for(&TypeError::UnboundTypeVar("α_List".into()));
    assert_eq!(alpha.marker, Some(true));
    let named = probes.probes_for(&TypeError::UnboundTypeVar("List".into()));
    assert_eq!(
        (named.marker, named.names_concrete_type),
        (Some(false), Some(true))
    );
    // The μ's own `α_L` is bound, so only the free `List` is looked at.
    let body = Type::sum(
        Type::Unit,
        Type::product(Type::TyVar("List".into()), Type::TyVar("α_L".into())),
    );
    let malformed = probes.probes_for(&TypeError::MalformedType(Type::mu("α_L", body)));
    assert_eq!(
        (malformed.marker, malformed.names_concrete_type),
        (Some(false), Some(true))
    );
    assert_eq!((malformed.lenient_equal, malformed.converts), (None, None));
    let free_marker = probes.probes_for(&TypeError::MalformedType(Type::TyVar("@X".into())));
    assert_eq!(
        (free_marker.marker, free_marker.names_concrete_type),
        (Some(true), Some(false))
    );
    let pair = probes.probes_for(&mismatch(Type::Nat, Type::Nat));
    assert_eq!(
        (pair.lenient_equal, pair.converts, pair.marker),
        (Some(true), Some(false), None)
    );
    let zero_plus_zero = Term::nat_add(Term::Zero, Term::Zero);
    let convertible = probes.probes_for(&mismatch(
        eq_nat(zero_plus_zero.clone(), Term::Zero),
        eq_nat(zero_plus_zero.clone(), zero_plus_zero),
    ));
    assert_eq!(
        (convertible.lenient_equal, convertible.converts),
        (Some(false), Some(true))
    );
    assert_eq!(probes.probes_for(&TypeError::EmptyMatch), Probes::default());
}
