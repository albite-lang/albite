//! AC 5 — each tactic emits the Core D3 and D4 name, through `elab_ok`; and
//! each E0082/E0083 refusal is reachable.

use albite_core::{Term, Type};

use super::tactic_error;
use crate::elaborate::error::{ElabErrorKind, TacticErrorKind};
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::CoreDef;

/// `add_zero_left`, proved by induction in term mode — the citation target.
const ADD_ZERO_LEFT: &str = "
theorem add_zero_left(n: Nat) -> Eq<Nat, 0 + n, n> {
    natind(|k: Nat| Eq<Nat, 0 + k, k>, refl, fn(k: Nat, ih) => cong(|x: Nat| x + 1, ih), n)
}
";

fn core_of(defs: &[CoreDef], name: &str) -> Term {
    defs.iter()
        .find(|def| def.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
        .term
        .term
        .strip_spans()
}

/// The body under the outermost `λ`.
fn under_lambda(term: &Term) -> &Term {
    match term {
        Term::Lambda(_, _, body) => body,
        other => panic!("expected a λ, got {other:?}"),
    }
}

fn not_applicable_why(source: &str) -> String {
    match tactic_error(source) {
        TacticErrorKind::TacticNotApplicable { why, .. } => why,
        other => panic!("expected E0082, got {other:?}"),
    }
}

// ADR 2.10.26c AC5
#[test]
fn tactic_refl_emits_the_term_mode_core() {
    let defs = elab_ok(
        "theorem by_term(n: Nat) -> Eq<Nat, n + 0, n> { refl }
         theorem by_tactic(n: Nat) -> Eq<Nat, n + 0, n> by { refl }",
    );
    assert_eq!(core_of(&defs, "by_term"), core_of(&defs, "by_tactic"));
}

// ADR 2.10.26c AC5
#[test]
fn tactic_exact_emits_the_term_mode_core() {
    let source = format!(
        "{ADD_ZERO_LEFT}
         theorem by_term(n: Nat) -> Eq<Nat, n * 1, n> {{ add_zero_left(n) }}
         theorem by_tactic(n: Nat) -> Eq<Nat, n * 1, n> by {{ exact add_zero_left(n) }}"
    );
    let defs = elab_ok(&source);
    assert_eq!(core_of(&defs, "by_term"), core_of(&defs, "by_tactic"));
}

// ADR 2.10.26c AC5: `intro h` is a `λh` around the next goal's solution.
#[test]
fn intro_emits_a_lambda() {
    let defs = elab_ok(
        "theorem imp(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n + 0, 0> by { intro h; exact h }",
    );
    let body = core_of(&defs, "imp");
    match under_lambda(&body) {
        Term::Lambda(binder, Type::Eq(..), inner) => {
            assert_eq!(binder, "h");
            assert_eq!(**inner, Term::var("h"));
        }
        other => panic!("expected λh:Eq. h, got {other:?}"),
    }
}

// ADR 2.10.26c AC5: one `App` per argument, the matched term for a Π
// variable and the closing proof for a premise.
#[test]
fn apply_emits_the_citation_shape() {
    let source = format!(
        "{ADD_ZERO_LEFT}
         theorem by_apply(k: Nat) -> Eq<Nat, 0 + k, k> by {{ apply add_zero_left }}
         theorem lem(n: Nat, h: Eq<Nat, n, n>) -> Eq<Nat, 0 + n, n> {{ add_zero_left(n) }}
         theorem with_premise(k: Nat) -> Eq<Nat, 0 + k, k> by {{ apply lem; refl }}"
    );
    let defs = elab_ok(&source);
    match under_lambda(&core_of(&defs, "by_apply")) {
        Term::App(head, arg) => {
            assert!(matches!(&**head, Term::Global(n) | Term::Var(n) if n == "add_zero_left"));
            assert_eq!(**arg, Term::var("k"));
        }
        other => panic!("expected add_zero_left k, got {other:?}"),
    }
    match under_lambda(&core_of(&defs, "with_premise")) {
        Term::App(inner, premise) => {
            assert!(matches!(&**premise, Term::Refl(..)), "{premise:?}");
            assert!(matches!(&**inner, Term::App(_, k) if **k == Term::var("k")));
        }
        other => panic!("expected lem k (refl), got {other:?}"),
    }
}

/// A tactic proof is an ordinary global: a term-mode proof can cite it.
#[test]
fn a_tactic_theorem_is_cited_like_any_other() {
    elab_ok(
        "theorem by_tactic(n: Nat) -> Eq<Nat, n + 0, n> by { refl }
         fn uses() -> Bool { let _p: Eq<Nat, 3 + 0, 3> = by_tactic(3); true }",
    );
}

#[test]
fn intro_on_a_non_arrow_is_e0082() {
    let why = not_applicable_why("theorem t(n: Nat) -> Eq<Nat, n, n> by { intro h; refl }");
    assert!(why.contains("not a function type"), "{why}");
}

// ADR 2.10.26c D4: shadowing a parameter is refused, not renamed.
#[test]
fn intro_of_a_bound_name_is_e0082() {
    let why = not_applicable_why(
        "theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> by { intro n; exact n }",
    );
    assert!(why.contains("`n` is already bound"), "{why}");
    let why = not_applicable_why(
        "theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> by { intro h; intro h; exact h }",
    );
    assert!(why.contains("`h` is already bound"), "{why}");
}

#[test]
fn an_apply_that_does_not_match_is_e0082() {
    let source = format!(
        "{ADD_ZERO_LEFT} theorem t(k: Nat) -> Eq<Nat, k * 1, k> by {{ apply add_zero_left }}"
    );
    let why = not_applicable_why(&source);
    assert!(why.contains("does not match"), "{why}");
}

#[test]
fn an_apply_that_leaves_a_variable_unbound_is_e0082() {
    // `n` is a Π variable — a premise mentions it — but the conclusion does not.
    let why = not_applicable_why(
        "theorem anything(n: Nat, h: Eq<Nat, n, n>) -> Eq<Nat, 0, 0> { refl }
         theorem t() -> Eq<Nat, 0, 0> by { apply anything }",
    );
    assert!(why.contains("cannot infer `n`"), "{why}");
}

/// A non-dependent data parameter peels as an arrow; it must not become a
/// term-valued goal (D3).
#[test]
fn an_apply_that_would_leave_a_data_goal_is_e0082() {
    let why = not_applicable_why(
        "theorem anything(n: Nat) -> Eq<Nat, 0, 0> { refl }
         theorem t() -> Eq<Nat, 0, 0> by { apply anything }",
    );
    assert!(
        why.contains("cannot infer an argument of type `Nat`"),
        "{why}"
    );
}

#[test]
fn a_sorry_in_an_apply_head_is_e0082() {
    let why = not_applicable_why(
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { apply (sorry: Eq<Nat, n, n>) }",
    );
    assert!(why.contains("sorry in an apply head"), "{why}");
}

#[test]
fn a_generic_apply_is_e0082() {
    let why = not_applicable_why(
        "theorem same<T>(x: T) -> Eq<T, x, x> { refl }
         theorem t(n: Nat) -> Eq<Nat, n, n> by { apply same }",
    );
    assert!(why.contains("generic"), "{why}");
}

#[test]
fn a_tactic_after_the_last_goal_is_e0082() {
    let why = not_applicable_why("theorem t(n: Nat) -> Eq<Nat, n, n> by { refl; refl }");
    assert!(why.contains("no goal is left"), "{why}");
}

#[test]
fn a_block_that_leaves_a_goal_is_e0083() {
    match tactic_error("theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> by { intro h }") {
        TacticErrorKind::UnsolvedGoals { goals } => {
            assert_eq!(goals.len(), 1);
            assert_eq!(goals[0].0.len(), 1, "the goal keeps `h` in its context");
            assert_eq!(goals[0].0[0].0, "h");
        }
        other => panic!("expected E0083, got {other:?}"),
    }
}

// ADR 2.10.26c AC1/AC6: an empty block parses, then elaborates to E0083.
#[test]
fn an_empty_block_is_e0083() {
    assert!(matches!(
        tactic_error("theorem t(n: Nat) -> Eq<Nat, n, n> by { }"),
        TacticErrorKind::UnsolvedGoals { goals } if goals.len() == 1
    ));
}

/// The two messages a reader sees: no placeholder goal after the last one,
/// and a goal with no locals rendered as a bare `⊢`.
#[test]
fn the_tactic_messages_name_what_the_reader_needs() {
    let errors = elab_err("theorem t(n: Nat) -> Eq<Nat, n, n> by { refl; refl }");
    assert!(
        errors[0].message.contains("`refl` has nothing to act on"),
        "{}",
        errors[0].message
    );
    assert!(!errors[0].message.contains("Unit"), "{}", errors[0].message);
    // An ordinary E0082 names the goal it could not act on.
    let errors = elab_err("theorem t(n: Nat) -> Eq<Nat, n, n> by { intro h; refl }");
    assert!(
        errors[0].message.contains(
            "`intro` does not apply to the goal `Eq Nat n n`: the goal is not a function type"
        ),
        "{}",
        errors[0].message
    );
    let errors = elab_err("theorem t(n: Nat) -> Eq<Nat, n, n> by { }");
    assert!(
        errors[0]
            .message
            .contains("1 unsolved goal(s): ⊢ Eq Nat n n"),
        "{}",
        errors[0].message
    );
    let errors = elab_err("theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> by { intro h }");
    assert!(
        errors[0].message.contains("h : Eq Nat n zero ⊢"),
        "{}",
        errors[0].message
    );
}

/// A wrong `exact` is today's E0010 — the elaborator still checks it.
#[test]
fn a_wrong_exact_is_the_elaborators_own_error() {
    let errors = elab_err("theorem t(n: Nat) -> Eq<Nat, n, n> by { exact 3 }");
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0010"),
        "{errors:?}"
    );
}

/// A poisoned goal (ADR 15.8.26d) passes through `intro` without a second
/// diagnostic: the block reports only that the goal stayed open.
#[test]
fn intro_passes_poison_through() {
    use crate::ast::{Ident, Tactic, TacticBlock};
    use crate::span::Span;
    let span = Span::new(0, 1);
    let block = TacticBlock {
        tactics: vec![Tactic::Intro(Ident::new("h", span), span)],
        span,
    };
    let mut ctx = albite_core::Context::new();
    let mut elaborator = crate::elaborate::Elaborator::new(&mut ctx);
    let error = elaborator
        .elaborate_tactic_block(&block, &Type::Error)
        .expect_err("the poisoned body is still an open goal");
    match error.kind {
        ElabErrorKind::Tactic(TacticErrorKind::UnsolvedGoals { goals }) => {
            assert_eq!(goals[0].0, vec![("h".to_string(), Type::Error)]);
            assert_eq!(goals[0].1, Type::Error);
        }
        other => panic!("expected E0083, got {other:?}"),
    }
}
