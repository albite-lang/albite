//! `simp` through the elaborator (ADR 4.10.26e AC 2): D2's rounds and close,
//! D3's refusals, D4's trace and kernel attribution.
//!
//! Lemmas are axioms where a proof would only add noise. A block is run
//! directly against a hand-built target where the trace or the deferral is
//! what is asserted; the kernel's verdict on the corpus is AC 4's.

use std::collections::{BTreeMap, HashMap};

use albite_core::{SpannedTerm, Term, TermSpan, Type};

use crate::ast::{Item, TacticBlock, TheoremBody};
use crate::elaborate::error::{ElabErrorKind, TacticErrorKind};
use crate::elaborate::kernel::{
    discharge_local, KernelObligation, ObligationOrigin, RecordPresentation,
};
use crate::elaborate::tactics::simp::NO_RULE_OCCURS;
use crate::elaborate::tactics::tests::{eq_nat, tactic_error};
use crate::elaborate::tactics::TacticTrace;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::{CoreDef, ElabResult, Elaborator};
use crate::span::Span;

/// `0 + n = n`, a bare lemma `simp` instantiates at each match.
const ADD_ZERO_LEFT: &str = "axiom add_zero_left(n: Nat): Eq<Nat, 0 + n, n>\n";

/// `n + m = m + n`: permutative as a bare lemma.
const ADD_COMM: &str = "axiom add_comm(n: Nat, m: Nat): Eq<Nat, n + m, m + n>\n";

fn var(name: &str) -> Term {
    Term::var(name)
}

fn add(left: Term, right: Term) -> Term {
    Term::NatAdd(Box::new(left), Box::new(right))
}

/// `Π name:Nat. body` for each name, outermost first.
fn over_nats(names: &[&str], body: Type) -> Type {
    names
        .iter()
        .rev()
        .fold(body, |body, name| Type::pi(*name, Type::Nat, body))
}

/// The `by` block of `source`'s first theorem.
fn block_of(source: &str) -> TacticBlock {
    let (file, errors) = crate::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    let Item::Theorem(theorem) = &file.items[0] else {
        panic!("expected a theorem");
    };
    let TheoremBody::Tactics(block) = &theorem.body else {
        panic!("expected a by block");
    };
    block.clone()
}

/// Run `source`'s block against `target` with no parameters in scope, with
/// conversion deferral as the per-module walk sets it.
fn run_block(
    source: &str,
    target: &Type,
    defer: bool,
) -> (ElabResult<(Term, TacticTrace)>, usize, usize) {
    let block = block_of(source);
    let mut ctx = albite_core::Context::new();
    let mut elaborator = Elaborator::new(&mut ctx);
    elaborator.defer_conversion = defer;
    let result = elaborator.elaborate_tactic_block(&block, target);
    let obligations = elaborator.conversion_obligations.len();
    (result, obligations, elaborator.errors.len())
}

/// Each trace step as `(tactic, source text)`.
fn steps_of<'a>(source: &'a str, trace: &TacticTrace) -> Vec<(&'static str, &'a str)> {
    trace
        .steps
        .iter()
        .map(|step| {
            (
                step.tactic,
                &source[step.span.start as usize..step.span.end as usize],
            )
        })
        .collect()
}

/// E0082's tactic and `why` for `source`.
fn not_applicable(source: &str) -> (String, String) {
    match tactic_error(source) {
        kind @ TacticErrorKind::TacticNotApplicable { .. } => {
            assert_eq!(kind.code(), "E0082");
            let TacticErrorKind::TacticNotApplicable { tactic, why, .. } = kind else {
                unreachable!()
            };
            (tactic, why)
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

/// `Π a b c d. Eq a b → Eq c a → Eq a d → Eq (c + c) (b + b)`, for D2's
/// restart: after `k` turns `c` into `a`, `h` must run before `l` would.
fn restart_target() -> Type {
    over_nats(
        &["a", "b", "c", "d"],
        Type::arrow(
            eq_nat(var("a"), var("b")),
            Type::arrow(
                eq_nat(var("c"), var("a")),
                Type::arrow(
                    eq_nat(var("a"), var("d")),
                    eq_nat(add(var("c"), var("c")), add(var("b"), var("b"))),
                ),
            ),
        ),
    )
}

const RESTART: &str = "theorem t() -> Nat by { intro a; intro b; intro c; intro d; \
                       intro h; intro k; intro l; simp [h, k, l] }";

// ADR 4.10.26e AC2 (D2): each round starts again from rule 1. Had the round
// gone on to `l` after `k`, the goal would be `d + d = b + b`, which `refl`
// cannot close.
#[test]
fn rounds_restart_from_the_first_rule() {
    let (result, _, _) = run_block(RESTART, &restart_target(), false);
    let (_, trace) = result.expect("the block closes its goal");
    let simp_steps: Vec<_> = steps_of(RESTART, &trace)
        .into_iter()
        .filter(|(tactic, _)| *tactic == "simp")
        .collect();
    assert_eq!(
        simp_steps,
        [("simp", "k"), ("simp", "h"), ("simp", "simp [h, k, l]")]
    );
    // D4: each rewrite solved its own goal, and the `refl` the last one.
    let goals: Vec<_> = trace.steps.iter().rev().take(3).map(|s| s.goal).collect();
    assert!(goals[0] != goals[1] && goals[1] != goals[2], "{goals:?}");
}

// ADR 4.10.26e AC2 (D2): a rule with no occurrence is skipped, not refused.
#[test]
fn an_absent_rule_is_skipped() {
    let defs = elab_ok(&format!(
        "{ADD_ZERO_LEFT}theorem t(a: Nat, c: Nat, h: Eq<Nat, c, a>) -> Eq<Nat, a + 0, 0 + a> by {{ \
         simp [h, add_zero_left] }}"
    ));
    assert!(defs.iter().any(|def| def.name == "t"));
}

// ADR 4.10.26e AC2 (D2): when no rule occurs, `refl` closes what it can, and
// the close is its own trace step at the `simp`.
#[test]
fn simp_closes_with_refl_when_refl_would() {
    let source = "theorem t() -> Nat by { intro a; intro b; intro h; simp [h] }";
    let target = over_nats(
        &["a", "b"],
        Type::arrow(
            eq_nat(var("a"), var("b")),
            eq_nat(add(var("a"), Term::Zero), var("b")),
        ),
    );
    let (result, obligations, _) = run_block(source, &target, false);
    let (term, trace) = result.expect("`refl` closes `b + 0 = b`");
    assert_eq!(
        &steps_of(source, &trace)[3..],
        [("simp", "h"), ("simp", "simp [h]")]
    );
    assert_eq!(obligations, 0);
    assert!(trace.state.goals.is_empty());
    assert!(!term.contains_sorry());
}

// ADR 4.10.26e AC2 (D2): a goal `refl` refuses stays focused, simplified, for
// the next tactic.
#[test]
fn simp_leaves_an_open_goal_focused() {
    let defs = elab_ok(
        "theorem t(a: Nat, b: Nat, c: Nat, h: Eq<Nat, a, b>, k: Eq<Nat, b, c>) -> Eq<Nat, a, c> by { \
         simp [h]; exact k }",
    );
    assert!(defs.iter().any(|def| def.name == "t"));
    // Alone, the simplified goal is left open: E0083, naming `b = c`.
    match tactic_error(
        "theorem t(a: Nat, b: Nat, c: Nat, h: Eq<Nat, a, b>) -> Eq<Nat, a, c> by { simp [h] }",
    ) {
        TacticErrorKind::UnsolvedGoals { goals } => {
            assert_eq!(goals.len(), 1);
            assert_eq!(goals[0].1, eq_nat(var("b"), var("c")));
        }
        other => panic!("expected E0083, got {other:?}"),
    }
}

// ADR 4.10.26e AC2 (D2): no rule occurring at all is a refusal, never a
// silent `refl`.
#[test]
fn no_progress_is_e0082() {
    let (tactic, why) = not_applicable(&format!(
        "{ADD_ZERO_LEFT}theorem t(a: Nat) -> Eq<Nat, a, a> by {{ simp [add_zero_left] }}"
    ));
    assert_eq!(tactic, "simp");
    assert_eq!(why, NO_RULE_OCCURS);
}

// ADR 4.10.26e AC2 (D3)
#[test]
fn a_permutative_bare_lemma_is_e0082() {
    let (tactic, why) = not_applicable(&format!(
        "{ADD_COMM}theorem t(a: Nat, b: Nat) -> Eq<Nat, a + b, b + a> by {{ simp [add_comm] }}"
    ));
    assert_eq!(tactic, "simp");
    assert_eq!(
        why,
        "`add_comm` swaps its arguments and would rewrite forever; \
         give its arguments, or use `rewrite`"
    );
    // Applied, it is an ordinary rule.
    elab_ok(&format!(
        "{ADD_COMM}theorem t(a: Nat, b: Nat) -> Eq<Nat, a + b, b + a> by {{ simp [add_comm(a, b)] }}"
    ));
}

// ADR 4.10.26e AC2 (D3): two instances that undo each other.
#[test]
fn a_loop_is_e0086() {
    let kind = tactic_error(&format!(
        "{ADD_COMM}theorem t(a: Nat, b: Nat, c: Nat) -> Eq<Nat, a + b, c> by {{ \
         simp [add_comm(a, b), add_comm(b, a)] }}"
    ));
    assert_eq!(kind.code(), "E0086");
    let TacticErrorKind::SimpStepBound {
        bound, last_rules, ..
    } = &kind
    else {
        panic!("expected E0086, got {kind:?}");
    };
    assert_eq!(*bound, 64);
    assert_eq!(last_rules, &[1, 2, 1, 2]);
    let message = kind.default_message();
    assert!(
        message.starts_with("`simp` took 64 rewrite steps"),
        "{message}"
    );
    assert!(message.contains("were 1, 2, 1, 2"), "{message}");
}

// ADR 4.10.26e AC2: S2's E0072, unchanged through `simp`.
#[test]
fn a_non_equation_rule_is_e0072() {
    let errors = elab_err("theorem t(j: Nat) -> Eq<Nat, j, j> by { simp [j] }");
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0072"),
        "{errors:?}"
    );
}

// ADR 4.10.26e AC2 (D8): a refusal `rewrite`'s step raises names `simp`.
#[test]
fn a_refusal_through_simp_names_simp() {
    let (tactic, why) = not_applicable(
        "axiom same<T>(x: T): Eq<T, x, x>\n\
         theorem t(k: Nat) -> Eq<Nat, k, k> by { simp [same] }",
    );
    assert_eq!(tactic, "simp");
    assert_eq!(
        why,
        "rule 1: a generic lemma cannot be rewritten by; pass explicit arguments"
    );
    let (tactic, why) = not_applicable(&format!(
        "{ADD_ZERO_LEFT}theorem t(a: Nat, b: Nat) -> Eq<Nat, a, b> -> Eq<Nat, b, a> by {{ simp [add_zero_left] }}"
    ));
    assert_eq!(
        (tactic.as_str(), why.as_str()),
        ("simp", "rule 1: the goal is not an equation")
    );
    // `rewrite`'s own refusals still name `rewrite`.
    let (tactic, _) = not_applicable(
        "axiom same<T>(x: T): Eq<T, x, x>\n\
         theorem t(k: Nat) -> Eq<Nat, k, k> by { rewrite [same] }",
    );
    assert_eq!(tactic, "rewrite");
}

/// `Π a. Eq a 2 → Eq a <global>`: `simp [h]` leaves `2 = <global>`, which
/// only δ can decide.
fn needs_delta_target() -> Type {
    over_nats(
        &["a"],
        Type::arrow(
            eq_nat(var("a"), Term::nat(2)),
            eq_nat(var("a"), Term::Global("two".into())),
        ),
    )
}

// ADR 4.10.26e AC2 (D2): a `refl` that defers commits — the goal is closed,
// and its obligation is left for the admission boundary.
#[test]
fn a_deferred_refl_closes_the_goal() {
    let source = "theorem t() -> Nat by { intro a; intro h; simp [h] }";
    let (result, obligations, errors) = run_block(source, &needs_delta_target(), true);
    let (_, trace) = result.expect("the deferred `refl` closes the goal");
    assert_eq!(steps_of(source, &trace).last(), Some(&("simp", "simp [h]")));
    assert_eq!((obligations, errors), (1, 0));
    // Without deferral the same `refl` refuses, and the goal stays open.
    let (result, obligations, _) = run_block(source, &needs_delta_target(), false);
    assert!(result.is_err());
    assert_eq!(obligations, 0);
}

// ADR 4.10.26e AC2 (P0.2): a refused closing `refl` queues no obligation and
// records no error; the block's own E0083 is the only report.
#[test]
fn a_refused_refl_leaves_no_obligation() {
    let source = "theorem t() -> Nat by { intro a; intro b; intro h; simp [h] }";
    let target = over_nats(
        &["a", "b"],
        Type::arrow(
            eq_nat(var("a"), var("b")),
            eq_nat(var("a"), add(var("b"), Term::nat(1))),
        ),
    );
    let (result, obligations, errors) = run_block(source, &target, true);
    let error = result.expect_err("`b = b + 1` stays open");
    assert!(
        matches!(
            error.kind,
            ElabErrorKind::Tactic(TacticErrorKind::UnsolvedGoals { .. })
        ),
        "{error:?}"
    );
    assert_eq!((obligations, errors), (0, 0));
}

// ADR 4.10.26e AC2 (D4): a hand-broken `simp` step — the second rewrite's
// equation swapped for the wrong hypothesis — is refused by the kernel and
// attributed to that rule's span, never to the closing `refl`.
#[test]
fn a_kernel_refusal_names_the_rule() {
    let (result, _, _) = run_block(RESTART, &restart_target(), false);
    let (_, mut trace) = result.expect("the block closes its goal");
    let broken = trace
        .steps
        .iter()
        .find(|step| {
            step.tactic == "simp"
                && &RESTART[step.span.start as usize..step.span.end as usize] == "h"
        })
        .expect("the `h` step")
        .clone();
    let solution = trace.state.metas[broken.goal]
        .solution
        .as_mut()
        .expect("a solved goal");
    let Term::Subst(_, _, eq_proof, _) = &mut solution.template else {
        panic!("a rewrite step is a Subst: {:?}", solution.template);
    };
    **eq_proof = var("l");
    let term = trace.assemble(&BTreeMap::new()).expect("a finished trace");
    let span = Span::new(0, RESTART.len() as u32);
    let def = CoreDef {
        name: "broken".into(),
        ty: restart_target(),
        term: SpannedTerm::new(term, TermSpan::new(span.start, span.end)),
        span,
    };
    let obligation = KernelObligation {
        def: "broken".into(),
        span,
        module: None,
        origin: ObligationOrigin::Tactics(trace),
    };
    let errors = discharge_local(
        &[def],
        &HashMap::new(),
        &RecordPresentation::EMPTY,
        vec![obligation],
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    let ElabErrorKind::KernelRefusedProof { tactic, .. } = &errors[0].kind else {
        panic!("expected E0081, got {:?}", errors[0].kind);
    };
    assert_eq!(
        tactic.as_ref().map(|(name, at)| (name.as_str(), *at)),
        Some(("simp", broken.span))
    );
}
