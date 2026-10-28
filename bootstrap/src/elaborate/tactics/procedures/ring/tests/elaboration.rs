//! `ring` through the elaborator (ADR 4.10.26g AC 2): its refusals and their
//! codes, the corpus through the kernel gate, and a hand-broken step refused
//! by the kernel and named with its lemma.
//!
//! The refusals all fire before any citation, so a single module with no
//! `mod nat_arith;` reaches them — except the one about the library itself.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use albite_core::{Context, SpannedTerm, Term, TermSpan, Type};

use super::{add, var};
use crate::elaborate::error::{ElabErrorKind, TacticErrorKind};
use crate::elaborate::kernel::{
    discharge_local, KernelObligation, ObligationOrigin, RecordPresentation,
};
use crate::elaborate::tactics::procedures::ring::plan::{Lemma, PlannedStep};
use crate::elaborate::tactics::procedures::ring::{
    equation_for, ring_needs_the_library, RING_NEEDS_NAT,
};
use crate::elaborate::tactics::state::{Partial, ProofState};
use crate::elaborate::tactics::tests::{eq_nat, tactic_error};
use crate::elaborate::tactics::TacticTrace;
use crate::elaborate::{CoreDef, Elaborator};
use crate::span::Span;

/// E0082's `why` for `source`, asserting the tactic is `ring`.
fn ring_not_applicable(source: &str) -> String {
    let kind = tactic_error(source);
    assert_eq!(kind.code(), "E0082", "{kind:?}");
    let TacticErrorKind::TacticNotApplicable { tactic, why, .. } = kind else {
        panic!("expected E0082, got {kind:?}");
    };
    assert_eq!(tactic, "ring");
    why
}

/// E0087's two rendered sides for `source`.
fn unequal_sides(source: &str) -> (String, String, String) {
    let kind = tactic_error(source);
    assert_eq!(kind.code(), "E0087", "{kind:?}");
    let message = kind.default_message();
    let TacticErrorKind::RingUnequal { left, right } = kind else {
        panic!("expected E0087, got {kind:?}");
    };
    (left, right, message)
}

// ADR 4.10.26g AC2 (D6): a false equation is `ring`'s own code, and the
// message prints both normal forms.
#[test]
fn a_false_equation_prints_both_normal_forms() {
    let (left, right, message) =
        unequal_sides("theorem t(n: Nat, m: Nat) -> Eq<Nat, n + m, n * m> by { ring }");
    assert_eq!((left.as_str(), right.as_str()), ("n + m", "n * m"));
    assert_eq!(
        message,
        "`ring`: the two sides normalise to different polynomials: `n + m` and `n * m`"
    );
    // An atom that hides falsity: `(n - 1) + 1 = n` fails at 0.
    let (left, right, _) =
        unequal_sides("theorem t(n: Nat) -> Eq<Nat, (n - 1) + 1, n> by { ring }");
    assert_eq!((left.as_str(), right.as_str()), ("(n - 1) + 1", "n"));
    // Normalised, not as written: both sides are spelled in D2's order.
    let (left, right, _) =
        unequal_sides("theorem t(n: Nat, m: Nat) -> Eq<Nat, m * 2 + n, n + m + 1> by { ring }");
    assert_eq!((left.as_str(), right.as_str()), ("m + m + n", "m + n + 1"));
}

// ADR 4.10.26g AC2 (D1, Q8)
#[test]
fn a_bool_goal_is_e0082() {
    let why = ring_not_applicable("theorem t(a: Bool) -> Eq<Bool, a, a> by { ring }");
    assert_eq!(why, RING_NEEDS_NAT);
}

// ADR 4.10.26g AC2 (D1)
#[test]
fn a_non_equation_is_e0082() {
    let why = ring_not_applicable("theorem t(n: Nat) -> Nat by { ring }");
    assert_eq!(why, "the goal is not an equation");
}

// ADR 4.10.26g AC2 (D4): a goal that needs a step, with no `mod nat_arith;`.
#[test]
fn ring_without_the_library_is_e0082() {
    let why = ring_not_applicable("theorem t(n: Nat, m: Nat) -> Eq<Nat, n + m, m + n> by { ring }");
    assert_eq!(why, ring_needs_the_library("nat_arith"));
    assert!(why.contains("mod nat_arith;"));
}

// ADR 4.10.26g AC2: a goal after the last one closed has nothing to act on.
#[test]
fn ring_after_the_goal_closed_is_e0082() {
    let why = ring_not_applicable("theorem t(n: Nat) -> Eq<Nat, n, n> by { refl; ring }");
    assert_eq!(why, crate::elaborate::error::NO_GOAL_LEFT);
}

/// `Π n m. Eq Nat (n + m) (m + n)`, `nat_arith::add_comm`'s type.
fn add_comm_type() -> Type {
    let (n, m) = (var("n"), var("m"));
    Type::pi(
        "n",
        Type::Nat,
        Type::pi("m", Type::Nat, eq_nat(add(n.clone(), m.clone()), add(m, n))),
    )
}

// ADR 4.10.26g AC2 (D4): a hand-broken plan step — `add_comm` cited at
// swapped arguments for the sides `n + m` to `m + n` — passes S2's back end,
// which trusts the planner's spelling, and is refused by the kernel: E0081,
// attributed to the `ring` step and naming the lemma and its arguments.
#[test]
fn a_kernel_refusal_names_ring_and_the_lemma() {
    let (n, m) = (var("n"), var("m"));
    let goal = eq_nat(add(n.clone(), m.clone()), add(m.clone(), n.clone()));
    let broken = PlannedStep {
        lemma: Lemma::AddComm,
        args: vec![m.clone(), n.clone()],
        left: add(n.clone(), m.clone()),
        right: add(m.clone(), n.clone()),
    };
    let lemma = "nat_arith::add_comm";
    let equation = equation_for(&broken, Term::global(lemma), &add_comm_type())
        .expect("the type is a Π per argument");
    let mut state = ProofState::new(goal.clone());
    let mut steps = Vec::new();
    let span = Span::new(10, 14);
    let mut ctx = Context::new();
    let mut elaborator = Elaborator::new(&mut ctx);
    elaborator
        .apply_ring_step(&mut state, &broken, equation, span, &mut steps)
        .expect("S2's back end takes the step");
    let closing = add(m.clone(), n.clone());
    let goal_left = state.focus().expect("the rewritten goal");
    state.refine(goal_left, Partial::closed(Term::refl(Type::Nat, closing)));
    let trace = TacticTrace {
        state,
        steps,
        value_params: vec![("n".into(), Type::Nat), ("m".into(), Type::Nat)],
        type_params: Vec::new(),
        block_span: Span::new(0, 20),
    };
    let term = trace.assemble(&BTreeMap::new()).expect("a finished trace");
    let def_span = Span::new(0, 30);
    let def = |name: &str, ty: Type, term: Term| CoreDef {
        name: name.into(),
        ty,
        term: SpannedTerm::new(term, TermSpan::new(def_span.start, def_span.end)),
        span: def_span,
    };
    let lemma_def = def(lemma, add_comm_type(), Term::Sorry);
    let broken_def = def(
        "broken",
        Type::pi("n", Type::Nat, Type::pi("m", Type::Nat, goal)),
        term,
    );
    let obligation = KernelObligation {
        def: "broken".into(),
        span: def_span,
        module: None,
        origin: ObligationOrigin::Tactics(trace),
    };
    let errors = discharge_local(
        &[lemma_def, broken_def],
        &HashMap::new(),
        &RecordPresentation::EMPTY,
        vec![obligation],
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind.code(), "E0081");
    let ElabErrorKind::KernelRefusedProof { tactic, .. } = &errors[0].kind else {
        panic!("expected E0081, got {:?}", errors[0].kind);
    };
    assert_eq!(
        tactic.as_ref().map(|(name, at)| (name.as_str(), *at)),
        Some(("ring (nat_arith::add_comm(m, n))", span))
    );
}

/// `main` beside copies of `nat_arith.tg` and `nat_pow.tg`, elaborated cold:
/// the defined names, or the refusal as its debug text.
fn with_pow_library(main: &str) -> Result<Vec<String>, String> {
    let dir = tempfile::TempDir::new().expect("a temp dir");
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/compiler");
    for file in ["nat_arith.tg", "nat_pow.tg"] {
        std::fs::copy(library.join(file), dir.path().join(file)).expect(file);
    }
    std::fs::write(dir.path().join("main.tg"), main).expect("main.tg");
    crate::driver::elaborate_project_cold(&dir.path().join("main.tg"), 20)
        .map(|project| project.defs.iter().map(|def| def.name.clone()).collect())
        .map_err(|error| format!("{error:?}"))
}

// ADR 9.10.26j AC2 (D4, D6): a numeral power is unfolded and the goal proved,
// through the kernel gate.
#[test]
fn a_numeral_power_is_proved_end_to_end() {
    let names = with_pow_library(
        "mod nat_arith;\nmod nat_pow;\n\
         pub theorem square(x: Nat) -> Eq<Nat, nat_pow::pow(x, 2), x * x> by { ring }\n\
         pub theorem open_power(x: Nat, n: Nat) \
             -> Eq<Nat, nat_pow::pow(x, n) + x, x + nat_pow::pow(x, n)> by { ring }\n",
    )
    .unwrap_or_else(|refusal| panic!("refused: {refusal}"));
    for theorem in ["square", "open_power"] {
        assert!(
            names.iter().any(|name| name.ends_with(theorem)),
            "{theorem}"
        );
    }
}

// ADR 9.10.26j AC2 (D4): each step cites its own library, and the refusal
// for one that does not resolve names it. With `mod nat_pow;` alone,
// `pow(x, 0) == 1` is `nat_pow`'s; `pow(x, 1) == x` ends with `nat_arith`'s
// `mul_one_left`, which resolves through `nat_pow`'s own import.
#[test]
fn each_step_names_its_own_library() {
    for goal in ["nat_pow::pow(x, 0), 1", "nat_pow::pow(x, 1), x"] {
        with_pow_library(&format!(
            "mod nat_pow;\npub theorem t(x: Nat) -> Eq<Nat, {goal}> by {{ ring }}\n"
        ))
        .unwrap_or_else(|refusal| panic!("{goal} refused: {refusal}"));
    }
    assert_eq!(Lemma::PowSucc.library(), "nat_pow");
    assert_eq!(Lemma::PowZero.library(), "nat_pow");
    assert_eq!(Lemma::MulOneLeft.library(), "nat_arith");
    assert_eq!(
        ring_needs_the_library("nat_pow"),
        "`ring` cites the `nat_pow` library; add `mod nat_pow;`"
    );
}

// ADR 9.10.26j AC2 (D4, D6): an exponent past the cap is E0082 naming it.
#[test]
fn an_exponent_past_the_cap_is_e0082() {
    let refusal = with_pow_library(
        "mod nat_arith;\nmod nat_pow;\n\
         pub theorem t(x: Nat) -> Eq<Nat, nat_pow::pow(x, 5), x * x * x * x * x> by { ring }\n",
    )
    .expect_err("past the cap");
    assert!(refusal.contains("E0082"), "{refusal}");
    assert!(
        refusal.contains("`ring` unfolds `pow` up to exponent 4, and `pow(x, 5)` has exponent 5"),
        "{refusal}"
    );
}

// ADR 9.10.26j AC2 (D4): the step cap reached after a `pow` was unfolded is
// E0082 naming the exponent; with nothing unfolded, or on a revisit, it stays
// the planner's own E9998.
#[test]
fn a_step_cap_after_unfolding_is_e0082() {
    use crate::ast::Tactic;
    use crate::elaborate::tactics::procedures::ring::plan::PlanFault;
    use crate::elaborate::tactics::procedures::ring::plan_fault_error;
    let tactic = Tactic::Ring(Span::new(3, 7));
    let goal = eq_nat(var("x"), var("x"));
    let cap = PlanFault::CapReached {
        cap: 512,
        last: vec!["nat_arith::add_comm(x, y)".into()],
    };
    let error = plan_fault_error(&tactic, &goal, &cap, Some(4));
    let ElabErrorKind::Tactic(TacticErrorKind::TacticNotApplicable { tactic, why, .. }) =
        error.kind
    else {
        panic!("expected E0082, got {:?}", error.kind);
    };
    assert_eq!(tactic, "ring");
    assert_eq!(
        why,
        "`ring` reached its cap of 512 steps expanding `pow` at exponent 4; cite `pow_succ` by \
         hand"
    );
    let untouched = plan_fault_error(&Tactic::Ring(Span::new(3, 7)), &goal, &cap, None);
    assert_eq!(untouched.kind.code(), "E9998", "{:?}", untouched.kind);
    let revisit = PlanFault::Revisited { last: Vec::new() };
    let revisited = plan_fault_error(&Tactic::Ring(Span::new(3, 7)), &goal, &revisit, Some(4));
    assert_eq!(revisited.kind.code(), "E9998", "{:?}", revisited.kind);
}

// ADR 9.10.26j AC2 (D4): with no `mod nat_pow;` a `pow` is an atom like any
// other, whatever its name — so a file's own `pow` is never unfolded.
#[test]
fn a_pow_without_the_library_is_an_atom() {
    with_pow_library(
        "mod nat_arith;\npub fn pow(a: Nat, n: Nat) -> Nat { a }\n\
         pub theorem t(x: Nat) -> Eq<Nat, pow(x, 2) + x, x + pow(x, 2)> by { ring }\n",
    )
    .unwrap_or_else(|refusal| panic!("refused: {refusal}"));
    let refusal = with_pow_library(
        "mod nat_arith;\npub fn pow(a: Nat, n: Nat) -> Nat { a }\n\
         pub theorem t(x: Nat) -> Eq<Nat, pow(x, 2), x * x> by { ring }\n",
    )
    .expect_err("an atom against a product");
    assert!(refusal.contains("E0087"), "{refusal}");
}

// ADR 4.10.26g AC4: the suite builds cold through the module-tree walk,
// whose admission gate discharges every tactic proof in the kernel.
#[test]
fn the_ring_corpus_passes_the_kernel_gate() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/compiler/test_tactic_ring.tg");
    let project = crate::driver::elaborate_project_cold(&corpus, 20)
        .unwrap_or_else(|errors| panic!("the corpus is refused: {errors:?}"));
    for theorem in [
        "ring_add_comm",
        "ring_add_assoc",
        "ring_mul_comm",
        "ring_mul_assoc",
        "ring_mul_add",
        "ring_add_mul",
        "ring_mul_one_left",
        "ring_add_left_comm",
        "ring_mul_left_comm",
        "ring_coefficient_right",
        "ring_coefficient_left",
        "ring_square",
        "ring_with_an_atom",
    ] {
        assert!(
            project.defs.iter().any(|def| def.name.ends_with(theorem)),
            "{theorem}"
        );
    }
}
