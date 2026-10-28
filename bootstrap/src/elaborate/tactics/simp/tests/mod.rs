//! `simp`'s tests (ADR 4.10.26e AC 2).
//!
//! This file holds D3's pure half — the permutative test and the round loop
//! under its bound — over hand-built Core. `elaboration.rs` runs the tactic
//! through `elab_ok`/`elab_err`: D2's rounds and close, and every refusal.

mod elaboration;

use albite_core::Term;

use super::bound::{is_permutative, run_rounds, RoundsOutcome, LAST_RULES_SHOWN, SIMP_STEP_BOUND};

fn var(name: &str) -> Term {
    Term::var(name)
}

fn add(left: Term, right: Term) -> Term {
    Term::NatAdd(Box::new(left), Box::new(right))
}

fn mul(left: Term, right: Term) -> Term {
    Term::NatMul(Box::new(left), Box::new(right))
}

fn vars(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

// ADR 4.10.26e AC2
#[test]
fn add_comm_and_mul_comm_are_permutative() {
    let nm = vars(&["n", "m"]);
    assert!(is_permutative(
        &nm,
        &add(var("n"), var("m")),
        &add(var("m"), var("n"))
    ));
    assert!(is_permutative(
        &nm,
        &mul(var("n"), var("m")),
        &mul(var("m"), var("n"))
    ));
    // Spans do not hide a renaming.
    let spanned = Term::Spanned(
        Box::new(add(var("n"), var("m"))),
        albite_core::TermSpan::new(0, 5),
    );
    assert!(is_permutative(&nm, &spanned, &add(var("m"), var("n"))));
}

// ADR 4.10.26e AC2: an applied instance has no variables left.
#[test]
fn an_applied_instance_is_not_permutative() {
    let applied_left = add(var("a"), var("b"));
    let applied_right = add(var("b"), var("a"));
    assert!(!is_permutative(&[], &applied_left, &applied_right));
    // `a`, `b` are not the lemma's variables: they cannot be renamed.
    assert!(!is_permutative(
        &vars(&["n"]),
        &applied_left,
        &applied_right
    ));
}

// ADR 4.10.26e AC2
#[test]
fn add_zero_left_is_not_permutative() {
    let n = vars(&["n"]);
    assert!(!is_permutative(&n, &add(Term::Zero, var("n")), &var("n")));
    // Associativity moves structure, not just names.
    let nmk = vars(&["n", "m", "k"]);
    let left = add(add(var("n"), var("m")), var("k"));
    let right = add(var("n"), add(var("m"), var("k")));
    assert!(!is_permutative(&nmk, &left, &right));
    // Equal sides rewrite nothing: not a loop, and `simp` skips them.
    assert!(!is_permutative(&n, &var("n"), &var("n")));
    // A renaming that is not one-to-one is not a permutation: `n + m` to
    // `n + n` binds both variables to `n`.
    let two = vars(&["n", "m"]);
    assert!(!is_permutative(
        &two,
        &add(var("n"), var("m")),
        &add(var("n"), var("n"))
    ));
    // A side that becomes a non-variable term is an instance, not a rename.
    assert!(!is_permutative(
        &n,
        &add(var("n"), Term::Zero),
        &add(Term::Zero, Term::Zero)
    ));
}

/// A round that cycles between rules 1 and 2 forever.
fn cycling() -> impl FnMut() -> Result<Option<usize>, ()> {
    let mut next = 0;
    move || {
        next = next % 2 + 1;
        Ok(Some(next))
    }
}

/// A round that applies rule 1 `count` times, then nothing.
fn stable_after(count: usize) -> impl FnMut() -> Result<Option<usize>, ()> {
    let mut taken = 0;
    move || {
        taken += 1;
        Ok((taken <= count).then_some(1))
    }
}

// ADR 4.10.26e AC2 (D3): the bound stops a cycle, and its edge is exact —
// stable after exactly the bound is not refused; a step past it is.
#[test]
fn the_bound_stops_a_cycle() {
    assert_eq!(SIMP_STEP_BOUND, 64);
    assert_eq!(
        run_rounds(SIMP_STEP_BOUND, cycling()),
        Ok(RoundsOutcome::BoundReached {
            last_rules: vec![1, 2, 1, 2]
        })
    );
    assert_eq!(LAST_RULES_SHOWN, 4);
    assert_eq!(
        run_rounds(SIMP_STEP_BOUND, stable_after(SIMP_STEP_BOUND)),
        Ok(RoundsOutcome::Stable {
            steps: SIMP_STEP_BOUND
        })
    );
    assert_eq!(
        run_rounds(SIMP_STEP_BOUND, stable_after(SIMP_STEP_BOUND + 1)),
        Ok(RoundsOutcome::BoundReached {
            last_rules: vec![1; LAST_RULES_SHOWN]
        })
    );
    assert_eq!(
        run_rounds(SIMP_STEP_BOUND, stable_after(0)),
        Ok(RoundsOutcome::Stable { steps: 0 })
    );
}

// ADR 4.10.26e AC2: `simp` after the last goal closed has nothing to act on.
#[test]
fn simp_after_the_last_goal_is_e0082() {
    let kind = crate::elaborate::tactics::tests::tactic_error(
        "theorem t(a: Nat, h: Eq<Nat, a, a>) -> Eq<Nat, a, a> by { refl; simp [h] }",
    );
    let crate::elaborate::error::TacticErrorKind::TacticNotApplicable { tactic, why, .. } = kind
    else {
        panic!("expected E0082, got {kind:?}");
    };
    assert_eq!(
        (tactic.as_str(), why.as_str()),
        ("simp", crate::elaborate::error::NO_GOAL_LEFT)
    );
}

// ADR 4.10.26e AC2: fewer steps than the window shows every rule, oldest
// first, and a round's error ends the call.
#[test]
fn the_last_rules_are_the_most_recent_in_order() {
    let mut positions = vec![3, 1, 2].into_iter();
    let outcome = run_rounds(2, move || Ok::<_, ()>(positions.next()));
    assert_eq!(
        outcome,
        Ok(RoundsOutcome::BoundReached {
            last_rules: vec![3, 1]
        })
    );
    let mut sequence = (1..=6).map(Some);
    let outcome = run_rounds(5, move || Ok::<_, ()>(sequence.next().flatten()));
    assert_eq!(
        outcome,
        Ok(RoundsOutcome::BoundReached {
            last_rules: vec![2, 3, 4, 5]
        })
    );
    assert_eq!(
        run_rounds(SIMP_STEP_BOUND, || Err::<Option<usize>, _>("stop")),
        Err("stop")
    );
}
