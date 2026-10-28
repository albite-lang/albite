//! `==` sugar with a leading identifier in statement position (ADR 30.9.26d
//! D10, AC 7): a return type or a theorem's or axiom's proposition may be
//! `n + 0 == n`, while every other type keeps today's parser.

use super::*;
use crate::parser::types::annotation_is_eq_sugar;
use crate::token::TokenKind::{self, *};

fn return_type(source: &str) -> TypeExpr {
    let file = parse_ok(source);
    unwrap_fn(&file)
        .return_type
        .clone()
        .expect("function has a return type")
}

fn is_sugar(ty: &TypeExpr) -> bool {
    matches!(ty, TypeExpr::Eq(..))
}

// ── The lookahead, over token slices

// 30.9.26d AC7: an `==` at depth 0 before the body is sugar.
#[test]
fn lookahead_finds_eq_before_the_body() {
    assert!(annotation_is_eq_sugar(&[
        Ident, Plus, IntLiteral, EqEq, Ident, LBrace
    ]));
    assert!(annotation_is_eq_sugar(&[
        LParen, Ident, Plus, IntLiteral, RParen, EqEq, Ident, LBrace
    ]));
}

// 30.9.26d AC7: a type with no `==` before its terminator is not sugar.
#[test]
fn lookahead_stops_at_each_terminator() {
    let terminators: [TokenKind; 6] = [LBrace, Semi, FatArrow, Where, Eof, Fn];
    for terminator in terminators {
        let tokens = [Ident, Plus, Ident, terminator, Ident, EqEq, Ident];
        assert!(
            !annotation_is_eq_sugar(&tokens),
            "{terminator:?} did not stop the scan"
        );
    }
}

// 30.9.26d AC7: an `==` inside brackets is an operand's, not the statement's.
#[test]
fn lookahead_ignores_a_nested_eq() {
    assert!(!annotation_is_eq_sugar(&[
        Ident, LParen, Ident, EqEq, Ident, RParen, LBrace
    ]));
    // An unbalanced close ends the scan rather than underflowing.
    assert!(!annotation_is_eq_sugar(&[Ident, RParen, EqEq, Ident]));
    // A top-level `{` is the body and stops the scan; it never nests, so the
    // `==` after its `}` belongs to the body, not the statement.
    assert!(!annotation_is_eq_sugar(&[
        Ident, LBrace, RBrace, EqEq, Ident
    ]));
    // A `{ }` inside brackets (a closure body) nests and closes again, so the
    // `==` after the brackets is still the statement's.
    assert!(annotation_is_eq_sugar(&[
        LParen, LBrace, RBrace, RParen, EqEq, Ident, LBrace
    ]));
}

// 30.9.26d AC7: a `|` or an `as` inside an operand is scanned past.
#[test]
fn lookahead_scans_past_pipe_and_as() {
    assert!(annotation_is_eq_sugar(&[
        Ident, As, Ident, EqEq, Ident, LBrace
    ]));
    assert!(annotation_is_eq_sugar(&[
        LParen, Pipe, Ident, Pipe, Ident, RParen, EqEq, Ident, LBrace
    ]));
}

// 30.9.26d AC7: only an identifier or `(` start is scanned.
#[test]
fn lookahead_needs_an_identifier_or_paren_start() {
    assert!(!annotation_is_eq_sugar(&[
        IntLiteral, EqEq, IntLiteral, LBrace
    ]));
    assert!(!annotation_is_eq_sugar(&[]));
}

// ── Statements

// 30.9.26d AC7: the three leading-identifier spellings parse to `TypeExpr::Eq`.
#[test]
fn leading_identifier_statements_are_sugar() {
    assert!(is_sugar(&return_type(
        "fn f(n: Nat) -> n + 0 == n { refl }"
    )));
    assert!(is_sugar(&return_type(
        "fn f(n: Nat) -> (n + 0) == n { refl }"
    )));
    assert!(is_sugar(&return_type(
        "fn f(n: Nat) -> add(n, 0) + 1 == n { refl }"
    )));
}

// 30.9.26d AC7: theorem and axiom propositions take the sugar too.
#[test]
fn theorem_and_axiom_propositions_are_sugar() {
    let file = parse_ok(
        "theorem t(n: Nat): n + 0 == n { refl }\n\
         axiom a(n: Nat): n + 0 == n\n\
         fn after() -> Nat { 0 }",
    );
    assert!(matches!(&file.items[0], Item::Theorem(t) if is_sugar(&t.prop)));
    assert!(matches!(&file.items[1], Item::Axiom(a) if is_sugar(&a.prop)));
    assert!(matches!(&file.items[2], Item::Function(_)));
}

// 30.9.26d AC7: the ambiguity controls parse exactly as before.
#[test]
fn types_without_eq_are_unchanged() {
    assert!(matches!(
        return_type("fn f(a: Nat) -> A + B { a }"),
        TypeExpr::Sum(..)
    ));
    assert!(matches!(
        return_type("fn f(a: Nat) -> A * B -> C { a }"),
        TypeExpr::Arrow(..)
    ));
    assert!(matches!(
        return_type("fn f(a: Nat) -> Eq<Nat, a, a> { refl }"),
        TypeExpr::EqExplicit(..)
    ));
    assert!(!is_sugar(&return_type("fn f(a: Nat) -> Nat { a }")));
}

// 30.9.26d AC7: `n { refl }` is the operand and the body, never a struct
// literal — a CamelCase operand included — and named records work again after.
#[test]
fn the_body_brace_is_never_a_record() {
    let file = parse_ok("fn f(n: Nat) -> n == Zero { refl }\nfn g() -> Nat { Point { x: 1 } }");
    let sugared = unwrap_fn(&file);
    let Some(TypeExpr::Eq(_, rhs, _)) = &sugared.return_type else {
        panic!("return type is not sugar");
    };
    assert!(matches!(rhs.as_ref(), Expr::Path(_)));
    assert!(
        matches!(&file.items[1], Item::Function(g) if format!("{:?}", g.body).contains("NamedRecord"))
    );
}

// 30.9.26d AC7: a `let` annotation keeps today's parser, so is still refused.
#[test]
fn a_let_annotation_is_unchanged() {
    assert!(parse_has_errors("let x: n + 0 == n = refl; x"));
}

// ── Annotation position keeps the three original spellings

// 30.9.26d AC7: `ident == expr` and `ident(args) == expr` still parse in an
// annotation, through the type-first fallbacks the statement lookahead skips.
#[test]
fn annotation_sugar_keeps_the_type_first_fallbacks() {
    let file = parse_ok("fn f(p: n == m) -> Nat { 0 }");
    let TypeExpr::Eq(lhs, _, _) = unwrap_first_param_type(&file) else {
        panic!("`n == m` is not sugar");
    };
    assert!(matches!(lhs.as_ref(), Expr::Path(_)));

    let file = parse_ok("fn f(p: add(n, 0) == m) -> Nat { 0 }");
    let TypeExpr::Eq(lhs, _, _) = unwrap_first_param_type(&file) else {
        panic!("`add(n, 0) == m` is not sugar");
    };
    assert!(matches!(lhs.as_ref(), Expr::App(..)));
}

// 30.9.26d AC7: a compound type before `==` in an annotation is still refused.
#[test]
fn annotation_sugar_refuses_a_compound_left_side() {
    assert!(parse_has_errors("let x: A * B == m = 0; x"));
}
