//! ADR 29.9.26c: the WHNF tests that drive the bootstrap's δ discharge or an
//! `Elaborator`, kept here when the engine moved to `albite_core::conversion`
//! (ADR 30.9.26g D6). Names unchanged, bar one split; the pure halves moved
//! with the engine.

use std::collections::HashMap;

use albite_core::conversion::{convertible, rename_apart, Convertible};
use albite_core::{Term, Type};

use super::tests::obligation;
use super::{discharge, DeltaEnv};

fn var(name: &str) -> Term {
    Term::var(name)
}

/// The by-name comparison under no globals and a generous budget — the
/// core's own `same` test helper, re-declared because `#[cfg(test)]` items
/// do not cross crates.
fn by_name(left: &Term, right: &Term) -> Convertible {
    let (left, right) = rename_apart(left, right);
    convertible(&left, &right, &HashMap::new(), 10_000)
}

/// An extern the registry cannot run: CBV leaves it stuck, WHNF never runs
/// over it (D5), and by-name `let` would simply drop it.
fn unregistered_extern() -> Term {
    Term::extern_call("tg_not_in_the_registry", vec![])
}

// 29.9.26c D1: a CBV stop keeps its verdict even where WHNF would say yes.
#[test]
fn a_cbv_stop_is_never_overridden_by_whnf() {
    // By name the argument is discarded; by value it must be counted first.
    let count = Term::natrec(
        Type::Nat,
        Term::Zero,
        Term::lambda("k", Type::Nat, Term::lambda("acc", Type::Nat, var("acc"))),
        Term::NatLit(50),
    );
    let discarded = Term::app(Term::lambda("x", Type::Nat, Term::Zero), count);
    for (left, right) in [(discarded.clone(), Term::Zero), (Term::Zero, discarded)] {
        let failures = discharge(&[obligation(left, right)], &DeltaEnv::default(), 20);
        assert_eq!(
            failures.len(),
            1,
            "a stopped side is a failure (D7 of 28.9.26d)"
        );
        assert_eq!(
            failures[0].notes[0].message,
            "conversion stopped after 20 steps without reaching a normal form"
        );
    }
}

// 29.9.26c D5: ONE effectful side is enough to keep WHNF out, in the
// discharge and in the fast path.
#[test]
fn one_effectful_side_keeps_whnf_out() {
    let dropped = Term::let_in("_", Type::Unit, unregistered_extern(), Term::Zero);
    assert_eq!(
        by_name(&dropped, &Term::Zero),
        Convertible::Yes,
        "by name, the extern is dropped"
    );
    let mut ctx = albite_core::Context::new();
    let elab = crate::elaborate::Elaborator::new(&mut ctx);
    for (left, right) in [(dropped.clone(), Term::Zero), (Term::Zero, dropped)] {
        let failures = discharge(
            &[obligation(left.clone(), right.clone())],
            &DeltaEnv::default(),
            1_000,
        );
        assert_eq!(
            failures.len(),
            1,
            "{left} ≡ {right} must not be WHNF-compared"
        );
        assert!(!elab.terms_definitionally_equal(&left, &right, &Type::Nat));
    }
}

// 29.9.26c AC5: a CLOSED failure still carries Phase 1's note byte for byte
// (D8); only an open one gets "first differ at".
#[test]
fn a_closed_failure_keeps_the_phase_one_note() {
    let env = DeltaEnv {
        globals: [("two".to_string(), Term::nat(2))].into(),
        ..DeltaEnv::default()
    };
    let closed = obligation(Term::global("two"), Term::nat(3));
    let failures = discharge(&[closed], &env, 1_000);
    assert_eq!(
        failures[0].notes[0].message,
        "after unfolding total definitions, the sides are `(succ (succ zero))` and `(succ (succ (succ zero)))`"
    );
    let open = obligation(Term::nat_add(Term::global("two"), var("n")), Term::nat(3));
    let failures = discharge(&[open], &env, 1_000);
    assert!(
        failures[0].notes[0]
            .message
            .starts_with("the sides first differ at `"),
        "{}",
        failures[0].notes[0].message
    );
}

// 29.9.26c AC3: D5 — the discharge half of the reference-cell test, whose
// pure half (`reference_cells_are_refused_before_by_name_reduction`) moved to
// the core (ADR 30.9.26g D6). By name both sides agree; CBV keeps 1 ≠ 0.
#[test]
fn a_reference_cell_obligation_keeps_its_cbv_verdict() {
    let cell = || Term::ref_new(Term::Zero);
    let written = Term::let_in(
        "r",
        Type::Nat,
        cell(),
        Term::let_in(
            "_",
            Type::Unit,
            Term::ref_set(var("r"), Term::NatLit(1)),
            Term::ref_get(var("r")),
        ),
    );
    let read = Term::let_in("r", Type::Nat, cell(), Term::ref_get(var("r")));
    assert_eq!(by_name(&written, &read), Convertible::Yes);

    let failures = discharge(
        &[obligation(written.clone(), read.clone())],
        &DeltaEnv::default(),
        10_000,
    );
    assert_eq!(failures.len(), 1, "the discharge keeps CBV's 1 ≠ 0");
    let mut ctx = albite_core::Context::new();
    let elab = crate::elaborate::Elaborator::new(&mut ctx);
    assert!(
        !elab.terms_definitionally_equal(&written, &read, &Type::Nat),
        "the fast path keeps CBV's 1 ≠ 0"
    );
}
