//! A lone arm that names an inner constructor takes the nested route (ADR
//! 13.10.26u): the inner match receives the outer catch-all, or refuses the
//! match as incomplete. Before it, the arm was destructured as if it could
//! not fail, and every other inner constructor ran a synthesised hole.

use std::collections::HashMap;

use crate::ast::{Expr, Ident, MatchArm, Path, Pattern};
use crate::elaborate::error::ElabError;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::Elaborator;
use crate::span::Span;

use super::core_of;

/// §1's types: a two-constructor payload inside a three-constructor type.
const TYPES: &str = r#"
    type M = Som(Nat) | Non
    type P = A(Nat) | B(M) | C(Nat)
"#;

fn span() -> Span {
    Span::new(0, 0)
}

fn ctor(name: &str, sub_patterns: Vec<Pattern>) -> Pattern {
    let path = Path {
        segments: vec![Ident::new(name, span())],
        span: span(),
    };
    Pattern::Constructor(path, sub_patterns, span())
}

fn var(name: &str) -> Pattern {
    Pattern::Var(Ident::new(name, span()))
}

fn arm(pattern: Pattern) -> MatchArm {
    MatchArm {
        pattern,
        guard: None,
        body: Expr::Unit(span()),
        span: span(),
    }
}

/// `needs_nested_match` for constructor index 1 holding exactly `arms`.
fn needs_nested(arms: &[MatchArm]) -> bool {
    let mut ctor_arms: HashMap<usize, Vec<&MatchArm>> = HashMap::new();
    ctor_arms.insert(1, arms.iter().collect());
    Elaborator::needs_nested_match(&ctor_arms, 1)
}

/// The errors in `errors` with `code`, which must be exactly one.
fn only(code: &str, errors: &[ElabError]) -> ElabError {
    let found: Vec<_> = errors.iter().filter(|e| e.kind.code() == code).collect();
    assert_eq!(found.len(), 1, "expected one {code}, got: {errors:?}");
    found[0].clone()
}

/// No `sorry` hole anywhere in the definition `name`.
fn assert_no_hole(defs: &[crate::elaborate::output::CoreDef], name: &str) {
    let core = core_of(defs, name);
    assert!(!core.contains("sorry"), "{name} still holes: {core}");
}

// 13.10.26u AC2: the predicate, over in-memory arm maps.
#[test]
fn a_lone_arm_naming_an_inner_constructor_needs_a_nested_match() {
    assert!(needs_nested(&[arm(ctor(
        "B",
        vec![ctor("Som", vec![var("n")])]
    ))]));
    assert!(needs_nested(&[
        arm(ctor("B", vec![var("n")])),
        arm(ctor("B", vec![var("m")])),
    ]));
    assert!(!needs_nested(&[arm(ctor("B", vec![var("n")]))]));
    assert!(!needs_nested(&[arm(ctor(
        "B",
        vec![Pattern::Wildcard(span())]
    ))]));
}

// 13.10.26u AC2: an index with no arms needs no inner match.
#[test]
fn a_constructor_with_no_arm_needs_no_nested_match() {
    let ctor_arms: HashMap<usize, Vec<&MatchArm>> = HashMap::new();
    assert!(!Elaborator::needs_nested_match(&ctor_arms, 1));
}

// 13.10.26u AC2: §1 row 1, a lone nested arm between two plain ones.
#[test]
fn a_lone_nested_arm_with_a_trailing_catch_all_leaves_no_hole() {
    let defs = elab_ok(&format!(
        "{TYPES} fn f(q: P) -> Nat {{ match q {{ A(n) => 1, B(Som(n)) => 2, C(n) => 3, _ => 7 }} }}"
    ));
    assert_no_hole(&defs, "f");
}

// 13.10.26u AC2: §1 row 3, the catch-all also covers `C`.
#[test]
fn a_lone_nested_arm_sharing_its_catch_all_leaves_no_hole() {
    let defs = elab_ok(&format!(
        "{TYPES} fn f(q: P) -> Nat {{ match q {{ A(n) => 1, B(Som(n)) => 2, _ => 7 }} }}"
    ));
    assert_no_hole(&defs, "f");
}

// 13.10.26u AC2: §1 row 4, the binary-sum walk.
#[test]
fn a_lone_nested_arm_over_a_binary_sum_leaves_no_hole() {
    let defs = elab_ok(
        "type M = Som(Nat) | Non
         type Q = A(Nat) | B(M)
         fn f(q: Q) -> Nat { match q { B(Som(n)) => 2, _ => 7 } }",
    );
    assert_no_hole(&defs, "f");
}

// 13.10.26u AC2: §1 row 5, a named catch-all still names the outer value.
#[test]
fn a_lone_nested_arm_with_a_named_catch_all_leaves_no_hole() {
    let defs = elab_ok(
        "type M = Som(Nat) | Non
         type W = Wr(M) | Ot(Nat)
         fn g(x: W) -> Nat { 7 }
         fn f(q: W) -> Nat { match q { Wr(Som(n)) => 2, Ot(n) => 3, other => g(other) } }",
    );
    assert_no_hole(&defs, "f");
    let core = core_of(&defs, "f");
    assert!(core.contains("= q in (global:g other)"), "{core}");
}

// 13.10.26u AC2: §1 row 7, a three-constructor inner type (the flat path).
#[test]
fn a_lone_nested_arm_over_three_inner_constructors_leaves_no_hole() {
    let defs = elab_ok(
        "type T3 = Tri1(Nat) | Tri2 | Tri3
         type P = A(Nat) | B(T3) | C(Nat)
         fn f(q: P) -> Nat { match q { A(n) => 1, B(Tri1(n)) => 2, C(n) => 3, _ => 7 } }",
    );
    assert_no_hole(&defs, "f");
}

// 13.10.26u AC2: the nested route checks a generic nullary body against the
// match's type rather than inferring it.
#[test]
fn a_lone_nested_arm_checks_a_generic_nullary_body() {
    let defs = elab_ok(&format!(
        "{TYPES}
         pub type List<T> = | Nil | Cons(T, List<T>)
         fn f(q: P) -> List<Nat> {{ match q {{ B(Som(n)) => Nil(), _ => Cons(1, Nil()) }} }}"
    ));
    assert_no_hole(&defs, "f");
}

// 13.10.26u D1: the first-arm fallback (a lone arm on a payload the nested
// route cannot match — a single-constructor chain) elaborates a generic
// nullary body without a hole. Inference and checking agree on this input, so
// this pins the path, not the `result_ty` it is now given.
#[test]
fn the_first_arm_fallback_elaborates_a_generic_nullary_body() {
    let defs = elab_ok(
        "pub type List<T> = | Nil | Cons(T, List<T>)
         type D = MkD(Nat)
         type C = MkC(D)
         fn f(c: C) -> List<Nat> { match c { MkC(MkD(n)) => Nil() } }",
    );
    assert_no_hole(&defs, "f");
}

// 13.10.26u AC2: §1 row 2 — no catch-all is E0020, spanned on the arm.
#[test]
fn a_lone_nested_arm_without_a_catch_all_is_e0020_at_the_arm() {
    let source = format!(
        "{TYPES} fn f(q: P) -> Nat {{ match q {{ A(n) => 1, B(Som(n)) => 2, C(n) => 3 }} }}"
    );
    let err = only("E0020", &elab_err(&source));
    let at = &source[err.span.start as usize..err.span.end as usize];
    assert!(at.starts_with("B(Som(n))"), "spanned on `{at}`");
}

// 13.10.26u D1 (as built): a GROUP of nested arms on a single-field payload
// the nested route cannot match (a single-constructor chain) keeps today's
// E0021 rather than silently taking the first arm and dropping the second.
#[test]
fn a_nested_group_on_an_unmatchable_payload_stays_e0021() {
    let source = "type D = MkD(Nat)
                  type C = MkC(D)
                  fn f(c: C) -> Nat { match c { MkC(MkD(n)) => n, MkC(MkD(m)) => m } }";
    only("E0021", &elab_err(source));
}

// 13.10.26u AC2: §1 row 6 — a lone nested arm over two fields is E0021.
#[test]
fn a_lone_nested_arm_over_two_fields_is_e0021() {
    let source = "type L = Cons(Nat, L) | Nil
                  fn f(l: L) -> Nat { match l { Cons(x, Nil()) => 1, _ => 0 } }";
    let err = only("E0021", &elab_err(source));
    assert!(err.message.contains("multi-field"), "{}", err.message);
}
