//! ADR 4.10.26g AC 2 — the plan (D3): it reaches the normal form, cites
//! every argument, never repeats a goal or plans a trivial step, re-reads an
//! atom a step rewrote inside, and clears zeros and units before the sort.

use albite_core::Term;

use super::{add, apply, mul, num, var};
use crate::elaborate::tactics::procedures::ring::plan::{
    plan, Lemma, Plan, PlanFault, PlannedStep, RING_STEP_CAP,
};
use crate::elaborate::tactics::rewrite::abstraction::abstract_occurrences;

fn planned(left: &Term, right: &Term) -> Plan {
    plan(left, right, RING_STEP_CAP).expect("the plan finishes")
}

/// The corpus's goals (D7), as Core: each holds in every commutative
/// semiring, so each plan must reach one spelling on both sides.
fn corpus() -> Vec<(Term, Term)> {
    let (n, m, k) = (var("n"), var("m"), var("k"));
    let sub_one = Term::nat_sub(n.clone(), num(1));
    vec![
        (add(n.clone(), m.clone()), add(m.clone(), n.clone())),
        (
            add(add(n.clone(), m.clone()), k.clone()),
            add(n.clone(), add(m.clone(), k.clone())),
        ),
        (mul(n.clone(), m.clone()), mul(m.clone(), n.clone())),
        (
            mul(mul(n.clone(), m.clone()), k.clone()),
            mul(n.clone(), mul(m.clone(), k.clone())),
        ),
        (
            mul(n.clone(), add(m.clone(), k.clone())),
            add(mul(n.clone(), m.clone()), mul(n.clone(), k.clone())),
        ),
        (
            mul(add(n.clone(), m.clone()), k.clone()),
            add(mul(n.clone(), k.clone()), mul(m.clone(), k.clone())),
        ),
        (mul(num(1), n.clone()), n.clone()),
        (
            add(n.clone(), add(m.clone(), k.clone())),
            add(m.clone(), add(n.clone(), k.clone())),
        ),
        (
            mul(n.clone(), mul(m.clone(), k.clone())),
            mul(m.clone(), mul(n.clone(), k.clone())),
        ),
        (mul(n.clone(), num(2)), add(n.clone(), n.clone())),
        (mul(num(2), n.clone()), add(n.clone(), n.clone())),
        (
            mul(add(n.clone(), m.clone()), add(n.clone(), m.clone())),
            add(
                mul(n.clone(), n.clone()),
                add(
                    mul(n.clone(), m.clone()),
                    add(mul(n.clone(), m.clone()), mul(m.clone(), m.clone())),
                ),
            ),
        ),
        (add(sub_one.clone(), m.clone()), add(m.clone(), sub_one)),
    ]
}

/// Replay `steps` from `(left, right)` with S2's own abstraction.
fn replay(left: &Term, right: &Term, steps: &[PlannedStep]) -> Vec<(Term, Term)> {
    let mut goals = vec![(left.clone(), right.clone())];
    for step in steps {
        let (l, r) = goals.last().expect("a goal").clone();
        goals.push((
            abstract_occurrences(&l, &step.left, &step.right),
            abstract_occurrences(&r, &step.left, &step.right),
        ));
    }
    goals
}

// ADR 4.10.26g AC2: replaying each plan with S2's own `abstract_occurrences`
// reaches the plan's own endpoints, and they are one term.
#[test]
fn the_plan_reaches_the_normal_form() {
    for (left, right) in corpus() {
        let plan = planned(&left, &right);
        let reached = replay(&left, &right, &plan.steps);
        assert_eq!(
            reached.last(),
            Some(&(plan.left.clone(), plan.right.clone()))
        );
        assert_eq!(plan.left, plan.right, "{left} = {right}");
    }
    // The constant's numerals are left for `refl`'s conversion (D2).
    let (n, m) = (var("n"), var("m"));
    let left = add(add(n.clone(), num(2)), add(m.clone(), num(3)));
    let right = add(num(5), add(m.clone(), n.clone()));
    let plan = planned(&left, &right);
    assert_eq!(
        plan.left,
        add(n.clone(), add(m.clone(), add(num(2), num(3))))
    );
    assert_eq!(plan.right, add(n, add(m, num(5))));
}

/// Each lemma's statement in its library, over its parameters in order.
pub(super) fn statement(lemma: Lemma, args: &[Term]) -> (Term, Term) {
    let a = |i: usize| args[i].clone();
    match lemma {
        Lemma::AddZeroRight => (add(a(0), num(0)), a(0)),
        Lemma::AddZeroLeft => (add(num(0), a(0)), a(0)),
        Lemma::AddAssoc => (add(add(a(0), a(1)), a(2)), add(a(0), add(a(1), a(2)))),
        Lemma::AddComm => (add(a(0), a(1)), add(a(1), a(0))),
        Lemma::AddLeftComm => (add(a(0), add(a(1), a(2))), add(a(1), add(a(0), a(2)))),
        Lemma::MulZeroRight => (mul(a(0), num(0)), num(0)),
        Lemma::MulZeroLeft => (mul(num(0), a(0)), num(0)),
        Lemma::MulOneRight => (mul(a(0), num(1)), a(0)),
        Lemma::MulOneLeft => (mul(num(1), a(0)), a(0)),
        // The goal spells `n * (m + 1)` as one numeral (P0.2).
        Lemma::MulSuccRight => (
            mul(a(0), Term::Succ(Box::new(a(1)))),
            add(mul(a(0), a(1)), a(0)),
        ),
        Lemma::MulComm => (mul(a(0), a(1)), mul(a(1), a(0))),
        Lemma::MulLeftComm => (mul(a(0), mul(a(1), a(2))), mul(a(1), mul(a(0), a(2)))),
        Lemma::MulAdd => (
            mul(a(0), add(a(1), a(2))),
            add(mul(a(0), a(1)), mul(a(0), a(2))),
        ),
        Lemma::AddMul => (
            mul(add(a(0), a(1)), a(2)),
            add(mul(a(0), a(2)), mul(a(1), a(2))),
        ),
        Lemma::MulAssoc => (mul(mul(a(0), a(1)), a(2)), mul(a(0), mul(a(1), a(2)))),
        // `nat_pow.tg`'s two, the exponent `n + 1` spelled as one numeral.
        Lemma::PowZero => (pow_of(a(0), num(0)), num(1)),
        Lemma::PowSucc => (
            pow_of(a(0), Term::Succ(Box::new(a(1)))),
            mul(pow_of(a(0), a(1)), a(0)),
        ),
    }
}

fn pow_of(base: Term, exponent: Term) -> Term {
    Term::app(Term::app(Term::global("pow"), base), exponent)
}

pub(super) fn arity(lemma: Lemma) -> usize {
    match lemma {
        Lemma::AddZeroRight
        | Lemma::AddZeroLeft
        | Lemma::MulZeroRight
        | Lemma::MulZeroLeft
        | Lemma::MulOneRight
        | Lemma::MulOneLeft
        | Lemma::PowZero => 1,
        Lemma::AddComm | Lemma::MulSuccRight | Lemma::MulComm | Lemma::PowSucc => 2,
        Lemma::AddAssoc
        | Lemma::AddLeftComm
        | Lemma::MulLeftComm
        | Lemma::MulAdd
        | Lemma::AddMul
        | Lemma::MulAssoc => 3,
    }
}

// ADR 4.10.26g AC2: every step gives the lemma all its arguments, and its two
// sides are the lemma's statement at them — so S2's matcher chooses nothing.
#[test]
fn every_step_cites_all_its_arguments() {
    let mut cited = Vec::new();
    for (left, right) in corpus() {
        for step in planned(&left, &right).steps {
            assert_eq!(step.args.len(), arity(step.lemma), "{}", step.citation());
            assert_eq!(
                statement(step.lemma, &step.args),
                (step.left.clone(), step.right.clone()),
                "{}",
                step.citation()
            );
            cited.push(step.lemma);
        }
    }
    // The corpus reaches every phase.
    for lemma in [
        Lemma::MulSuccRight,
        Lemma::MulOneRight,
        Lemma::MulComm,
        Lemma::MulAdd,
        Lemma::AddMul,
        Lemma::AddAssoc,
        Lemma::AddComm,
        Lemma::AddLeftComm,
        Lemma::MulLeftComm,
        Lemma::MulOneLeft,
    ] {
        assert!(cited.contains(&lemma), "{}", lemma.name());
    }
    let step = &planned(&mul(var("n"), num(2)), &add(var("n"), var("n"))).steps[0];
    assert_eq!(step.citation(), "nat_arith::mul_succ_right(n, 1)");
}

// ADR 4.10.26g AC2: no plan revisits a goal, and the cap is a fault, named
// with the last steps, never a loop.
#[test]
fn the_plan_never_repeats_a_goal() {
    for (left, right) in corpus() {
        let goals = replay(&left, &right, &planned(&left, &right).steps);
        for (index, goal) in goals.iter().enumerate() {
            assert!(!goals[index + 1..].contains(goal), "{left} = {right}");
        }
    }
    let (left, right) = corpus().swap_remove(11);
    let fault = plan(&left, &right, 1).expect_err("the square takes more than one step");
    let PlanFault::CapReached { cap, last } = &fault else {
        panic!("expected the cap, got {fault:?}");
    };
    assert_eq!((*cap, last.len()), (1, 1));
    assert!(
        fault.describe().contains("cap of 1 steps"),
        "{}",
        fault.describe()
    );
    assert!(fault.describe().contains("nat_arith::mul_add("));
    let revisited = PlanFault::Revisited {
        last: vec!["nat_arith::add_comm(n, m)".into()],
    };
    assert!(revisited
        .describe()
        .contains("already reached; the last steps were nat_arith::add_comm(n, m)"));
    // Exactly the cap fits: the plan's own length.
    let steps = planned(&left, &right).steps.len();
    assert!(plan(&left, &right, steps).is_ok());
    assert!(plan(&left, &right, steps - 1).is_err());
}

// ADR 4.10.26g AC2: a step whose sides are one term — `add_comm(x, x)`,
// `mul_left_comm(x, x, z)` — would be S2's E0085, and is never planned.
#[test]
fn the_plan_never_emits_a_trivial_step() {
    let (x, z, n, m) = (var("x"), var("z"), var("n"), var("m"));
    let goals = [
        (add(x.clone(), x.clone()), add(x.clone(), x.clone())),
        (
            mul(x.clone(), mul(x.clone(), z.clone())),
            mul(x.clone(), mul(z.clone(), x.clone())),
        ),
        (
            add(add(n.clone(), m.clone()), add(n.clone(), m.clone())),
            add(add(m.clone(), n.clone()), add(n.clone(), m.clone())),
        ),
        (
            add(mul(x.clone(), z.clone()), mul(x.clone(), z.clone())),
            add(mul(z.clone(), x.clone()), mul(x.clone(), z.clone())),
        ),
    ];
    for (left, right) in goals {
        let plan = planned(&left, &right);
        assert_eq!(plan.left, plan.right);
        for step in &plan.steps {
            assert_ne!(step.left, step.right, "{}", step.citation());
        }
    }
    // An already-normal goal plans nothing at all.
    assert_eq!(
        planned(&add(x.clone(), x.clone()), &add(x.clone(), x)).steps,
        []
    );
}

// ADR 4.10.26g AC2: a step rewrites inside an atom too (S2's every-occurrence
// rule); the atom it makes is read afresh and sorted like any other.
#[test]
fn an_atom_rewritten_inside_is_re_atomised() {
    let (n, m, k) = (var("n"), var("m"), var("k"));
    let grouped = add(add(n.clone(), m.clone()), k.clone());
    let left = add(apply("f", grouped.clone()), grouped.clone());
    let right = add(grouped.clone(), apply("f", grouped));
    let plan = planned(&left, &right);
    let regrouped = add(n.clone(), add(m.clone(), k.clone()));
    assert_eq!(plan.steps[0].citation(), "nat_arith::add_assoc(n, m, k)");
    // The first step reached inside `f(…)` on both sides.
    let after_first = &replay(&left, &right, &plan.steps[..1])[1];
    assert_eq!(
        after_first.0,
        add(apply("f", regrouped.clone()), regrouped.clone())
    );
    // The new atom `f(n + (m + k))` sorts after `n`, `m` and `k`, which
    // first appeared before it.
    let atom = apply("f", regrouped);
    assert_eq!(plan.left, plan.right);
    assert_eq!(plan.left, add(n, add(m, add(k, atom))));
}

// ADR 4.10.26g AC2: zeros and units go before any sort step — phases 1 and
// 2 finish before phase 5 starts.
#[test]
fn zeros_and_units_are_gone_before_the_sort() {
    let (n, m) = (var("n"), var("m"));
    let left = add(
        add(mul(n.clone(), num(1)), num(0)),
        mul(num(1), add(mul(num(0), n.clone()), m.clone())),
    );
    let right = add(m.clone(), add(n.clone(), mul(m.clone(), num(0))));
    let plan = planned(&left, &right);
    let is_identity = |lemma: Lemma| {
        matches!(
            lemma,
            Lemma::AddZeroRight
                | Lemma::AddZeroLeft
                | Lemma::MulZeroRight
                | Lemma::MulZeroLeft
                | Lemma::MulOneRight
                | Lemma::MulOneLeft
        )
    };
    let is_sort = |lemma: Lemma| {
        matches!(
            lemma,
            Lemma::AddComm | Lemma::AddLeftComm | Lemma::MulComm | Lemma::MulLeftComm
        )
    };
    let lemmas: Vec<Lemma> = plan.steps.iter().map(|step| step.lemma).collect();
    let first_sort = lemmas
        .iter()
        .position(|l| is_sort(*l))
        .expect("a sort step");
    assert!(lemmas[..first_sort].iter().any(|l| is_identity(*l)));
    assert!(
        !lemmas[first_sort..].iter().any(|l| is_identity(*l)),
        "{lemmas:?}"
    );
    assert_eq!(plan.left, plan.right);
    assert_eq!(plan.left, add(n, m));
}
