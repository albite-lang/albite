//! `ring`'s `pow` arm, pure (ADR 9.10.26j AC 2, D4, D6).

use albite_core::{Term, Type};

use super::plan::{arity, statement};
use super::{add, mul, num, var};
use crate::elaborate::tactics::procedures::ring::lemma::Lemma;
use crate::elaborate::tactics::procedures::ring::plan::{plan, PlannedStep, RING_STEP_CAP};
use crate::elaborate::tactics::procedures::ring::power::{
    unfold_powers, unfolded_head, PastCap, POW_UNFOLD_CAP,
};
use crate::elaborate::tactics::tests::eq_nat;

/// `head(base, exponent)`.
fn power(head: &str, base: Term, exponent: Term) -> Term {
    Term::app(Term::app(Term::global(head), base), exponent)
}

fn pow(base: Term, exponent: Term) -> Term {
    power("pow", base, exponent)
}

fn lemmas(steps: &[PlannedStep]) -> Vec<Lemma> {
    steps.iter().map(|step| step.lemma).collect()
}

// ADR 9.10.26j AC2 (D4): an open exponent is one atom, left as written.
#[test]
fn an_open_exponent_is_an_atom() {
    let (x, n) = (var("x"), var("n"));
    let left = add(pow(x.clone(), n.clone()), x.clone());
    let right = add(x.clone(), pow(x, n));
    let unfolded = unfold_powers(&left, &right, "pow", POW_UNFOLD_CAP).expect("under the cap");
    assert!(unfolded.steps.is_empty());
    assert_eq!((unfolded.left, unfolded.right), (left, right));
    assert_eq!(unfolded.largest_exponent, None);
}

// ADR 9.10.26j AC2 (D4): a numeral exponent unfolds by `pow_succ` down to
// `pow_zero`, the base kept as written, every step citing `nat_pow`.
#[test]
fn a_numeral_exponent_unfolds_to_pow_zero() {
    let x = var("x");
    let left = pow(x.clone(), num(2));
    let right = mul(x.clone(), x.clone());
    let unfolded = unfold_powers(&left, &right, "pow", POW_UNFOLD_CAP).expect("under the cap");
    assert_eq!(
        lemmas(&unfolded.steps),
        [Lemma::PowSucc, Lemma::PowSucc, Lemma::PowZero]
    );
    let citations: Vec<String> = unfolded.steps.iter().map(PlannedStep::citation).collect();
    assert_eq!(
        citations,
        [
            "nat_pow::pow_succ(x, 1)",
            "nat_pow::pow_succ(x, 0)",
            "nat_pow::pow_zero(x)"
        ]
    );
    // Each step's sides are the goal's spelling: `pow(x, 2)` to `pow(x, 1) * x`.
    assert_eq!(unfolded.steps[0].left, pow(x.clone(), num(2)));
    assert_eq!(
        unfolded.steps[0].right,
        mul(pow(x.clone(), num(1)), x.clone())
    );
    assert_eq!(unfolded.steps[2].right, num(1));
    assert_eq!(unfolded.left, mul(mul(num(1), x.clone()), x.clone()));
    assert_eq!(unfolded.right, right);
    assert_eq!(unfolded.largest_exponent, Some(2));
    // Every step is its lemma's statement at its arguments.
    for step in &unfolded.steps {
        assert_eq!(step.args.len(), arity(step.lemma), "{}", step.citation());
        assert_eq!(
            statement(step.lemma, &step.args),
            (step.left.clone(), step.right.clone()),
            "{}",
            step.citation()
        );
    }
}

// ADR 9.10.26j AC2 (D4): exponent zero is one `pow_zero` step, to `1`.
#[test]
fn exponent_zero_is_one_step() {
    let x = var("x");
    let unfolded = unfold_powers(&pow(x.clone(), num(0)), &num(1), "pow", POW_UNFOLD_CAP)
        .expect("under the cap");
    assert_eq!(lemmas(&unfolded.steps), [Lemma::PowZero]);
    assert_eq!(unfolded.left, num(1));
    assert_eq!(unfolded.largest_exponent, Some(0));
}

// ADR 9.10.26j AC2 (D4): a `pow` whose head is not the library's is an atom.
#[test]
fn a_foreign_pow_is_an_atom() {
    let x = var("x");
    let left = power("power", x.clone(), num(2));
    let unfolded = unfold_powers(&left, &x, "pow", POW_UNFOLD_CAP).expect("under the cap");
    assert!(unfolded.steps.is_empty());
    assert_eq!(unfolded.left, left);
    // A local named `pow` is not a global head either.
    let local = Term::app(Term::app(var("pow"), x.clone()), num(2));
    let unfolded = unfold_powers(&local, &x, "pow", POW_UNFOLD_CAP).expect("under the cap");
    assert!(unfolded.steps.is_empty());
}

// ADR 9.10.26j AC2 (D4): an exponent past the cap is refused before any
// step, naming the power, its exponent and the cap; the cap itself is not.
#[test]
fn the_cap_refuses_a_larger_exponent() {
    let x = var("x");
    let past = POW_UNFOLD_CAP + 1;
    let left = add(pow(x.clone(), num(2)), pow(x.clone(), num(past)));
    let refusal = unfold_powers(&x, &left, "pow", POW_UNFOLD_CAP).expect_err("past the cap");
    assert_eq!(
        refusal,
        PastCap {
            power: format!("pow(x, {past})"),
            exponent: past,
            cap: POW_UNFOLD_CAP,
        }
    );
    assert_eq!(
        refusal.describe(),
        format!(
            "`ring` unfolds `pow` up to exponent {POW_UNFOLD_CAP}, and `pow(x, {past})` has \
             exponent {past}; cite `pow_succ` by hand"
        )
    );
    let at_cap = pow(x.clone(), num(POW_UNFOLD_CAP));
    let unfolded = unfold_powers(&at_cap, &x, "pow", POW_UNFOLD_CAP).expect("at the cap");
    assert_eq!(unfolded.largest_exponent, Some(POW_UNFOLD_CAP));
    assert_eq!(unfolded.steps.len() as u64, POW_UNFOLD_CAP + 1);
}

// ADR 9.10.26j AC2 (D4): of two powers past the cap at the same exponent,
// the refusal names the first in pre-order.
#[test]
fn the_cap_names_the_first_of_two_equal_exponents() {
    let past = POW_UNFOLD_CAP + 1;
    let left = mul(pow(var("y"), num(past)), pow(var("x"), num(past)));
    let refusal = unfold_powers(&left, &var("x"), "pow", POW_UNFOLD_CAP).expect_err("past the cap");
    assert_eq!(refusal.power, format!("pow(y, {past})"));
}

// ADR 9.10.26j AC2 (D4): a `pow` in another's base unfolds first, and a
// `pow` occurring twice unfolds once, since each step rewrites both.
#[test]
fn a_base_unfolds_before_its_host_and_twins_once() {
    let x = var("x");
    let inner = pow(x.clone(), num(1));
    let host = pow(inner.clone(), num(1));
    let unfolded = unfold_powers(&host, &inner, "pow", POW_UNFOLD_CAP).expect("under the cap");
    assert_eq!(unfolded.steps[0].left, inner);
    // `pow(x, 1)` on both sides: two steps for it, two for its host.
    assert_eq!(unfolded.steps.len(), 4);
    let one_x = mul(num(1), x.clone());
    assert_eq!(unfolded.right, one_x);
    assert_eq!(unfolded.left, mul(num(1), one_x));
}

// ADR 9.10.26j AC2 (D4): a `pow` under a binder is not unfolded — S2's
// rewrite cannot reach it — so the unfolding still terminates.
#[test]
fn a_pow_under_a_binder_is_left() {
    let x = var("x");
    let under = Term::lambda("y", Type::Nat, pow(x.clone(), num(2)));
    let left = Term::app(var("f"), under);
    let unfolded = unfold_powers(&left, &x, "pow", POW_UNFOLD_CAP).expect("under the cap");
    assert!(unfolded.steps.is_empty());
    assert_eq!(unfolded.largest_exponent, None);
}

// ADR 9.10.26j AC2 (D4, P0.3): the head is read from `pow_succ`'s type.
#[test]
fn the_head_is_read_from_pow_succs_type() {
    let (a, n) = (var("a"), var("n"));
    let pow_succ = Type::pi(
        "a",
        Type::Nat,
        Type::pi(
            "n",
            Type::Nat,
            eq_nat(
                pow(a.clone(), add(n.clone(), num(1))),
                mul(pow(a.clone(), n), a),
            ),
        ),
    );
    assert_eq!(unfolded_head(&pow_succ).as_deref(), Some("pow"));
    // One binder short, or no `pow` on the left, is not the lemma's type.
    assert_eq!(unfolded_head(&Type::pi("a", Type::Nat, Type::Nat)), None);
    let not_a_power = Type::pi(
        "a",
        Type::Nat,
        Type::pi("n", Type::Nat, eq_nat(var("a"), var("n"))),
    );
    assert_eq!(unfolded_head(&not_a_power), None);
    assert_eq!(unfolded_head(&Type::Nat), None);
}

/// Steps to prove `pow(base, k) == base * (base * …)`, unfolding and plan.
fn steps_for(base: &Term, k: u64) -> usize {
    let product = (1..k).fold(base.clone(), |acc, _| mul(base.clone(), acc));
    let right = if k == 0 { num(1) } else { product };
    let unfolded = unfold_powers(&pow(base.clone(), num(k)), &right, "pow", k).expect("at k");
    let remaining = RING_STEP_CAP.saturating_sub(unfolded.steps.len());
    plan(&unfolded.left, &unfolded.right, remaining).map_or(usize::MAX, |planned| {
        unfolded.steps.len() + planned.steps.len()
    })
}

// ADR 9.10.26j AC2 (P0.4): the cap is the largest exponent every measured
// base — an atom, a binomial, a numeral — plans within half of the step cap.
#[test]
fn the_cap_is_p0_4s_calibration() {
    let bases = [var("x"), add(var("x"), num(1)), num(2)];
    let half = RING_STEP_CAP / 2;
    for base in &bases {
        for k in 0..=POW_UNFOLD_CAP {
            assert!(steps_for(base, k) <= half, "{base:?} at {k}");
        }
    }
    assert!(
        bases
            .iter()
            .any(|base| steps_for(base, POW_UNFOLD_CAP + 1) > half),
        "the cap could be higher"
    );
    // P0.4's record: the atom is linear, the binomial is not.
    let counts = |base: &Term| -> Vec<usize> { (0..=4).map(|k| steps_for(base, k)).collect() };
    assert_eq!(counts(&bases[0]), [1, 3, 4, 6, 8]);
    assert_eq!(counts(&bases[1]), [1, 3, 11, 45, 129]);
    assert_eq!(counts(&bases[2]), [1, 4, 10, 20, 36]);
    assert_eq!(steps_for(&bases[1], 5), 403);
}
