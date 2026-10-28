//! ADR 28.9.26d D6 / D10: how the δ environment orders and drops nullary
//! globals, and what dedup keeps. Split from `tests.rs` for size.

use std::collections::{BTreeMap, HashMap};

use albite_core::eval::EvalStopped;
use albite_core::terms::termination::AdmissionState;
use albite_core::Term;

use super::numerals::canonicalize_numerals;
use super::tests::{admission_of, obligation};
use super::*;
use crate::span::Span;

/// D6: nullaries settle dependencies first — `alpha` needs `beta`'s value, and
/// sorts before it, so a name-ordered walk would leave `alpha` stuck.
#[test]
fn a_nullary_settles_after_the_nullary_it_reaches() {
    let candidates: HashMap<String, Term> = [
        ("alpha".to_string(), Term::succ(Term::Global("beta".into()))),
        (
            "beta".to_string(),
            Term::nat_add(Term::nat(1), Term::nat(1)),
        ),
    ]
    .into();
    let total = AdmissionState::Total;
    let admission = admission_of(&[("alpha", total), ("beta", total)]);
    let obligations = [obligation(Term::Global("alpha".into()), Term::nat(3))];
    let env = delta_globals(&candidates, &admission, &obligations, 1_000);
    assert_eq!(canonicalize_numerals(&env.globals["alpha"]), Term::nat(3));
    assert!(discharge(&obligations, &env, 1_000).is_empty());
}

/// D6: only a cycle's own members are named in its note, even when another
/// cycle is reachable from it.
#[test]
fn a_cycle_names_only_its_own_members() {
    let loop_through = |own: &str, other: &str| {
        Term::nat_add(Term::Global(own.into()), Term::Global(other.into()))
    };
    let candidates: HashMap<String, Term> = [
        ("a".to_string(), loop_through("a", "b")),
        ("b".to_string(), Term::succ(Term::Global("b".into()))),
    ]
    .into();
    let admission = admission_of(&[("a", AdmissionState::Total), ("b", AdmissionState::Total)]);
    let obligations = [obligation(Term::Global("a".into()), Term::nat(2))];
    let env = delta_globals(&candidates, &admission, &obligations, 1_000);
    let self_loop = |name: &str| EvalStopped::BlackHole {
        cycle: vec![name.into(), name.into()],
    };
    assert_eq!(env.dropped["a"], self_loop("a"));
    assert_eq!(env.dropped["b"], self_loop("b"));
}

#[test]
fn settle_order_is_dependencies_first_once_each_and_skips_cycles() {
    let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
    let pending: BTreeMap<String, std::collections::BTreeSet<String>> = [
        ("a".to_string(), set(&["b", "c"])),
        ("b".to_string(), set(&["c"])),
        ("c".to_string(), set(&[])),
        ("d".to_string(), set(&[])),
    ]
    .into();
    let order = super::delta::settle_order(&pending, &set(&["d"]));
    assert_eq!(order, vec!["c", "b", "a"]);

    // Independent nullaries each settle: none is mistaken for already visited.
    let independent: BTreeMap<String, std::collections::BTreeSet<String>> =
        [("x".to_string(), set(&[])), ("y".to_string(), set(&[]))].into();
    let none_cyclic = std::collections::BTreeSet::new();
    assert_eq!(
        super::delta::settle_order(&independent, &none_cyclic),
        vec!["x", "y"]
    );
}

#[test]
fn dedup_keeps_obligations_that_differ_in_any_one_field() {
    let make = |start, left: &str, right| {
        let mut placed = obligation(Term::Global(left.into()), Term::nat(right));
        placed.error.span = Span::new(start, start + 4);
        placed
    };
    assert_eq!(
        dedup_and_sort(vec![make(10, "two", 3), make(20, "two", 3)]).len(),
        2
    );
    assert_eq!(
        dedup_and_sort(vec![make(10, "two", 3), make(10, "six", 3)]).len(),
        2
    );
    assert_eq!(
        dedup_and_sort(vec![make(10, "two", 3), make(10, "two", 4)]).len(),
        2
    );
    let mut elsewhere = make(10, "two", 3);
    elsewhere.error.file_path = Some("other.tg".into());
    assert_eq!(dedup_and_sort(vec![make(10, "two", 3), elsewhere]).len(), 2);
    assert_eq!(
        dedup_and_sort(vec![make(10, "two", 3), make(10, "two", 3)]).len(),
        1
    );
}
