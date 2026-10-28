//! `exists x: A. B` in type position (ADR 4.10.26h D5, AC 3): a type atom
//! whose body is parsed as a statement, so `==` sugar works inside it.

use super::*;

mod families;
mod function_type; // `(x: A) -> B` (ADR 7.10.26n)

fn return_type(source: &str) -> TypeExpr {
    let file = parse_ok(source);
    unwrap_fn(&file)
        .return_type
        .clone()
        .expect("function has a return type")
}

/// `(binder, binder type, body)` of an `exists`.
fn exists_parts(ty: &TypeExpr) -> (&str, &TypeExpr, &TypeExpr) {
    match ty {
        TypeExpr::Exists(binder, binder_ty, body, _) => (&binder.name, binder_ty, body),
        other => panic!("expected an `exists`, got {other:?}"),
    }
}

// 4.10.26h AC3: the body reads `n + k == m` as statement sugar.
#[test]
fn exists_body_reads_statement_sugar() {
    let ty = return_type("fn le(n: Nat, m: Nat) -> exists k: Nat. n + k == m { (0, refl) }");
    let (binder, binder_ty, body) = exists_parts(&ty);
    assert_eq!(binder, "k");
    assert!(matches!(binder_ty, TypeExpr::Path(p) if p.item_name().name == "Nat"));
    assert!(matches!(body, TypeExpr::Eq(..)), "{body:?}");
}

// 4.10.26h AC3: the span runs from `exists` to the end of the body.
#[test]
fn exists_span_covers_binder_and_body() {
    let source = "fn f(n: Nat) -> exists k: Nat. Eq<Nat, k, n> { (n, refl) }";
    let ty = return_type(source);
    let start = source.find("exists").expect("present") as u32;
    let end = source.find(" {").expect("present") as u32;
    assert_eq!((ty.span().start, ty.span().end), (start, end));
}

// 4.10.26h AC3: `exists` nests, the inner one in the outer one's body.
#[test]
fn nested_exists_parses() {
    let ty =
        return_type("fn f(n: Nat) -> exists j: Nat. exists k: Nat. j + k == n { (0, (n, refl)) }");
    let (outer, _, body) = exists_parts(&ty);
    let (inner, _, innermost) = exists_parts(body);
    assert_eq!((outer, inner), ("j", "k"));
    assert!(matches!(innermost, TypeExpr::Eq(..)));
}

// 4.10.26h AC3: `exists` is a type atom, so a `let` annotation parses it too.
#[test]
fn exists_parses_in_a_let_annotation() {
    let file = parse_ok("fn f() -> Unit { let _p: exists k: Nat. Eq<Nat, k, 0> = (0, refl); () }");
    let Expr::Block(stmts, _, _) = &unwrap_fn(&file).body else {
        panic!("expected a block body");
    };
    let Stmt::Let(_, Some(annotation), _, _) = &stmts[0] else {
        panic!("expected an annotated let, got {:?}", stmts[0]);
    };
    assert_eq!(exists_parts(annotation).0, "k");
}

// 4.10.26h AC3: a malformed `exists` is a parse error, not a silent type.
#[test]
fn exists_without_its_colon_or_dot_is_refused() {
    let (_, missing_colon) = parse("fn f() -> exists k Nat. Eq<Nat, k, k> { 0 }");
    assert!(!missing_colon.is_empty());
    let (_, missing_dot) = parse("fn f() -> exists k: Nat Eq<Nat, k, k> { 0 }");
    assert!(!missing_dot.is_empty());
}

// 6.10.26h D1: a type parameter's bound is its kind, one `TypeExpr`; a plain
// parameter keeps an empty list.
#[test]
fn a_type_parameter_bound_is_parsed_as_its_kind() {
    let file = parse_ok("fn f<T, P: Nat -> Prop>(n: Nat) -> Nat { n }");
    let params = &unwrap_fn(&file).type_params;
    assert!(params[0].bounds.is_empty());
    assert!(matches!(
        params[1].bounds.as_slice(),
        [TypeExpr::Arrow(domain, codomain, _)]
            if matches!(**domain, TypeExpr::Path(_)) && matches!(**codomain, TypeExpr::Prop(_))
    ));
}

// 6.10.26h D5: `f::<T>` after a path is a turbofish, not a path segment.
#[test]
fn a_turbofish_after_a_path_is_a_type_application() {
    let expr = parse_expr_ok("pred_transport::<IsZero>(0)");
    let Expr::App(callee, _, _) = expr else {
        panic!("expected an application, got {expr:?}");
    };
    assert!(matches!(*callee, Expr::TypeApp(ref base, ref args, _)
        if matches!(**base, Expr::Path(ref p) if p.segments.len() == 1) && args.len() == 1));
    // A qualified path still reads its segments.
    assert!(matches!(parse_expr_ok("m::f"), Expr::Path(ref p) if p.segments.len() == 2));
}
