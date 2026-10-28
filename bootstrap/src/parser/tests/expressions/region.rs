//! `region` block parsing tests — ADR 9.10.26o AC 2.

use crate::ast::*;
use crate::parser::tests::{parse_expr_ok, parse_has_errors};

// 9.10.26o AC2: `region { e }` is an expression whose body is the block.
#[test]
fn a_region_parses_to_its_block() {
    match parse_expr_ok("region { 42 }") {
        Expr::Region(body, _) => assert!(matches!(*body, Expr::Block(_, _, _)), "{body:?}"),
        other => panic!("Expected Region, got {other:?}"),
    }
}

// 9.10.26o AC2: a region is a value — it can be bound and nested.
#[test]
fn a_region_nests_and_binds() {
    let value = parse_expr_ok("region { region { 1 } } + 1");
    let Expr::Binary(left, _, _, _) = &value else {
        panic!("Expected a sum, got {value:?}");
    };
    let Expr::Region(outer, _) = &**left else {
        panic!("Expected Region, got {left:?}");
    };
    let Expr::Block(_, Some(tail), _) = &**outer else {
        panic!("Expected a block body, got {outer:?}");
    };
    assert!(matches!(**tail, Expr::Region(_, _)), "{tail:?}");
}

// 9.10.26o AC2: `region` is reserved, so it is no identifier.
#[test]
fn region_is_not_an_identifier() {
    assert!(parse_has_errors("let region = 1; region"));
    assert!(parse_has_errors("region"));
}
