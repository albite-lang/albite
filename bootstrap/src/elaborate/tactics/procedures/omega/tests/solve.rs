//! The solver over hand-built Core (ADR 6.10.26f AC 2, D3): each step, both
//! cancellation cases, a negative coefficient, an unused hypothesis and the
//! occurs check — and P0.2's plan for every corpus goal.

use albite_core::Term;

use super::{add, eq_hypothesis, le_goal, le_hypothesis, one, var};
use crate::elaborate::error::OmegaFailure;
use crate::elaborate::tactics::procedures::omega::fragment::{
    goal_shape, hypothesis, le_shape, outside_fragment, Goal,
};
use crate::elaborate::tactics::procedures::omega::polynomial::{numeral, polynomial_term, Signed};
use crate::elaborate::tactics::procedures::omega::solve::{
    apply_sigma, orient, solve, OmegaPlan, Oriented, Source,
};
use crate::elaborate::tactics::procedures::ring::normal_form::{polynomial, AtomOrder};

fn fst(name: &str) -> Term {
    Term::fst(var(name))
}

fn solved(hypotheses: &[super::Hypothesis], goal: &Goal) -> OmegaPlan {
    solve(hypotheses, goal).unwrap_or_else(|failure| panic!("{}", failure.message()))
}

fn failed(hypotheses: &[super::Hypothesis], goal: &Goal) -> OmegaFailure {
    match solve(hypotheses, goal) {
        Ok(plan) => panic!("expected a failure, got {plan:?}"),
        Err(failure) => failure,
    }
}

fn vars(sigma: &[Oriented]) -> Vec<Term> {
    sigma.iter().map(|entry| entry.var.clone()).collect()
}

// ADR 6.10.26f AC2 (D3 steps 1–3): `le_trans` — σ = { b ↦ a + k1 ; c ↦ b + k2 },
// and the witness is `k1 + k2`.
#[test]
fn le_trans_orients_both_hypotheses_and_finds_the_sum() {
    let (a, b, c) = (var("a"), var("b"), var("c"));
    let hypotheses = [
        le_hypothesis("h1", a.clone(), b.clone()),
        le_hypothesis("h2", b.clone(), c.clone()),
    ];
    let plan = solved(&hypotheses, &le_goal(a.clone(), c.clone()));
    assert_eq!(vars(&plan.sigma), [b.clone(), c.clone()]);
    assert_eq!(plan.sigma[0].value, add(a, fst("h1")));
    assert!(!plan.sigma[0].var_on_left);
    assert_eq!(plan.sigma[1].source, Source::Hypothesis(1));
    assert_eq!(plan.witness, Some(add(fst("h1"), fst("h2"))));
    assert!(plan.cancellations.is_empty() && plan.unused.is_empty());
}

// ADR 6.10.26f AC2 (D3 step 3): no hypothesis, a zero witness.
#[test]
fn le_refl_needs_no_hypothesis() {
    let n = var("n");
    let plan = solved(&[], &le_goal(n.clone(), n));
    assert_eq!(plan.witness, Some(Term::Zero));
}

// ADR 6.10.26f AC2 (D3 step 2): an `Eq` hypothesis orients too; the witness is
// a numeral spelled as the elaborator spells one.
#[test]
fn an_eq_hypothesis_orients_its_bare_side() {
    let (a, b) = (var("a"), var("b"));
    let two = numeral(2);
    let hypotheses = [eq_hypothesis("h", add(a.clone(), two.clone()), b.clone())];
    let plan = solved(&hypotheses, &le_goal(a, b));
    assert_eq!(plan.witness, Some(two));
    assert_eq!(numeral(2), Term::succ(Term::succ(Term::Zero)));
}

// ADR 6.10.26f AC2: `orientation_never_records_a_cycle` — `le_antisymm`'s
// second equation would make `n ↦ m + k2` with `m ↦ n + k1` already in σ, so
// it stays a residual.
#[test]
fn orientation_never_records_a_cycle() {
    let (n, m) = (var("n"), var("m"));
    let first = le_hypothesis("h1", n.clone(), m.clone());
    let second = le_hypothesis("h2", m.clone(), n.clone());
    let (var_m, value, on_left) = orient(&first.left, &first.right, &[]).expect("m is bare");
    assert_eq!((var_m.clone(), on_left), (m.clone(), false));
    let sigma = [Oriented {
        var: var_m,
        value,
        var_on_left: false,
        source: Source::Hypothesis(0),
    }];
    assert_eq!(orient(&second.left, &second.right, &sigma), None);
    // Without the first entry, the second orients.
    assert!(orient(&second.left, &second.right, &[]).is_some());
}

// ADR 6.10.26f AC2 (D3 step 4, `B = 0`): `le_antisymm` cancels `n` from
// `n + k1 + k2 == n`, sends both witness atoms to `0`, and closes `n == m`.
#[test]
fn le_antisymm_cancels_to_zero_atoms() {
    let (n, m) = (var("n"), var("m"));
    let hypotheses = [
        le_hypothesis("h1", n.clone(), m.clone()),
        le_hypothesis("h2", m.clone(), n.clone()),
    ];
    let plan = solved(
        &hypotheses,
        &Goal::Eq {
            left: n.clone(),
            right: m.clone(),
        },
    );
    assert_eq!(plan.witness, None);
    assert_eq!(plan.cancellations.len(), 1);
    let cancellation = &plan.cancellations[0];
    assert_eq!(cancellation.residual, 1);
    assert_eq!(cancellation.sigma_len, 1);
    assert_eq!(cancellation.common, n);
    assert_eq!(cancellation.left_rest, add(fst("h1"), fst("h2")));
    assert_eq!(cancellation.right_rest, Term::Zero);
    let atoms: Vec<(Term, Option<Term>)> = cancellation
        .zero_atoms
        .iter()
        .map(|zero| (zero.atom.clone(), zero.rest.clone()))
        .collect();
    assert_eq!(
        atoms,
        [(fst("h1"), Some(fst("h2"))), (fst("h2"), Some(fst("h1")))]
    );
    assert_eq!(vars(&plan.sigma), [m, fst("h1"), fst("h2")]);
    assert_eq!(
        plan.sigma[2].source,
        Source::ZeroAtom {
            cancellation: 0,
            index: 1
        }
    );
    assert!(plan.unused.is_empty());
}

// ADR 6.10.26f AC2 (D3 step 4, re-oriented): `Le<a + 1, b + 1> ⊢ Le<a, b>`
// cancels `1`, then orients `b ↦ a + k`.
#[test]
fn a_common_summand_cancels_and_re_orients() {
    let (a, b) = (var("a"), var("b"));
    let hypotheses = [le_hypothesis(
        "h",
        add(a.clone(), one()),
        add(b.clone(), one()),
    )];
    let plan = solved(&hypotheses, &le_goal(a.clone(), b.clone()));
    let cancellation = &plan.cancellations[0];
    assert_eq!(cancellation.common, one());
    assert_eq!(cancellation.left_rest, add(a, fst("h")));
    assert_eq!(cancellation.right_rest, b.clone());
    assert!(cancellation.zero_atoms.is_empty());
    assert_eq!(vars(&plan.sigma), [b]);
    assert_eq!(plan.sigma[0].source, Source::Cancellation(0));
    assert_eq!(plan.witness, Some(fst("h")));
}

// ADR 6.10.26f AC2 (D8): Example 3's two bounds, stated over `Lt`'s
// expansion `Le<a + 1, b>`.
#[test]
fn example_three_bounds_are_solved() {
    let (k, p) = (var("k"), var("p"));
    let lt = |name: &str, a: Term, b: Term| le_hypothesis(name, add(a, one()), b);
    let plan = solved(
        &[lt("h", k.clone(), p.clone())],
        &le_goal(k.clone(), p.clone()),
    );
    assert_eq!(plan.witness, Some(add(fst("h"), one())));
    let hypotheses = [lt("h1", Term::Zero, k.clone()), lt("h2", k, p.clone())];
    let plan = solved(&hypotheses, &le_goal(add(one(), one()), p));
    assert_eq!(plan.witness, Some(add(fst("h1"), fst("h2"))));
}

// ADR 6.10.26f AC3 (D7, second message): a false goal has a negative term.
#[test]
fn a_negative_coefficient_is_no_witness() {
    let n = var("n");
    let failure = failed(&[], &le_goal(add(n.clone(), one()), n.clone()));
    assert_eq!(
        failure,
        OmegaFailure::NoWitness {
            difference_of: "t - s".into(),
            difference: "-1".into(),
            negative: "-1".into(),
        }
    );
    let failure = failed(
        &[],
        &le_goal(n.clone(), Term::nat_mul(n.clone(), n.clone())),
    );
    assert_eq!(
        failure.message(),
        "`omega`: no witness: `t - s` is `-n + n * n`, with a negative term `-n`"
    );
}

// ADR 6.10.26f AC3 (D7): an `Eq` goal whose `t - s` is already negative is
// reported as it stands, not turned round.
#[test]
fn an_eq_goal_with_a_negative_difference_keeps_t_minus_s() {
    let n = var("n");
    let failure = failed(
        &[],
        &Goal::Eq {
            left: add(n.clone(), one()),
            right: n,
        },
    );
    assert_eq!(
        failure,
        OmegaFailure::NoWitness {
            difference_of: "t - s".into(),
            difference: "-1".into(),
            negative: "-1".into(),
        }
    );
}

// ADR 6.10.26f AC2 (D3 step 4): a rest `A` that carries a constant is not a
// sum of witness atoms, so `n + k1 + k2 + 1 == n` sends nothing to `0` and
// the hypothesis is reported unused.
#[test]
fn a_rest_with_a_constant_sends_no_atom_to_zero() {
    let (n, m) = (var("n"), var("m"));
    let hypotheses = [
        le_hypothesis("h1", n.clone(), m.clone()),
        le_hypothesis("h2", add(m.clone(), one()), n.clone()),
    ];
    let failure = failed(&hypotheses, &Goal::Eq { left: n, right: m });
    let OmegaFailure::Unoriented { unused } = &failure else {
        panic!("expected unused, got {failure:?}");
    };
    assert_eq!(unused[0].0, "h2");
}

// ADR 6.10.26f AC3 (D7): an `Eq` goal whose `t - s` is positive is reported
// the other way round, so the term named is negative.
#[test]
fn an_unequal_eq_goal_names_the_negative_side() {
    let n = var("n");
    let failure = failed(
        &[],
        &Goal::Eq {
            left: n.clone(),
            right: add(n, one()),
        },
    );
    let OmegaFailure::NoWitness {
        difference_of,
        difference,
        negative,
    } = failure
    else {
        panic!("expected no witness, got {failure:?}");
    };
    assert_eq!(
        (
            difference_of.as_str(),
            difference.as_str(),
            negative.as_str()
        ),
        ("s - t", "-1", "-1")
    );
}

// ADR 6.10.26f AC2/AC3 (D7, third message): an unoriented hypothesis with no
// common summand is unused, and named with its equation.
#[test]
fn an_unoriented_hypothesis_is_reported_unused() {
    let (a, b, c, d) = (var("a"), var("b"), var("c"), var("d"));
    let hypotheses = [le_hypothesis(
        "h",
        add(a.clone(), b.clone()),
        add(c.clone(), d.clone()),
    )];
    let failure = failed(&hypotheses, &le_goal(add(b, a), add(d, c)));
    let OmegaFailure::Unoriented { unused } = &failure else {
        panic!("expected unused, got {failure:?}");
    };
    assert_eq!(unused.len(), 1);
    assert_eq!(unused[0].0, "h");
    assert!(failure
        .message()
        .starts_with("`omega`: no witness over the oriented hypotheses; unused: `h` (`"));
}

// ADR 6.10.26f AC2: a solved goal still reports the candidates it never used.
#[test]
fn a_solved_goal_lists_its_unused_candidates() {
    let (a, b, c, d) = (var("a"), var("b"), var("c"), var("d"));
    let hypotheses = [le_hypothesis(
        "h",
        add(a.clone(), b.clone()),
        add(c.clone(), d),
    )];
    let plan = solved(&hypotheses, &le_goal(a.clone(), add(a, c)));
    assert_eq!(plan.unused, [0]);
    assert_eq!(plan.witness, Some(var("c")));
}

// ADR 6.10.26f AC2 (D3 step 2): σ is applied to its fixed point along a chain.
#[test]
fn sigma_applies_along_a_chain() {
    let (x, y, z) = (var("x"), var("y"), var("z"));
    let entry = |var: Term, value: Term| Oriented {
        var,
        value,
        var_on_left: true,
        source: Source::Hypothesis(0),
    };
    let sigma = [entry(x.clone(), add(y.clone(), one())), entry(y, z.clone())];
    assert_eq!(
        apply_sigma(&add(x.clone(), x), &sigma),
        add(add(z.clone(), one()), add(z, one()))
    );
}

// ADR 6.10.26f AC2 (D3 step 3): a witness is spelled as copies up to `ring`'s
// render bound, then as a product with a numeral.
#[test]
fn a_large_coefficient_is_spelled_as_a_product() {
    let n = var("n");
    let mut atoms = AtomOrder::default();
    atoms.extend_from(&n);
    let three_n = add(n.clone(), add(n.clone(), n.clone()));
    let p = polynomial(&three_n, &atoms).expect("indexed");
    assert_eq!(polynomial_term(&p, &atoms), three_n);
    let mut p = p;
    p.monomials.insert(vec![0], 17);
    p.constant = 2;
    assert_eq!(
        polynomial_term(&p, &atoms),
        add(Term::nat_mul(n, numeral(17)), numeral(2))
    );
    let signed = Signed::difference(&p, &Default::default());
    assert_eq!(signed.render(&atoms), "n * 17 + 2");
    assert_eq!(signed.first_negative(&atoms), None);
    // At the bound itself, still copies.
    p.monomials.insert(vec![0], 16);
    p.constant = 0;
    let copies = polynomial_term(&p, &atoms);
    assert!(!matches!(copies, Term::NatMul(..)), "{copies:?}");
    assert_eq!(polynomial(&copies, &atoms).expect("indexed"), p);
}

// ADR 6.10.26f AC2 (D2): the shapes, read after expansion.
#[test]
fn the_fragment_reads_le_and_eq_shapes() {
    let (a, b) = (var("a"), var("b"));
    let le = super::le_type(a.clone(), b.clone());
    assert_eq!(le_shape(&le), Some(("k", &a, &b)));
    assert_eq!(
        goal_shape(&le),
        Some(Goal::Le {
            left: a.clone(),
            right: b.clone()
        })
    );
    // The bound variable may not occur in `s` or `t`.
    let dependent = super::le_type(var("k"), b.clone());
    assert_eq!(le_shape(&dependent), None);
    let eq = albite_core::Type::eq(albite_core::Type::Nat, a.clone(), b.clone());
    assert_eq!(
        goal_shape(&eq),
        Some(Goal::Eq {
            left: a.clone(),
            right: b.clone()
        })
    );
    let over_bool = albite_core::Type::eq(albite_core::Type::Bool, a.clone(), b.clone());
    assert_eq!(goal_shape(&over_bool), None);
    assert_eq!(hypothesis("h", &over_bool), None);
    let read = hypothesis("h", &le).expect("a Le local is a candidate");
    assert_eq!(read.witness, Some(fst("h")));
    assert_eq!(read.left, add(a, fst("h")));
}

// ADR 6.10.26f AC3 (D7, first message, over hand-built Core): a `pow` atom is
// outside the fragment; as a hypothesis it is ignored, never refused.
#[test]
fn a_pow_atom_is_outside_the_fragment() {
    let n = var("n");
    let pow = Term::app(Term::app(Term::global("pow"), n.clone()), numeral(2));
    let goal = le_goal(n.clone(), pow.clone());
    assert_eq!(outside_fragment(&goal), Some(pow.clone()));
    assert_eq!(outside_fragment(&le_goal(n.clone(), n.clone())), None);
    let ty = albite_core::Type::eq(albite_core::Type::Nat, n, pow);
    assert_eq!(hypothesis("h", &ty), None);
    let failure = OmegaFailure::OutsideFragment {
        term: "pow(n, 2)".into(),
    };
    assert_eq!(
        failure.message(),
        "`omega`: the goal is outside its fragment: `ring` cannot read `pow(n, 2)`"
    );
}
