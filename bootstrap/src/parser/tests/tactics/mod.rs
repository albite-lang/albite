//! AC 1 — `by` blocks parse, and tactic names stay identifiers
//! (ADR 2.10.26c D1).

mod disjunction;
mod exists;
mod omega;
mod ring;
mod simp;

use super::*;
use crate::ast::CasesArm;
use crate::parser::tactics::EXPECTED_A_TACTIC;

fn theorem_body(file: &SourceFile, index: usize) -> &TheoremBody {
    match &file.items[index] {
        Item::Theorem(t) | Item::Lemma(t) => &t.body,
        other => panic!("expected a theorem, got {other:?}"),
    }
}

fn tactics_of(body: &TheoremBody) -> &[Tactic] {
    match body {
        TheoremBody::Tactics(block) => &block.tactics,
        TheoremBody::Term(e) => panic!("expected a by block, got a term {e:?}"),
    }
}

// ADR 2.10.26c AC1
#[test]
fn a_by_block_parses_each_tactic() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { intro h; exact h; apply f; refl }
lemma l(n: Nat): n + 0 == n by { intro h; exact h; apply f; refl; }";
    let file = parse_ok(source);
    for index in 0..2 {
        let tactics = tactics_of(theorem_body(&file, index));
        let names: Vec<&str> = tactics.iter().map(Tactic::name).collect();
        assert_eq!(names, ["intro", "exact", "apply", "refl"]);
        assert!(matches!(&tactics[0], Tactic::Intro(binder, _) if binder.name == "h"));
    }
    // Each step's span covers the step, from its name to its end.
    let first = tactics_of(theorem_body(&file, 0));
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    assert_eq!(text(first[0].span()), "intro h");
    assert_eq!(text(first[1].span()), "exact h");
    assert_eq!(text(first[2].span()), "apply f");
    assert_eq!(text(first[3].span()), "refl");
    let TheoremBody::Tactics(block) = theorem_body(&file, 0) else {
        unreachable!()
    };
    assert_eq!(text(block.span), "by { intro h; exact h; apply f; refl }");
}

// ADR 2.10.26c AC1
#[test]
fn a_term_body_parses_unchanged() {
    let file = parse_ok(
        "theorem a(n: Nat) -> Eq<Nat, n, n> { refl }
         theorem b(n: Nat) -> Eq<Nat, n, n> = refl",
    );
    for index in 0..2 {
        assert!(matches!(theorem_body(&file, index), TheoremBody::Term(_)));
    }
}

// ADR 2.10.26c AC1: contextual names, not keywords.
#[test]
fn tactic_names_are_not_keywords() {
    parse_ok("fn apply(intro: Nat) -> Nat { intro }");
    parse_ok("fn f() -> Nat { let exact = 1; exact }");
}

// ADR 2.10.26c AC1
#[test]
fn an_unknown_tactic_is_a_parse_error() {
    // A name no planned tactic takes: `simp` and `omega` were (ADR 6.10.26f D1).
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { not_a_tactic }");
    assert!(
        errors.iter().any(|e| matches!(
            &e.kind,
            crate::error::ParseErrorKind::Expected(what) if what == EXPECTED_A_TACTIC
        )),
        "{errors:?}"
    );
}

// ADR 2.10.26c AC1: the elaborator then reports E0083 (steps_tests).
#[test]
fn an_empty_block_parses() {
    let file = parse_ok("theorem t(n: Nat) -> Eq<Nat, n, n> by { }");
    assert!(tactics_of(theorem_body(&file, 0)).is_empty());
}

/// The theorem's span ends where its body does, for a term and for a block —
/// `TheoremBody::span` is what `parse_theorem` reads the end from.
#[test]
fn a_theorem_span_ends_at_its_body() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { refl }\ntheorem u(n: Nat) -> Eq<Nat, n, n> { refl }";
    let file = parse_ok(source);
    for (index, tail) in [(0, "by { refl }"), (1, "{ refl }")] {
        let Item::Theorem(t) = &file.items[index] else {
            panic!("expected a theorem");
        };
        assert!(source[..t.span.end as usize].ends_with(tail), "{index}");
        assert_eq!(t.body.span().end, t.span.end);
        assert!(
            t.body.span().start > t.span.start,
            "the body is not the whole item"
        );
    }
}

/// The walkers visit a block's `exact`/`apply` arguments, and nothing for
/// `intro`/`refl`; a term body is its one expression (D11).
#[test]
fn a_body_lists_the_expressions_a_user_wrote() {
    let file = parse_ok(
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { intro h; exact first; apply second; refl }\n\
         theorem u(n: Nat) -> Eq<Nat, n, n> { refl }",
    );
    let names: Vec<String> = theorem_body(&file, 0)
        .expressions()
        .iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(
        names[0].contains("first") && names[1].contains("second"),
        "{names:?}"
    );
    assert_eq!(theorem_body(&file, 1).expressions().len(), 1);
}

/// A missing separator ends the block, and the stray step is an error.
#[test]
fn steps_need_a_separator() {
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { refl refl }");
    assert!(!errors.is_empty());
}

fn has_expected(errors: &[crate::error::ParseError], what: &str) -> bool {
    errors
        .iter()
        .any(|e| matches!(&e.kind, crate::error::ParseErrorKind::Expected(found) if found == what))
}

// ADR 2.10.26e AC1
#[test]
fn rewrite_parses_a_bracketed_rule_list() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { rewrite [h]; \
                  rewrite [nat_arith::add_succ_left(j, n), <- ih]; refl }";
    let file = parse_ok(source);
    let tactics = tactics_of(theorem_body(&file, 0));
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    let Tactic::Rewrite(single, span) = &tactics[0] else {
        panic!("expected a rewrite, got {:?}", tactics[0]);
    };
    assert_eq!(text(*span), "rewrite [h]");
    assert_eq!(single.len(), 1);
    assert!(!single[0].reverse);
    assert_eq!(text(single[0].span), "h");
    let Tactic::Rewrite(pair, span) = &tactics[1] else {
        panic!("expected a rewrite, got {:?}", tactics[1]);
    };
    assert_eq!(
        text(*span),
        "rewrite [nat_arith::add_succ_left(j, n), <- ih]"
    );
    assert_eq!(tactics[1].name(), "rewrite");
    assert_eq!(tactics[1].span(), *span);
    let flags: Vec<bool> = pair.iter().map(|rule| rule.reverse).collect();
    assert_eq!(flags, [false, true]);
    assert_eq!(text(pair[0].span), "nat_arith::add_succ_left(j, n)");
    assert_eq!(text(pair[1].span), "<- ih");
    // The walkers see each rule's expression (D11).
    assert_eq!(theorem_body(&file, 0).expressions().len(), 3);
}

// ADR 2.10.26e AC1: `<` at a rule's start means only the `<-` pair.
#[test]
fn a_lone_lt_at_a_rule_start_is_refused() {
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { rewrite [< h] }");
    assert!(
        has_expected(&errors, crate::parser::tactics::EXPECTED_A_REVERSE_ARROW),
        "{errors:?}"
    );
}

// ADR 2.10.26e AC1
#[test]
fn rewrite_without_brackets_is_refused() {
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { rewrite h }");
    assert!(
        has_expected(&errors, crate::parser::tactics::EXPECTED_A_RULE_LIST),
        "{errors:?}"
    );
    assert!(crate::parser::tactics::EXPECTED_A_RULE_LIST.contains("rewrite [h]"));
}

// ADR 2.10.26e AC1
#[test]
fn an_unknown_tactic_lists_rewrite() {
    for name in ["intro", "exact", "apply", "refl", "rewrite"] {
        assert!(EXPECTED_A_TACTIC.contains(name), "{name}");
    }
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { rw [h] }");
    assert!(has_expected(&errors, EXPECTED_A_TACTIC), "{errors:?}");
    // The helper's other half: an expectation no error carries is absent.
    let rule_list = crate::parser::tactics::EXPECTED_A_RULE_LIST;
    assert!(!has_expected(&errors, rule_list), "{errors:?}");
    assert!(!has_expected(&[], EXPECTED_A_TACTIC));
}

// ADR 2.10.26e AC1: `rewrite` is contextual, as the self-host's
// `harness::rewrite` module needs (D1). The elaboration half is in
// `elaborate/tactics/rewrite/tests/`.
#[test]
fn rewrite_stays_an_identifier_outside_a_by_block() {
    parse_ok("mod rewrite;");
    parse_ok("use a::rewrite::{f};");
    parse_ok("fn f(rewrite: Nat) -> Nat { rewrite }");
}

// ADR 3.10.26e AC1
#[test]
fn induction_parses_a_variable() {
    let source = "theorem t(n: Nat) -> Eq<Nat, n, n> by { induction n; refl }";
    let file = parse_ok(source);
    let tactics = tactics_of(theorem_body(&file, 0));
    let Tactic::Induction(target, span) = &tactics[0] else {
        panic!("expected an induction, got {:?}", tactics[0]);
    };
    assert_eq!(target.name, "n");
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        "induction n"
    );
    assert_eq!(tactics[0].span(), *span);
    assert_eq!(tactics[0].name(), "induction");
    // An `induction` writes no expression for the walkers (D11).
    assert!(theorem_body(&file, 0).expressions().is_empty());
}

// ADR 3.10.26e AC1: no target, and an expression target, both name the form.
#[test]
fn induction_without_a_variable_is_refused() {
    let expected = crate::parser::tactics::EXPECTED_AN_INDUCTION_TARGET;
    assert!(expected.contains("induction <variable>"));
    for source in [
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { induction; refl }",
        "theorem t(n: Nat) -> Eq<Nat, n, n> by { induction }",
    ] {
        let (_, errors) = parse(source);
        assert!(has_expected(&errors, expected), "{source}: {errors:?}");
    }
}

// ADR 3.10.26e AC1
#[test]
fn induction_on_an_expression_is_refused() {
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { induction n + 1; refl }");
    assert!(
        has_expected(
            &errors,
            crate::parser::tactics::EXPECTED_AN_INDUCTION_TARGET
        ),
        "{errors:?}"
    );
    // The last step of a block may end at the brace.
    parse_ok("theorem t(n: Nat) -> Eq<Nat, n, n> by { induction n }");
}

// ADR 3.10.26e AC1
#[test]
fn an_unknown_tactic_lists_induction() {
    assert!(EXPECTED_A_TACTIC.contains("induction"));
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { not_a_tactic n }");
    assert!(has_expected(&errors, EXPECTED_A_TACTIC), "{errors:?}");
}

// ADR 3.10.26e AC1: `induction` is contextual, as the self-host's
// `elab::exprs::forms::induction` module needs (D1). The elaboration half is
// in `elaborate/tactics/tests/induction_tests.rs`.
#[test]
fn induction_stays_an_identifier_outside_a_by_block() {
    parse_ok("mod induction;");
    parse_ok("use a::induction::{f};");
    parse_ok("fn f(induction: Nat) -> Nat { induction }");
}

/// The arms of the first tactic of theorem `index`, which must be a `cases`.
fn cases_arms(file: &SourceFile, index: usize) -> &[CasesArm] {
    match &tactics_of(theorem_body(file, index))[0] {
        Tactic::Cases(_, arms, _) => arms,
        other => panic!("expected a cases, got {other:?}"),
    }
}

/// Each arm's constructor and binders, `_` as `"_"`.
fn arm_shapes(arms: &[CasesArm]) -> Vec<(&str, Vec<&str>)> {
    arms.iter()
        .map(|arm| {
            let binders = arm
                .binders
                .iter()
                .map(|binder| binder.as_ref().map_or("_", |b| b.name.as_str()))
                .collect();
            (arm.ctor.name.as_str(), binders)
        })
        .collect()
}

// ADR 3.10.26f AC1
#[test]
fn cases_parses_an_arm_per_constructor() {
    let source = "theorem t(l: Lst) -> Eq<Lst, l, l> by {
    cases l { Nil => refl, Cons(h, t) => { refl; }, Pair(_, b) => exact e, Some(x) => refl, }
}";
    let file = parse_ok(source);
    let arms = cases_arms(&file, 0);
    assert_eq!(
        arm_shapes(arms),
        [
            ("Nil", vec![]),
            ("Cons", vec!["h", "t"]),
            ("Pair", vec!["_", "b"]),
            ("Some", vec!["x"]),
        ]
    );
    let text = |span: Span| &source[span.start as usize..span.end as usize];
    assert_eq!(text(arms[0].span), "Nil => refl");
    assert_eq!(text(arms[1].span), "Cons(h, t) => { refl; }");
    let bodies: Vec<Vec<&str>> = arms
        .iter()
        .map(|arm| arm.body.iter().map(Tactic::name).collect())
        .collect();
    assert_eq!(
        bodies,
        [vec!["refl"], vec!["refl"], vec!["exact"], vec!["refl"]]
    );
    let tactic = &tactics_of(theorem_body(&file, 0))[0];
    assert_eq!(tactic.name(), "cases");
    assert!(text(tactic.span()).starts_with("cases l {"));
    assert!(text(tactic.span()).ends_with('}'));
    // The scrutinee and the arms' expressions reach the walkers (D11).
    assert_eq!(theorem_body(&file, 0).expressions().len(), 2);
}

// ADR 3.10.26f AC1
#[test]
fn cases_without_arms_is_refused() {
    let expected = crate::parser::tactics::EXPECTED_A_CASES_ARM;
    let (_, errors) = parse("theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o {} }");
    assert!(has_expected(&errors, expected), "{errors:?}");
}

// ADR 3.10.26f AC1: a nested pattern and a literal are not binders.
#[test]
fn a_nested_pattern_in_a_cases_arm_is_refused() {
    let expected = crate::parser::tactics::EXPECTED_A_CASES_BINDER;
    for source in [
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(Some(x)) => refl } }",
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(0) => refl } }",
    ] {
        let (_, errors) = parse(source);
        assert!(has_expected(&errors, expected), "{source}: {errors:?}");
    }
}

// ADR 3.10.26f AC1
#[test]
fn an_unknown_tactic_lists_cases() {
    assert!(EXPECTED_A_TACTIC.contains("cases"));
    let (_, errors) = parse("theorem t(n: Nat) -> Eq<Nat, n, n> by { destruct n }");
    assert!(has_expected(&errors, EXPECTED_A_TACTIC), "{errors:?}");
}

// ADR 3.10.26f AC1: `cases` is contextual (D1).
#[test]
fn cases_stays_an_identifier_outside_a_by_block() {
    parse_ok("mod cases;");
    parse_ok("use a::cases::{f};");
    parse_ok("fn f(cases: Nat) -> Nat { cases }");
}
