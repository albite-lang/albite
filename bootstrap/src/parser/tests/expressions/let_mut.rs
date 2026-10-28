//! `let mut` and assignment parsing tests — ADR 13.10.26r D2.

use crate::ast::*;
use crate::error::ParseErrorKind;
use crate::parser::tests::{parse_expr_ok, parse_has_errors};

fn block_stmts(source: &str) -> Vec<Stmt> {
    match parse_expr_ok(source) {
        Expr::Block(stmts, _, _) => stmts,
        other => panic!("Expected a block, got {other:?}"),
    }
}

fn parse_errors(source: &str) -> Vec<ParseErrorKind> {
    let wrapped = format!("fn probe() -> Nat {source}");
    let (_, errors) = crate::parser::parse(&wrapped);
    errors.into_iter().map(|error| error.kind).collect()
}

// 13.10.26r D2: `let mut x = e;` is its own statement, typed or not.
#[test]
fn let_mut_parses_with_and_without_an_annotation() {
    let stmts = block_stmts("{ let mut x = 1; let mut y: Nat = 2; x }");
    assert!(
        matches!(&stmts[0], Stmt::LetMut(name, None, Expr::IntLiteral(1, _), _) if name == "x")
    );
    assert!(
        matches!(&stmts[1], Stmt::LetMut(name, Some(_), Expr::IntLiteral(2, _), _) if name == "y")
    );
}

// 13.10.26r D2: `x = e;` is an assignment statement; `==` is not one.
#[test]
fn an_assignment_parses_as_a_statement() {
    let stmts = block_stmts("{ let mut x = 1; x = x + 1; x == 2 }");
    assert!(matches!(&stmts[1], Stmt::Assign(name, Expr::Binary(..), _) if name == "x"));
    assert_eq!(stmts.len(), 2);
}

// 13.10.26r D2: the assignment's span starts at its name and ends at its `;`.
#[test]
fn an_assignment_spans_its_name_to_its_semicolon() {
    let stmts = block_stmts("{ x = 5; 1 }");
    let Stmt::Assign(_, _, span) = &stmts[0] else {
        panic!("Expected an assignment, got {:?}", stmts[0]);
    };
    // `fn test() { ` precedes the block (12 bytes).
    assert_eq!((span.start, span.end), (14, 20));
}

// 13.10.26r D2: an assignment has no value, so it cannot be a block's tail.
#[test]
fn an_assignment_as_a_tail_says_to_add_a_semicolon() {
    let errors = parse_errors("{ let mut x = 1; x = 2 }");
    assert!(
        errors.iter().any(|kind| matches!(kind,
            ParseErrorKind::Expected(what) if what.contains("after an assignment"))),
        "{errors:?}"
    );
}

// 13.10.26r D2: `mut` declares one name; a pattern after it is refused.
#[test]
fn let_mut_with_a_pattern_names_the_rule() {
    let errors = parse_errors("{ let mut (a, b) = (1, 2); a }");
    assert!(
        errors.iter().any(|kind| matches!(kind,
            ParseErrorKind::Expected(what) if what.contains("a name after `let mut`"))),
        "{errors:?}"
    );
}

// 13.10.26r D2: `mut` is a keyword, so it is no identifier, and anywhere but
// after `let` it is a parse error.
#[test]
fn mut_is_a_keyword_only_after_let() {
    assert!(parse_has_errors("{ let mut = 1; 1 }"));
    assert!(parse_has_errors("{ mut x = 1; x }"));
    assert!(!parse_has_errors("{ let mut x = 1; x }"));
}
