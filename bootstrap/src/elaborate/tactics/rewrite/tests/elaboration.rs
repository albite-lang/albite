//! `rewrite` through the elaborator (ADR 2.10.26e AC 1, AC 3, AC 4, D6).
//!
//! Lemmas are axioms where a proof would only add noise: `elab_ok` runs the
//! elaborator, not the per-module kernel boundary, so what is pinned here is
//! the Core the tactic emits and each refusal's kind and code. The kernel's
//! verdict on that Core is AC 5's, over `src/compiler/test_tactic_rewrite.tg`.

use albite_core::{Context, Term, Type};

use super::{add, endpoints, eq_nat, var};
use crate::ast::{Item, TheoremBody};
use crate::elaborate::error::TacticErrorKind;
use crate::elaborate::tactics::rewrite::abstraction::motive_and_new_goal;
use crate::elaborate::tactics::tests::tactic_error;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::{CoreDef, Elaborator};
use crate::span::Span;

/// `0 + n = n`, the lemma a bare `rewrite` instantiates.
const ADD_ZERO_LEFT: &str = "axiom add_zero_left(n: Nat): Eq<Nat, 0 + n, n>\n";

/// `n + m = m + n`, whose sides both mention every variable.
const ADD_COMM: &str = "axiom add_comm(n: Nat, m: Nat): Eq<Nat, n + m, m + n>\n";

fn core_of(defs: &[CoreDef], name: &str) -> Term {
    defs.iter()
        .find(|def| def.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
        .term
        .term
        .strip_spans()
}

/// Peel `count` `λ`s.
fn under_lambdas(term: &Term, count: usize) -> &Term {
    (0..count).fold(term, |term, _| match term {
        Term::Lambda(_, _, body) => body,
        other => panic!("expected a λ, got {other:?}"),
    })
}

fn not_applicable_why(source: &str) -> String {
    match tactic_error(source) {
        kind @ TacticErrorKind::TacticNotApplicable { .. } => {
            assert_eq!(kind.code(), "E0082");
            let TacticErrorKind::TacticNotApplicable { tactic, why, .. } = kind else {
                unreachable!()
            };
            assert_eq!(tactic, "rewrite");
            why
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

/// E0085's code and message for `source`.
fn no_occurrence_message(source: &str) -> String {
    let kind = tactic_error(source);
    assert!(
        matches!(kind, TacticErrorKind::RewriteNoOccurrence { .. }),
        "expected E0085, got {kind:?}"
    );
    assert_eq!(kind.code(), "E0085");
    kind.default_message()
}

// ADR 2.10.26e AC1: the elaboration half — a parameter named `rewrite`.
#[test]
fn rewrite_stays_an_identifier_outside_a_by_block() {
    let defs = elab_ok("fn f(rewrite: Nat) -> Nat { rewrite }");
    assert_eq!(
        core_of(&defs, "f"),
        Term::lambda("rewrite", Type::Nat, var("rewrite"))
    );
}

// ADR 2.10.26e AC2
#[test]
fn the_motive_passes_validate_synthesised_motive() {
    let goal = eq_nat(add(var("a"), var("a")), var("b"));
    let (motive, _) = motive_and_new_goal(endpoints(&goal), &Type::Nat, &var("a"), &var("b"));
    let mut ctx = Context::new();
    let elaborator = Elaborator::new(&mut ctx);
    let span = Span::new(0, 1);
    assert!(elaborator
        .validate_synthesised_motive(&motive, &Type::Nat, span)
        .is_ok());
    // The validator is not vacuous: a motive over the wrong domain fails.
    assert!(elaborator
        .validate_synthesised_motive(&motive, &Type::Bool, span)
        .is_err());
}

// ADR 2.10.26e AC2 (D6): `rewrite [h, <- h]` is two trace steps, each
// spanning its own rule, exactly as two `rewrite`s would be.
#[test]
fn a_rule_list_records_one_trace_step_per_rule() {
    let source = "theorem t() -> Nat by { intro a; intro b; intro h; rewrite [h, <- h]; refl }";
    let (file, errors) = crate::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    let Item::Theorem(theorem) = &file.items[0] else {
        panic!("expected a theorem");
    };
    let TheoremBody::Tactics(block) = &theorem.body else {
        panic!("expected a by block");
    };
    // `Π a b. Eq a b → Eq (a + a) (b + b)`, built by hand so the block runs
    // with no parameters in scope.
    let target = Type::pi(
        "a",
        Type::Nat,
        Type::pi(
            "b",
            Type::Nat,
            Type::arrow(
                eq_nat(var("a"), var("b")),
                eq_nat(add(var("a"), var("a")), add(var("b"), var("b"))),
            ),
        ),
    );
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
    assert_eq!(
        steps,
        [
            ("intro", "intro a"),
            ("intro", "intro b"),
            ("intro", "intro h"),
            ("rewrite", "h"),
            ("rewrite", "<- h"),
            ("refl", "refl"),
        ]
    );
    // Each rule solved its own goal: the second rewrites the first's hole.
    assert_ne!(trace.steps[3].goal, trace.steps[4].goal);
    assert!(trace.steps.iter().all(|step| !step.exact_sorry));
}

// ADR 2.10.26e AC3: D4's forward row, with `sym` around the citation.
#[test]
fn a_bare_lemma_is_matched_against_the_goal() {
    let source = format!(
        "{ADD_ZERO_LEFT}theorem t(n: Nat) -> Eq<Nat, n + 0, 0 + n> by {{ rewrite [add_zero_left]; refl }}"
    );
    let defs = elab_ok(&source);
    let Term::Subst(base, motive, eq_proof, proof) = under_lambdas(&core_of(&defs, "t"), 1).clone()
    else {
        panic!("expected a Subst");
    };
    assert_eq!(base, Type::Nat);
    let n_plus_zero = add(var("n"), Term::Zero);
    // The motive abstracts `0 + n`, so the new goal is `n + 0 = n`, closed by
    // `refl` at that goal.
    assert!(
        albite_core::types_equal_alpha(
            &motive,
            &Type::pi("#x", Type::Nat, eq_nat(n_plus_zero.clone(), var("#x")))
        ),
        "the motive's body mentions `#x` exactly where `0 + n` stood: {motive}"
    );
    assert_eq!(*proof, Term::refl(Type::Nat, n_plus_zero.clone()));
    // A forward rule is transported along `sym(add_zero_left n)`.
    let Term::Subst(_, _, cited, _) = *eq_proof else {
        panic!("expected `sym`'s Subst, got {eq_proof:?}");
    };
    assert_eq!(
        *cited,
        Term::app(Term::Global("add_zero_left".into()), var("n"))
    );
}

// ADR 2.10.26e AC3: with `<-`, the right side is the pattern and no `sym`.
#[test]
fn a_reversed_bare_lemma_matches_its_right_side() {
    let source = format!(
        "{ADD_COMM}theorem t(a: Nat, b: Nat) -> Eq<Nat, b + a, a + b> by {{ rewrite [<- add_comm]; refl }}"
    );
    let defs = elab_ok(&source);
    let Term::Subst(_, _, eq_proof, proof) = under_lambdas(&core_of(&defs, "t"), 2).clone() else {
        panic!("expected a Subst");
    };
    // `m + n` first matches `b + a` (n := a, m := b): the citation is
    // `add_comm a b`, applied directly, and the goal becomes `a + b = a + b`.
    assert_eq!(
        *eq_proof,
        Term::app(
            Term::app(Term::Global("add_comm".into()), var("a")),
            var("b")
        )
    );
    assert_eq!(*proof, Term::refl(Type::Nat, add(var("a"), var("b"))));
}

// ADR 2.10.26e AC4: E0072, for a value and for a partially applied lemma.
#[test]
fn a_non_equation_rule_is_refused() {
    for rule in ["j", "add_comm(j)"] {
        let source =
            format!("{ADD_COMM}theorem t(j: Nat) -> Eq<Nat, j, j> by {{ rewrite [{rule}]; refl }}");
        let errors = elab_err(&source);
        assert!(
            errors.iter().any(|e| e.kind.code() == "E0072"),
            "{rule}: {errors:?}"
        );
    }
}

// ADR 2.10.26e AC4
#[test]
fn a_non_equation_goal_is_refused() {
    let why = not_applicable_why(&format!(
        "{ADD_ZERO_LEFT}theorem t(a: Nat, b: Nat) -> Eq<Nat, a, b> -> Eq<Nat, b, a> by {{ rewrite [add_zero_left] }}"
    ));
    assert_eq!(why, "rule 1: the goal is not an equation");
}

// ADR 2.10.26e AC4: the message names the rule, the pattern and the goal.
#[test]
fn a_missing_occurrence_is_refused() {
    let message = no_occurrence_message(
        "theorem t(a: Nat, b: Nat, c: Nat, h: Eq<Nat, a, b>) -> Eq<Nat, c, c> by { rewrite [h] }",
    );
    assert_eq!(
        message,
        "`rewrite` rule 1 (`Eq Nat a b`) finds no occurrence of `a` in the goal `Eq Nat c c`"
    );
}

// ADR 2.10.26e AC4: a bare lemma with no match reports its pattern
// uninstantiated, and its position in the list.
#[test]
fn a_bare_lemma_with_no_match_is_refused() {
    let message = no_occurrence_message(&format!(
        "{ADD_COMM}theorem t(a: Nat, h: Eq<Nat, a, a>) -> Eq<Nat, a, a> by {{ rewrite [h, add_comm] }}"
    ));
    assert!(message.starts_with("`rewrite` rule "), "{message}");
    // Rule 1 rewrites nothing (both sides `a`), so that is what is reported.
    assert!(message.contains("rewrites nothing"), "{message}");
    let message = no_occurrence_message(&format!(
        "{ADD_COMM}theorem t(a: Nat) -> Eq<Nat, a, a> by {{ rewrite [add_comm] }}"
    ));
    assert!(
        message.contains("finds no occurrence of `(n + m)`") && !message.contains("did you mean"),
        "{message}"
    );
}

// ADR 2.10.26e AC4 (D3): a rule whose sides are equal rewrites nothing.
#[test]
fn a_rule_with_equal_sides_is_refused() {
    let message = no_occurrence_message(
        "theorem t(a: Nat, h: Eq<Nat, a, a>) -> Eq<Nat, a + 1, a + 1> by { rewrite [h]; refl }",
    );
    assert_eq!(
        message,
        "`rewrite` rule 1 (`Eq Nat a a`) rewrites nothing: both sides are the same term `a`"
    );
}

// ADR 2.10.26e AC4: a binder the pattern does not mention, and a premise.
#[test]
fn an_undetermined_argument_is_refused() {
    let why = not_applicable_why(
        "axiom extra(n: Nat, m: Nat): Eq<Nat, n + 0, n>\n\
         theorem t(k: Nat) -> Eq<Nat, k + 0, k> by { rewrite [extra]; refl }",
    );
    // `m` is non-dependent, so it elaborates as an arrow's domain.
    assert_eq!(
        why,
        "rule 1: cannot infer an argument of type `Nat`; pass explicit arguments"
    );
    let why = not_applicable_why(
        "axiom premised(n: Nat, h: Eq<Nat, n, 0>): Eq<Nat, 0, n>\n\
         theorem t(k: Nat) -> Eq<Nat, 0, k> by { rewrite [premised]; refl }",
    );
    assert_eq!(
        why,
        "rule 1: cannot infer an argument of type `Eq Nat n zero`; pass explicit arguments"
    );
    // A `Π` variable the pattern omits, behind one it mentions.
    let why = not_applicable_why(
        "axiom pinned(n: Nat, m: Nat): Eq<Nat, n + 0, n + 0 + m * 0>\n\
         theorem t(k: Nat) -> Eq<Nat, k + 0, k> by { rewrite [pinned]; refl }",
    );
    assert_eq!(
        why,
        "rule 1: cannot infer `m` from the pattern `(n + zero)`; pass explicit arguments"
    );
}

// ADR 2.10.26e AC4: a bare-variable pattern would match anything.
#[test]
fn a_bare_metavariable_pattern_is_refused() {
    let why = not_applicable_why(&format!(
        "{ADD_ZERO_LEFT}theorem t(k: Nat) -> Eq<Nat, k, k> by {{ rewrite [<- add_zero_left]; refl }}"
    ));
    assert_eq!(
        why,
        "rule 1: the pattern is the bare argument `n`, which matches anything; pass explicit arguments"
    );
}

// ADR 2.10.26e AC4: generic lemmas are refused, as `apply` refuses them.
#[test]
fn a_generic_lemma_is_refused() {
    let why = not_applicable_why(
        "axiom same<T>(x: T): Eq<T, x, x>\n\
         theorem t(k: Nat) -> Eq<Nat, k, k> by { rewrite [same]; refl }",
    );
    assert_eq!(
        why,
        "rule 1: a generic lemma cannot be rewritten by; pass explicit arguments"
    );
}

// ADR 2.10.26e AC4: same code as a missing occurrence, told apart by the
// hint — in both directions, for a term rule and for a bare lemma.
#[test]
fn the_wrong_direction_suggests_the_arrow() {
    let hyp = "theorem t(a: Nat, b: Nat, h: Eq<Nat, a, b>) -> Eq<Nat, b, b> by";
    let message = no_occurrence_message(&format!("{hyp} {{ rewrite [h] }}"));
    assert!(
        message.ends_with("the right-hand side occurs: did you mean `<-`?"),
        "{message}"
    );
    let hyp = "theorem t(a: Nat, b: Nat, h: Eq<Nat, a, b>) -> Eq<Nat, a, a> by";
    let message = no_occurrence_message(&format!("{hyp} {{ rewrite [<- h] }}"));
    assert!(
        message.ends_with("the left-hand side occurs: did you mean the rule without `<-`?"),
        "{message}"
    );
    let message = no_occurrence_message(&format!(
        "axiom succ_ax(n: Nat): Eq<Nat, n + 1, 1 + n>\n\
         theorem t(k: Nat) -> Eq<Nat, 1 + k, 1 + k> by {{ rewrite [succ_ax] }}"
    ));
    assert!(message.ends_with("did you mean `<-`?"), "{message}");
}
