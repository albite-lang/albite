//! `type Le(n: Nat, m: Nat): Prop = …` (ADR 4.10.26i D5, AC 2): term
//! parameters and a sort annotation between a type's name and its `=`.

use super::*;

fn unwrap_alias(file: &SourceFile) -> &TypeAlias {
    match &file.items[0] {
        Item::TypeAlias(alias) => alias,
        other => panic!("expected a type alias, got {other:?}"),
    }
}

fn param_name(param: &Param) -> &str {
    match &param.pattern {
        Pattern::Var(ident) => &ident.name,
        other => panic!("expected a variable pattern, got {other:?}"),
    }
}

const LE: &str = "type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m";

// 4.10.26i AC2: the parameters and the sort are read, and the body is the alias.
#[test]
fn a_family_reads_its_parameters_and_sort() {
    let file = parse_ok(LE);
    let alias = unwrap_alias(&file);
    assert_eq!(alias.name.name, "Le");
    let names: Vec<&str> = alias.value_params.iter().map(param_name).collect();
    assert_eq!(names, ["n", "m"]);
    assert_eq!(alias.sort, Some(SortAnnot::Prop));
    assert!(alias.type_params.is_empty());
    assert!(matches!(alias.ty, TypeExpr::Exists(..)), "{:?}", alias.ty);
}

// 4.10.26i AC2: `Type` is read in the sort position as an identifier.
#[test]
fn type_in_the_sort_position_is_the_data_sort() {
    let file = parse_ok("type Pair(n: Nat): Type = Nat");
    assert_eq!(unwrap_alias(&file).sort, Some(SortAnnot::Type));
}

// 4.10.26i AC2: a sort without parameters is still an alias, never variants.
#[test]
fn a_sorted_definition_without_parameters_is_an_alias() {
    let file = parse_ok("type Top: Prop = Void");
    let alias = unwrap_alias(&file);
    assert!(alias.value_params.is_empty());
    assert_eq!(alias.sort, Some(SortAnnot::Prop));
}

// 4.10.26i AC2: a plain alias has neither, so nothing changed for it.
#[test]
fn a_plain_alias_has_no_parameters_or_sort() {
    let file = parse_ok("type Count = Nat");
    let alias = unwrap_alias(&file);
    assert!(alias.value_params.is_empty());
    assert_eq!(alias.sort, None);
}

// 4.10.26i AC2: a parameter list without `: Prop`/`: Type` is refused, and so
// is any other word in the sort position.
#[test]
fn a_parameter_list_requires_a_sort() {
    assert!(parse_has_errors("type Le(n: Nat, m: Nat) = Void"));
    assert!(parse_has_errors("type Le(n: Nat): Nat = Void"));
}

// 4.10.26i P0.3: a use arrives as an application over two simple paths.
#[test]
fn a_family_use_parses_as_an_application_of_paths() {
    let file = parse_ok("fn f(n: Nat) -> Le<n, n> { sorry }");
    let ret = unwrap_fn(&file).return_type.clone().expect("a return type");
    let TypeExpr::App(_, args, _) = ret else {
        panic!("expected an application, got {ret:?}");
    };
    assert_eq!(args.len(), 2);
    assert!(args
        .iter()
        .all(|arg| matches!(arg, TypeExpr::Path(p) if p.is_simple())));
}

// ── ADR 6.10.26d D2: a family argument may be a term ────────────────────────

use crate::parser::types::type_argument_is_term;
use crate::token::TokenKind as T;

// 6.10.26d AC1 (D2): the lookahead, as a pure function over each spelling.
#[test]
fn the_lookahead_reads_a_term_only_where_a_type_cannot_be() {
    let term = [
        vec![T::IntLiteral, T::Comma],                         // `0,`
        vec![T::Ident, T::Plus, T::IntLiteral, T::Comma],      // `n + 1,`
        vec![T::Ident, T::Star, T::IntLiteral, T::Gt],         // `n * 2>`
        vec![T::Ident, T::Minus, T::Ident, T::Comma],          // `n - m,`
        vec![T::Ident, T::LParen, T::Ident, T::RParen, T::Gt], // `f(n)>`
        vec![
            T::LParen,
            T::Ident,
            T::Plus,
            T::IntLiteral,
            T::RParen,
            T::Comma,
        ], // `(n + 1),`
        vec![T::True, T::Gt],
        // `(a, 1)>` — a `,` inside parentheses does not end the argument
        vec![
            T::LParen,
            T::Ident,
            T::Comma,
            T::IntLiteral,
            T::RParen,
            T::Gt,
        ],
        vec![T::StringLiteral, T::Gt],
    ];
    for tokens in term {
        assert!(type_argument_is_term(tokens.clone()), "{tokens:?}");
    }
    let ty = [
        vec![T::Ident, T::Star, T::Ident, T::Comma], // `n * m,` — a product
        vec![T::Ident, T::Plus, T::Ident, T::Gt],    // `A + B>` — a sum
        vec![T::IntLiteral, T::EqEq, T::IntLiteral, T::Gt], // `0 == 1>` — sugar
        vec![T::Ident, T::Lt, T::IntLiteral, T::Gt, T::Gt], // `F<0>>` — nested
        vec![T::Ident, T::Comma, T::IntLiteral, T::Gt], // `n, 0>` — stops at `,`
        vec![T::RParen, T::IntLiteral],              // unbalanced: stops
        vec![T::Eof, T::IntLiteral],
    ];
    for tokens in ty {
        assert!(!type_argument_is_term(tokens.clone()), "{tokens:?}");
    }
}

/// The arguments of `ty`, written as a parameter's annotation. (A return
/// type is a statement, whose own `==` lookahead reads `Foo<0 == 1, A>` whole.)
fn annotation_arguments(ty: &str) -> Vec<TypeExpr> {
    let file = parse_ok(&format!("fn f(x: {ty}) -> Nat {{ 0 }}"));
    match unwrap_first_param_type(&file).clone() {
        TypeExpr::App(_, args, _) => args,
        other => panic!("expected an application, got {other:?}"),
    }
}

// 6.10.26d AC1 (D2): each term spelling becomes a `TypeExpr::Term`.
#[test]
fn a_term_argument_parses_as_a_term() {
    for ret in [
        "Le<0, n>",
        "Le<n + 1, m>",
        "Le<n * 2, m>",
        "Le<f(n), m>",
        "Le<(n + 1), m>",
    ] {
        let args = annotation_arguments(ret);
        assert!(matches!(args[0], TypeExpr::Term(_)), "{ret}: {:?}", args[0]);
        assert!(matches!(args[1], TypeExpr::Path(_)), "{ret}: {:?}", args[1]);
    }
}

// 6.10.26d AC1 (D2): no type that parsed before changes parse.
#[test]
fn a_type_argument_parses_as_before() {
    assert!(matches!(
        annotation_arguments("Option<A + B>")[0],
        TypeExpr::Sum(..)
    ));
    assert!(matches!(
        annotation_arguments("Foo<A * B>")[0],
        TypeExpr::Product(..)
    ));
    assert!(matches!(
        annotation_arguments("Le<n * m, m>")[0],
        TypeExpr::Product(..)
    ));
    // A last `==` argument reads its `>` as a comparison, before this ADR as
    // after it, so the sugar is pinned before a `,`.
    assert!(matches!(
        annotation_arguments("Foo<0 == 1, A>")[0],
        TypeExpr::Eq(..)
    ));
    assert!(matches!(
        annotation_arguments("Foo<n == m, A>")[0],
        TypeExpr::Eq(..)
    ));
    assert!(matches!(
        annotation_arguments("Foo<List<Nat>>")[0],
        TypeExpr::App(..)
    ));
}
