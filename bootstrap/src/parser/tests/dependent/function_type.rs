//! `(x: A) -> B` in type position (ADR 7.10.26n D1, AC 1): one token of
//! lookahead after `(` — an identifier and a single `:` — opens a binder, and
//! every other opening parses exactly as before.

use super::*;

fn first_param_type(source: &str) -> TypeExpr {
    let file = parse_ok(source);
    unwrap_fn(&file).params[0].ty.clone()
}

/// `(binder, domain, codomain)` of a dependent function type.
fn pi_parts(ty: &TypeExpr) -> (&str, &TypeExpr, &TypeExpr) {
    match ty {
        TypeExpr::Pi(binder, domain, codomain, _) => (&binder.name, domain, codomain),
        other => panic!("expected a dependent function type, got {other:?}"),
    }
}

fn is_path(ty: &TypeExpr, name: &str) -> bool {
    matches!(ty, TypeExpr::Path(p) if p.item_name().name == name)
}

/// The rendered message of each parse error `source` raises.
fn parse_messages(source: &str) -> Vec<String> {
    let (_, errors) = parse(source);
    errors
        .iter()
        .map(|error| {
            let diagnostic = error.to_diagnostic();
            let helps: Vec<String> = diagnostic
                .suggestions
                .iter()
                .map(|s| s.message.clone())
                .collect();
            format!(
                "{} [{}] {}",
                diagnostic.message,
                error.kind.code(),
                helps.join(" | ")
            )
        })
        .collect()
}

// 7.10.26n AC1: a parameter's annotation reads `(n: Nat) -> P<n>` as a `Pi`.
#[test]
fn a_binder_before_an_arrow_is_a_dependent_function_type() {
    let ty = first_param_type("fn f(h: (n: Nat) -> P<n>, k: Nat) -> Nat { k }");
    let (binder, domain, codomain) = pi_parts(&ty);
    assert_eq!(binder, "n");
    assert!(is_path(domain, "Nat"));
    assert!(!is_path(domain, "Bool") && !is_path(codomain, "P"));
    assert!(matches!(codomain, TypeExpr::App(..)), "{codomain:?}");
}

// The refusal helper below is itself asserted: a wrong expected message
// panics, so the three refusal tests cannot pass vacuously.
#[test]
#[should_panic]
fn assert_first_error_refuses_a_wrong_message() {
    assert_first_error(
        "fn f(h: (n: Nat), k: Nat) -> Nat { k }",
        "not the message",
        "",
    );
}

// 7.10.26n AC1: the span runs from `(` to the end of the codomain.
#[test]
fn the_span_covers_binder_and_codomain() {
    let source = "fn f(h: (n: Nat) -> Eq<Nat, n, n>) -> Nat { 0 }";
    let ty = first_param_type(source);
    let start = source.find("(n").expect("present") as u32;
    let end = source.find(") -> Nat").expect("present") as u32;
    assert_eq!((ty.span().start, ty.span().end), (start, end));
}

// 7.10.26n AC1: the arrow stays right-associative — the codomain holds the
// rest, a nested binder included.
#[test]
fn the_arrow_is_right_associative() {
    let ty = first_param_type("fn f(h: (n: Nat) -> (m: Nat) -> A -> B) -> Nat { 0 }");
    let (outer, _, codomain) = pi_parts(&ty);
    let (inner, _, innermost) = pi_parts(codomain);
    assert_eq!((outer, inner), ("n", "m"));
    assert!(matches!(innermost, TypeExpr::Arrow(a, b, _) if is_path(a, "A") && is_path(b, "B")));
}

// 7.10.26n AC1: `(n: Nat) -> n == n` reads `==` sugar in its codomain, as a
// parameter's annotation and as a return type alike.
#[test]
fn the_codomain_reads_eq_sugar_in_both_positions() {
    let as_param = first_param_type("fn f(h: (n: Nat) -> n == n) -> Nat { 0 }");
    assert!(matches!(pi_parts(&as_param).2, TypeExpr::Eq(..)));
    let file = parse_ok("fn f() -> (n: Nat) -> n == n { |n: Nat| refl }");
    let as_return = unwrap_fn(&file).return_type.clone().expect("a return type");
    assert!(matches!(pi_parts(&as_return).2, TypeExpr::Eq(..)));
}

// 7.10.26n AC1 (D1): the statement lookahead does not read a leading binder
// as an equation.
#[test]
fn the_statement_lookahead_skips_a_leading_binder() {
    use crate::parser::types::annotation_is_eq_sugar;
    use crate::token::TokenKind::{Colon, EqEq, Ident, LParen, RParen};
    assert!(!annotation_is_eq_sugar(&[
        LParen, Ident, Colon, Ident, RParen, EqEq
    ]));
    assert!(annotation_is_eq_sugar(&[
        LParen, Ident, RParen, EqEq, Ident
    ]));
}

// 7.10.26n AC1: a binder's parentheses are a primary operand of `+`.
#[test]
fn a_binder_is_a_sum_operand() {
    let ty = first_param_type("fn f(h: A + (x: N) -> B) -> Nat { 0 }");
    let TypeExpr::Sum(left, right, _) = &ty else {
        panic!("expected a sum, got {ty:?}");
    };
    assert!(is_path(left, "A"));
    assert_eq!(pi_parts(right).0, "x");
}

// 7.10.26n AC1: the shapes the lookahead must keep parse as before.
#[test]
fn paren_tuple_and_forall_types_are_unchanged() {
    let tuple = first_param_type("fn g(f: (Nat, Nat) -> Nat) -> Nat { 0 }");
    assert!(
        matches!(&tuple, TypeExpr::Arrow(domain, _, _) if matches!(**domain, TypeExpr::Product(..)))
    );
    let paren = first_param_type("fn g(f: (Nat) -> Nat) -> Nat { 0 }");
    assert!(
        matches!(&paren, TypeExpr::Arrow(domain, _, _) if matches!(**domain, TypeExpr::Paren(..)))
    );
    let forall = first_param_type("fn g(f: forall T. T -> T) -> Nat { 0 }");
    assert!(matches!(forall, TypeExpr::Forall(..)));
}

/// `source`'s first parse error opens with `message` and code, and its
/// rendered help lines contain `help`.
fn assert_first_error(source: &str, message: &str, help: &str) {
    let messages = parse_messages(source);
    assert!(messages[0].starts_with(message), "{messages:?}");
    assert!(messages[0].contains(help), "{messages:?}");
}

// 7.10.26n AC1 (D1): a binder with no arrow is refused, E0110.
#[test]
fn a_binder_without_an_arrow_is_refused() {
    assert_first_error(
        "fn f(h: (n: Nat), k: Nat) -> Nat { k }",
        "expected `->` after a dependent binder, found `,` [E0110]",
        "",
    );
}

// 7.10.26n AC1 (D1): a telescope is refused at its `,`, E0110, with a help
// line naming the nested spelling.
#[test]
fn a_telescope_is_refused_at_its_comma() {
    assert_first_error(
        "fn f(h: (x: Nat, y: Nat) -> Nat) -> Nat { 0 }",
        "expected `)`, found `,` [E0110]",
        "`(x: A) -> (y: B) -> C`",
    );
}

// 7.10.26n AC1 (D1): `forall n: Nat.` keeps its error and gains a note
// pointing at the value binder's spelling.
#[test]
fn a_forall_over_a_value_points_at_the_binder() {
    assert_first_error(
        "fn f(h: forall n: Nat. Eq<Nat, n, n>) -> Nat { 0 }",
        "expected `.`, found `:` [E0110]",
        "a value binder is written `(n: Nat) -> …`",
    );
}
