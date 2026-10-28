//! `induction` (ADR 3.10.26e AC 2, AC 3): the pure goal construction, the
//! Core it emits, its trace step, and each refusal's kind and code.
//!
//! `elab_ok` runs the elaborator, not the per-module kernel boundary, so the
//! kernel's verdict on this Core is AC 6's, over
//! `src/compiler/test_tactic_induction.tg`.

use albite_core::{Context, Term, Type};

use super::{eq_nat, tactic_error, zero_plus};
use crate::ast::{Item, TheoremBody};
use crate::elaborate::error::{ElabErrorKind, TacticErrorKind};
use crate::elaborate::tactics::induction::{
    induction_goals, induction_template, local_mentioning, successor_spelling,
};
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::Elaborator;
use crate::span::Span;

/// `n + 1`, as the source elaborates it.
fn plus_one(name: &str) -> Term {
    Term::NatAdd(Box::new(Term::var(name)), Box::new(Term::succ(Term::Zero)))
}

/// `0 + n = n`, `add_zero_left`'s statement as it elaborates.
fn zero_add_goal() -> Type {
    eq_nat(zero_plus(Term::var("n")), Term::var("n"))
}

fn not_applicable_why(source: &str) -> String {
    match tactic_error(source) {
        TacticErrorKind::TacticNotApplicable { tactic, why, .. } => {
            assert_eq!(tactic, "induction");
            why
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

/// Every error code `source` raises, in order.
fn codes_of(source: &str) -> Vec<&'static str> {
    elab_err(source).iter().map(|e| e.kind.code()).collect()
}

// ADR 3.10.26e AC2
#[test]
fn induction_splits_a_goal_into_base_and_step() {
    let goals = induction_goals("n", &zero_add_goal());
    assert_eq!(goals.base, eq_nat(zero_plus(Term::Zero), Term::Zero));
    assert_eq!(
        goals.step,
        Type::pi(
            "n",
            Type::Nat,
            Type::arrow(
                zero_add_goal(),
                eq_nat(zero_plus(plus_one("n")), plus_one("n")),
            ),
        )
    );
}

// ADR 3.10.26e AC2 (D2): `x + 1`, never `succ x`, so `rewrite` meets the
// library's `+ 1` lemmas.
#[test]
fn the_step_spells_the_successor_as_plus_one() {
    assert_eq!(successor_spelling("j"), plus_one("j"));
    assert_ne!(successor_spelling("j"), Term::succ(Term::var("j")));
}

// ADR 3.10.26e AC2 (D2)
#[test]
fn the_motive_binder_is_the_target() {
    let goals = induction_goals("n", &zero_add_goal());
    assert_eq!(goals.motive, Type::arrow(Type::Nat, zero_add_goal()));
    let Term::NatInd(binder, ..) = induction_template("n", &goals, "%goal1", "%goal2") else {
        panic!("expected a natind");
    };
    assert_eq!(binder, "n");
}

// ADR 3.10.26e AC2: base hole, then step hole, then the target itself.
#[test]
fn the_template_is_a_two_hole_natind() {
    let goals = induction_goals("n", &zero_add_goal());
    assert_eq!(
        induction_template("n", &goals, "%goal1", "%goal2"),
        Term::natind(
            "n",
            goals.motive.clone(),
            Term::var("%goal1"),
            Term::var("%goal2"),
            Term::var("n"),
        )
    );
}

// ADR 3.10.26e AC2: a goal that does not mention the target is a
// non-dependent motive — accepted, merely weaker.
#[test]
fn a_goal_without_the_target_still_splits() {
    let constant = eq_nat(Term::var("m"), Term::var("m"));
    let goals = induction_goals("n", &constant);
    assert_eq!(goals.base, constant);
    assert_eq!(
        goals.step,
        Type::pi("n", Type::Nat, Type::arrow(constant.clone(), constant))
    );
}

// ADR 3.10.26e AC3 (D6): the target itself is skipped; the first other local
// whose type mentions it is named.
#[test]
fn local_mentioning_names_the_first_dependent_local() {
    let mentions = eq_nat(Term::var("n"), Term::Zero);
    let unrelated = eq_nat(Term::var("m"), Term::Zero);
    let locals = [
        ("n", &Type::Nat),
        ("u", &unrelated),
        ("h", &mentions),
        ("k", &mentions),
    ];
    assert_eq!(local_mentioning("n", locals), Some("h"));
    assert_eq!(local_mentioning("n", [("n", &mentions)]), None);
    assert_eq!(local_mentioning("n", [("u", &unrelated)]), None);
}

// ADR 3.10.26e AC2: the hole term grafts into the `natind` the kernel types,
// with each case's solution in place.
#[test]
fn an_induction_proof_emits_a_natind() {
    let defs = elab_ok(
        "theorem t(n: Nat) -> Eq<Nat, 0 + n, n> by {
             induction n; refl; intro j; intro ih; exact cong(|x: Nat| x + 1, ih)
         }",
    );
    let Term::Lambda(_, _, body) = defs[0].term.term.strip_spans() else {
        panic!("expected the parameter's λ");
    };
    let Term::NatInd(binder, motive, _, step, scrutinee) = *body else {
        panic!("expected a natind, got {body:?}");
    };
    assert_eq!(binder, "n");
    assert_eq!(motive, Type::arrow(Type::Nat, zero_add_goal()));
    assert_eq!(*scrutinee, Term::var("n"));
    let Term::Lambda(index, Type::Nat, inner) = *step else {
        panic!("expected the step's λj");
    };
    assert_eq!(index, "j");
    assert!(matches!(*inner, Term::Lambda(ref ih, _, _) if ih == "ih"));
}

// ADR 3.10.26e AC2 (D5): one trace step for `induction`, spanning it.
#[test]
fn induction_records_one_trace_step() {
    let source = "theorem t() -> Nat by { intro n; induction n; refl; intro j; intro ih; \
                  exact cong(|x: Nat| x + 1, ih) }";
    let (file, errors) = crate::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    let Item::Theorem(theorem) = &file.items[0] else {
        panic!("expected a theorem");
    };
    let TheoremBody::Tactics(block) = &theorem.body else {
        panic!("expected a by block");
    };
    let target = Type::pi("n", Type::Nat, zero_add_goal());
    let mut ctx = Context::new();
    let mut elaborator = Elaborator::new(&mut ctx);
    let (_, trace) = elaborator
        .elaborate_tactic_block(block, &target)
        .expect("the block closes its goal");
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    let steps: Vec<(&str, &str)> = trace
        .steps
        .iter()
        .map(|step| (step.tactic, text(step.span)))
        .collect();
    assert_eq!(steps[1], ("induction", "induction n"));
    assert_eq!(
        steps
            .iter()
            .filter(|(name, _)| *name == "induction")
            .count(),
        1
    );
    // The base is focused first, then the step: `refl` and `intro j` solve
    // the two holes `induction` opened, in order.
    assert_eq!(trace.steps[2].goal + 1, trace.steps[3].goal);
}

// ADR 3.10.26e AC3
#[test]
fn induction_on_a_bool_is_refused() {
    let why = not_applicable_why("theorem t(b: Bool) -> Eq<Bool, b, b> by { induction b; refl }");
    assert!(why.contains("over `Nat` only"), "{why}");
    assert_eq!(
        codes_of("theorem t(b: Bool) -> Eq<Bool, b, b> by { induction b; refl }"),
        ["E0082"]
    );
}

// ADR 3.10.26e AC3
#[test]
fn induction_on_an_unbound_name_is_refused() {
    let source = "theorem t(n: Nat) -> Eq<Nat, 0 + n, n> by { induction k; refl }";
    assert_eq!(not_applicable_why(source), "`k` is not a variable in scope");
    assert_eq!(codes_of(source), ["E0082"]);
}

// ADR 3.10.26e AC3 (D6): a theorem parameter and an `intro`'d hypothesis
// that mention the target are each refused, by name.
#[test]
fn a_local_that_mentions_the_target_is_refused() {
    let parameter =
        "theorem t(n: Nat, h: Eq<Nat, n, 0>) -> Eq<Nat, 0 + n, n> by { induction n; refl }";
    assert_eq!(not_applicable_why(parameter), "`h` mentions `n`");
    let introduced = "theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, 0 + n, n> by { \
                      intro hyp; induction n; refl }";
    assert_eq!(not_applicable_why(introduced), "`hyp` mentions `n`");
    // An `intro`'d target, found among the goal's locals, is
    // `induction_records_one_trace_step`'s.
}

// ADR 3.10.26e AC3 (D7): `ih` proves the predecessor's case, so the step goal
// is `G[x := x + 1]`, and `exact`'s mismatch is E0010.
#[test]
fn the_predecessor_hypothesis_does_not_close_the_step() {
    let errors = elab_err(
        "theorem t(n: Nat) -> Eq<Nat, 0 + n, n> by { induction n; refl; intro j; intro ih; exact ih }",
    );
    assert!(
        matches!(errors[0].kind, ElabErrorKind::TypeMismatch { .. }),
        "{errors:?}"
    );
    assert_eq!(errors[0].kind.code(), "E0010");
}

// ADR 3.10.26e AC3
#[test]
fn an_open_step_is_unsolved() {
    let source = "theorem t(n: Nat) -> Eq<Nat, 0 + n, n> by { induction n; refl }";
    let TacticErrorKind::UnsolvedGoals { goals } = tactic_error(source) else {
        panic!("expected E0083");
    };
    assert_eq!(goals.len(), 1);
    assert!(matches!(goals[0].1, Type::Pi(..)), "{goals:?}");
    assert_eq!(codes_of(source), ["E0083"]);
}

// ADR 3.10.26e AC1: `induction` is an ordinary name outside a `by` block.
#[test]
fn induction_is_an_ordinary_name_outside_a_by_block() {
    elab_ok("fn f(induction: Nat) -> Nat { induction + 1 }");
}

// ADR 3.10.26e: a theorem refused mid-proof leaves its hypotheses out of the
// next theorem's scope, so `h` does not falsely mention the next `n`.
#[test]
fn a_failed_theorems_hypotheses_leave_scope() {
    let source =
        "theorem bad(n: Nat, h: Eq<Nat, n, 0>) -> Eq<Nat, 0 + n, n> by { induction n; refl }
         theorem good(n: Nat) -> Eq<Nat, 0 + n, n> by {
             induction n; refl; intro j; intro ih; exact cong(|x: Nat| x + 1, ih)
         }";
    assert_eq!(codes_of(source), ["E0082"]);
}
