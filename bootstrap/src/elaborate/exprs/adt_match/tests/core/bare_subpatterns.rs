//! A bare nullary name inside a constructor pattern names that constructor
//! (ADR 13.10.26h): `Som(A)` is `Som(A())`, never a binder that takes every
//! payload and drops the arms after it.

use super::core_of;
use crate::ast::{self, MatchArm, Pattern};
use crate::elaborate::exprs::adt_match::classification::name_bare_nullary_subpatterns;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::span::Span;

/// §1.1's types: a three-constructor payload, an option of it, a list.
const TYPES: &str = r#"
    type E = A | B | C
    type O = Som(E) | Non
    type L = Cons(Nat, L) | Nil
    type P = Pr(E, Nat) | Q
    type Col = Red | Grn
"#;

// ----------------------------------------------------------------------------
// The rewrite over in-memory arms
// ----------------------------------------------------------------------------

fn ident(name: &str) -> ast::Ident {
    ast::Ident::new(name, Span::new(0, 0))
}

fn var(name: &str) -> Pattern {
    Pattern::Var(ident(name))
}

fn ctor(name: &str, subs: Vec<Pattern>) -> Pattern {
    Pattern::Constructor(ast::Path::simple(ident(name)), subs, Span::new(0, 0))
}

fn arm(pattern: Pattern) -> MatchArm {
    MatchArm {
        pattern,
        guard: None,
        body: ast::Expr::Unit(Span::new(0, 0)),
        span: Span::new(0, 0),
    }
}

/// A pattern in source spelling: `Som(A())` for a constructor, `x` for a var.
fn spelled(pattern: &Pattern) -> String {
    match pattern {
        Pattern::Var(id) => id.name.clone(),
        Pattern::Wildcard(_) => "_".to_string(),
        Pattern::Constructor(path, subs, _) => {
            let inner: Vec<String> = subs.iter().map(spelled).collect();
            format!("{}({})", path.item_name().name, inner.join(", "))
        }
        Pattern::Tuple(subs, _) => {
            let inner: Vec<String> = subs.iter().map(spelled).collect();
            format!("({})", inner.join(", "))
        }
        Pattern::Or(left, right, _) => format!("{} | {}", spelled(left), spelled(right)),
        other => format!("{other:?}"),
    }
}

/// The resolver the tests inject: `A`, `B` and `Nil` are nullary.
fn is_nullary(id: &ast::Ident) -> bool {
    matches!(id.name.as_str(), "A" | "B" | "Nil")
}

/// Each arm's pattern after the rewrite, in source spelling.
fn rewritten(patterns: Vec<Pattern>) -> Vec<String> {
    let arms: Vec<MatchArm> = patterns.into_iter().map(arm).collect();
    name_bare_nullary_subpatterns(&arms, is_nullary)
        .iter()
        .map(|a| spelled(&a.pattern))
        .collect()
}

// 13.10.26h AC2: `Som(A)` reads as `Som(A())`, `Cons(x, Nil)` as `Cons(x, Nil())`.
#[test]
fn a_bare_inner_nullary_name_is_bracketed() {
    assert_eq!(
        rewritten(vec![
            ctor("Som", vec![var("A")]),
            ctor("Cons", vec![var("x"), var("Nil")]),
        ]),
        vec!["Som(A())", "Cons(x, Nil())"]
    );
}

// 13.10.26h AC2: a binder, an unknown name and a top-level name stay as written.
#[test]
fn a_binder_an_unknown_name_and_a_top_level_name_are_left_alone() {
    assert_eq!(
        rewritten(vec![
            ctor("Som", vec![var("x")]),
            ctor("Som", vec![var("Unknown")]),
            var("A"),
        ]),
        vec!["Som(x)", "Som(Unknown)", "A"]
    );
}

// 13.10.26h D1: the rewrite reaches every depth below a constructor.
#[test]
fn the_rewrite_reaches_nested_tuples_or_patterns_and_constructors() {
    let tuple = Pattern::Tuple(vec![var("A"), var("y")], Span::new(0, 0));
    let or = Pattern::Or(Box::new(var("A")), Box::new(var("B")), Span::new(0, 0));
    assert_eq!(
        rewritten(vec![
            ctor("Som", vec![ctor("Som", vec![var("B")])]),
            ctor("Wrap", vec![tuple]),
            ctor("Wrap", vec![or]),
        ]),
        vec!["Som(Som(B()))", "Wrap((A(), y))", "Wrap(A() | B())"]
    );
}

// 13.10.26h D1: a nullary name on either side of an or-pattern, or inside a
// tuple, is enough to rewrite a match whose other arms need nothing.
#[test]
fn a_single_or_or_tuple_sub_pattern_still_triggers_the_rewrite() {
    let either_side = |left: Pattern, right: Pattern| {
        let or = Pattern::Or(Box::new(left), Box::new(right), Span::new(0, 0));
        rewritten(vec![ctor("Wrap", vec![or])])
    };
    assert_eq!(either_side(var("x"), var("A")), vec!["Wrap(x | A())"]);
    assert_eq!(either_side(var("B"), var("y")), vec!["Wrap(B() | y)"]);
    let tuple = Pattern::Tuple(vec![var("y"), var("Nil")], Span::new(0, 0));
    assert_eq!(
        rewritten(vec![ctor("Wrap", vec![tuple])]),
        vec!["Wrap((y, Nil()))"]
    );
}

// 13.10.26h D2: a match with nothing to rewrite is borrowed, not cloned.
#[test]
fn nothing_to_rewrite_borrows_the_arms() {
    let arms = vec![arm(ctor("Som", vec![var("x")])), arm(var("A"))];
    let named = name_bare_nullary_subpatterns(&arms, is_nullary);
    assert!(matches!(named, std::borrow::Cow::Borrowed(_)));
    let arms = vec![arm(var("x")), arm(ctor("Som", vec![var("B")]))];
    let named = name_bare_nullary_subpatterns(&arms, is_nullary);
    assert!(matches!(named, std::borrow::Cow::Owned(_)));
    assert_eq!(spelled(&named[0].pattern), "x");
}

// ----------------------------------------------------------------------------
// Through the elaborator: the bare program is its bracketed twin
// ----------------------------------------------------------------------------

/// The Core of `f` for one function body over §1.1's types.
fn core_of_f(signature_and_body: &str) -> String {
    core_of(&elab_ok(&format!("{TYPES} {signature_and_body}")), "f")
}

fn assert_twins(bare: &str, bracketed: &str) {
    let (bare_core, bracketed_core) = (core_of_f(bare), core_of_f(bracketed));
    assert_eq!(bare_core, bracketed_core, "bare: {bare}");
}

// 13.10.26h AC2: §1.1 row 1, a catch-all after two nested arms.
#[test]
fn two_nested_arms_and_a_catch_all_equal_their_bracketed_twin() {
    assert_twins(
        "fn f(o: O) -> Nat { match o { Som(A) => 1, Som(B) => 2, other => 3 } }",
        "fn f(o: O) -> Nat { match o { Som(A()) => 1, Som(B()) => 2, other => 3 } }",
    );
    let core = core_of_f("fn f(o: O) -> Nat { match o { Som(A) => 1, Som(B) => 2, other => 3 } }");
    assert!(!core.contains("let A :"), "A is no binder: {core}");
}

// 13.10.26h AC2: §1.1 row 3, every payload named, no catch-all.
#[test]
fn an_exhaustive_nested_match_equals_its_bracketed_twin() {
    assert_twins(
        "fn f(o: O) -> Nat { match o { Som(A) => 1, Som(B) => 2, Som(C) => 3, Non => 0 } }",
        "fn f(o: O) -> Nat { match o { Som(A()) => 1, Som(B()) => 2, Som(C()) => 3, Non => 0 } }",
    );
}

// 13.10.26h AC2: §1.1's `Som(B) => 2` row, a lone nested arm. Without the
// catch-all it is E0020 since 13.10.26u, which is the hole that row reached.
#[test]
fn a_lone_nested_arm_equals_its_bracketed_twin() {
    assert_twins(
        "fn f(o: O) -> Nat { match o { Som(B) => 2, _ => 0 } }",
        "fn f(o: O) -> Nat { match o { Som(B()) => 2, _ => 0 } }",
    );
}

fn codes(source: &str) -> Vec<String> {
    elab_err(&format!("{TYPES} {source}"))
        .iter()
        .map(|e| e.kind.code().to_string())
        .collect()
}

// 13.10.26h AC2: the multi-field rows are refused as their bracketed twins are.
#[test]
fn bare_names_in_a_multi_field_constructor_are_e0021() {
    let pair = "fn f(p: P) -> Nat { match p { Pr(A, n) => 1, Pr(B, n) => 2, _ => 3 } }";
    let list = "fn f(l: L) -> Nat { match l { Cons(x, Nil) => 1, Cons(x, rest) => 2, Nil => 0 } }";
    assert!(
        codes(pair).contains(&"E0021".to_string()),
        "{:?}",
        codes(pair)
    );
    assert!(
        codes(list).contains(&"E0021".to_string()),
        "{:?}",
        codes(list)
    );
}

// 13.10.26h AC2: another type's nullary constructor in a sub-pattern is E0022.
#[test]
fn a_foreign_inner_nullary_name_is_e0022() {
    let foreign = "fn f(o: O) -> Nat { match o { Som(Red) => 1, Som(B) => 2, _ => 3 } }";
    assert!(
        codes(foreign).contains(&"E0022".to_string()),
        "{:?}",
        codes(foreign)
    );
}

// ----------------------------------------------------------------------------
// Through the module-tree walk: the kernel sees the source's meaning
// ----------------------------------------------------------------------------

/// `f` with §1.1 row 1's arms, and one theorem about `f(Som(B))`.
fn theorem_project(claimed: u32) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a scratch dir");
    let path = dir.path().join("main.tg");
    let source = format!(
        "{TYPES}
        fn f(o: O) -> Nat {{ match o {{ Som(A) => 1, Som(B) => 2, other => 3 }} }}
        theorem f_b_claim : f(Som(B)) == {claimed} = refl
        fn main() -> Nat {{ 0 }}
        "
    );
    std::fs::write(&path, source).expect("written");
    (dir, path)
}

// 13.10.26h AC3: the false theorem, which the kernel admitted, is refused.
#[test]
fn the_theorem_that_f_of_som_b_is_one_is_refused() {
    let (_dir, path) = theorem_project(1);
    let error = crate::driver::elaborate_project_cold(&path, 20)
        .err()
        .expect("f(Som(B)) is not 1");
    assert!(format!("{error:?}").contains("refl"), "{error:?}");
}

// 13.10.26h AC3: the true one is admitted.
#[test]
fn the_theorem_that_f_of_som_b_is_two_is_admitted() {
    let (_dir, path) = theorem_project(2);
    let project = crate::driver::elaborate_project_cold(&path, 20)
        .unwrap_or_else(|error| panic!("f(Som(B)) is 2: {error:?}"));
    assert!(project.defs.iter().any(|d| d.name.ends_with("f_b_claim")));
}
