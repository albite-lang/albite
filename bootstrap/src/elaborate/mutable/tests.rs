//! `lower_joins` over injected blocks, and the three refusals through the
//! elaborator (ADR 13.10.26r AC 2). Values are asserted by the tier-5 suite
//! (`src/compiler/test_let_mut.tg`); here the lowered shape is.

use super::{lower_joins, needs_lowering, BindingFacts, JoinSource, LoweredBlock, OuterScope};
use crate::ast::{Expr, Pattern, Stmt};
use crate::elaborate::error::{ElabErrorKind, MutableErrorKind};
use crate::elaborate::tests::{elab_err, elab_ok};

/// The body block of `fn probe() -> Nat { <body> }`.
fn body_of(body: &str) -> (Vec<Stmt>, Option<Expr>) {
    let source = format!("fn probe(c: Bool, p: Nat) -> Nat {{ {body} }}");
    let (file, errors) = crate::parser::parse(&source);
    assert!(errors.is_empty(), "fixture must parse: {errors:?}");
    let crate::ast::Item::Function(function) = &file.items[0] else {
        panic!("a function");
    };
    match &function.body {
        Expr::Block(stmts, tail, _) => (stmts.clone(), tail.as_deref().cloned()),
        other => panic!("a block body, got {other:?}"),
    }
}

fn immutable_parameters(name: &str) -> Option<BindingFacts> {
    matches!(name, "c" | "p").then_some(BindingFacts {
        mutable: false,
        region_depth: 0,
        closure_depth: 0,
    })
}

fn lowered(body: &str) -> LoweredBlock {
    let (stmts, tail) = body_of(body);
    let scope = OuterScope {
        lookup: &immutable_parameters,
        region_depth: 0,
        closure_depth: 0,
    };
    lower_joins(&stmts, tail.as_ref(), &scope, 0).expect("the block lowers")
}

fn refusal(body: &str, scope: &OuterScope<'_>) -> MutableErrorKind {
    let (stmts, tail) = body_of(body);
    lower_joins(&stmts, tail.as_ref(), scope, 0)
        .expect_err("the block is refused")
        .kind
}

fn binders(pattern: &Pattern) -> Vec<String> {
    crate::elaborate::mutable::pattern_names(pattern)
}

/// The `let`s a lowered block binds, by their pattern names.
fn let_names(block: &LoweredBlock) -> Vec<Vec<String>> {
    block
        .stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Let(pattern, _, _, _) => Some(binders(pattern)),
            _ => None,
        })
        .collect()
}

fn joined_sources(block: &LoweredBlock) -> Vec<String> {
    let mut sources: Vec<String> = block
        .joins
        .values()
        .filter_map(|source| match source {
            JoinSource::Binding(name) => Some(name.clone()),
            JoinSource::Value(_) => None,
        })
        .collect();
    sources.sort();
    sources
}

// 13.10.26r AC2: a rebinding stays an assignment, and nothing is joined.
#[test]
fn a_rebinding_is_left_for_the_elaborator() {
    let block = lowered("let mut x = 1; x = x + 1; x");
    assert!(matches!(block.stmts[1], Stmt::Assign(ref name, _, _) if name == "x"));
    assert!(block.joins.is_empty());
}

// 13.10.26r AC2: a join of one binding in an `if`: snapshot, unpack, rebind.
#[test]
fn an_if_that_assigns_one_binding_joins_it() {
    let block = lowered("let mut x = 1; if c { x = 2; } else { }; x");
    assert_eq!(joined_sources(&block), ["x"]);
    let lets = let_names(&block);
    assert!(lets[0][0].starts_with("x#"), "the snapshot: {lets:?}");
    assert_eq!(lets[1].len(), 1, "one binding, no tuple: {lets:?}");
    assert!(matches!(block.stmts.last(), Some(Stmt::Assign(name, _, _)) if name == "x"));
}

// 13.10.26r D3: an arm that assigns a `let mut` declared inside itself is a
// plain rebinding; only a binding declared outside the construct joins.
#[test]
fn an_arm_local_assignment_is_not_joined() {
    let block = lowered("if c { let mut z = 1; z = 2; } else { }; 0");
    assert!(block.joins.is_empty(), "{:?}", block.joins);
    elab_ok("fn f(c: Bool) -> Nat { if c { let mut z = 1; z = 2; } else { }; 0 }");
}

// 13.10.26r AC2: two bindings in a `match`, one assigned per arm.
#[test]
fn a_match_that_assigns_two_bindings_joins_both() {
    let block = lowered(
        "let mut x = 1; let mut y = 2; match p { 0 => { x = 5; }, _ => { y = 6; } }; x + y",
    );
    assert_eq!(joined_sources(&block), ["x", "y"]);
    let unpack = let_names(&block).into_iter().find(|names| names.len() == 2);
    assert!(unpack.is_some(), "a two-binding tuple");
}

// 13.10.26r AC2: a join in a `let`'s right side keeps the construct's value.
#[test]
fn a_join_in_a_let_keeps_its_value() {
    let block = lowered("let mut x = 1; let v = if c { x = 4; 100 } else { 200 }; v + x");
    let values = block
        .joins
        .values()
        .filter(|source| matches!(source, JoinSource::Value(_)))
        .count();
    assert_eq!(values, 1);
    let user_let = block.stmts.iter().find(|stmt| {
        matches!(stmt, Stmt::Let(Pattern::Var(ident), _, Expr::Path(_), _) if ident.name == "v")
    });
    assert!(user_let.is_some(), "`let v` reads the joined value");
}

// 13.10.26r AC2: an `if` nested in an arm joins to the arm, then the arm's
// construct joins to the block.
#[test]
fn a_nested_join_joins_twice() {
    let block = lowered("let mut x = 1; if c { if c { x = 2; } else { }; } else { }; x");
    let x_joins = block
        .joins
        .values()
        .filter(|source| matches!(source, JoinSource::Binding(name) if name == "x"))
        .count();
    assert_eq!(x_joins, 2);
}

// 13.10.26r AC2: an arm that assigns nothing passes the current value through
// (its snapshot), and a missing `if let` `else` gets that pass-through arm.
#[test]
fn an_arm_that_assigns_nothing_passes_the_value_through() {
    let block = lowered("let mut x = 1; if let 0 = p { x = 2; }; x");
    let joined = block.stmts.iter().find_map(|stmt| match stmt {
        Stmt::Let(_, _, Expr::IfLet(_, _, _, otherwise, _), _) => Some(otherwise.clone()),
        _ => None,
    });
    let otherwise = joined
        .expect("the joined `if let`")
        .expect("a synthesised else");
    let Expr::Block(_, Some(tail), _) = *otherwise else {
        panic!("a pass-through block");
    };
    assert!(matches!(*tail, Expr::Path(ref path) if path.segments[0].name.starts_with("x#")));
}

// 13.10.26r AC2 (hygiene): an arm's pairing reads a fresh name, never the
// source one, so a later `let x` in the arm cannot change what it hands back.
#[test]
fn an_arm_pairs_through_a_fresh_name() {
    let block = lowered("let mut x = 1; if c { x = 2; let x = 99; } else { }; x");
    let then_tail = block.stmts.iter().find_map(|stmt| match stmt {
        Stmt::Let(_, _, Expr::If(_, then, _, _), _) => match &**then {
            Expr::Block(_, Some(tail), _) => Some((**tail).clone()),
            _ => None,
        },
        _ => None,
    });
    let tail = then_tail.expect("the joined `if`");
    assert!(matches!(tail, Expr::Path(ref path) if path.segments[0].name.starts_with("x#")));
}

// 13.10.26r AC2 (hygiene): a `match` arm whose pattern binds `x` makes the
// arm's `x` the pattern's, so assigning it is refused, and an arm that does
// not assign passes the outer `x` through.
#[test]
fn a_pattern_variable_shadows_the_mut_binding() {
    let kind = refusal(
        "let mut x = 1; match p { x => { x = 2; } }; x",
        &OuterScope::EMPTY,
    );
    assert_eq!(kind, MutableErrorKind::NotMut { name: "x".into() });
    lowered("let mut x = 1; match p { 0 => { x = 3; }, x => { } }; x");
}

// 13.10.26r AC2: not `mut` — a parameter, a plain `let`, and an immutable
// `let` shadowing a `let mut`.
#[test]
fn an_assignment_to_an_immutable_binding_is_refused() {
    let scope = OuterScope {
        lookup: &immutable_parameters,
        region_depth: 0,
        closure_depth: 0,
    };
    for (body, name) in [
        ("p = 1; p", "p"),
        ("let x = 1; x = 2; x", "x"),
        ("let mut x = 1; let x = x; x = 2; x", "x"),
    ] {
        let expected = MutableErrorKind::NotMut { name: name.into() };
        assert_eq!(refusal(body, &scope), expected, "{body}");
    }
}

/// An outer `mut` `x` declared at depth 0 in regions and closures.
fn outer_mutable(name: &str) -> Option<BindingFacts> {
    (name == "x").then_some(BindingFacts {
        mutable: true,
        region_depth: 0,
        closure_depth: 0,
    })
}

// 13.10.26r AC2: an outer `mut` assigned in a block no join reaches is not
// joined; in a lambda it is E0105, in a region E0106 (never E0104).
#[test]
fn an_outer_assignment_is_refused_by_where_it_sits() {
    let at = |region_depth, closure_depth| OuterScope {
        lookup: &outer_mutable,
        region_depth,
        closure_depth,
    };
    let not_joined = MutableErrorKind::NotJoined {
        name: "x".into(),
        exit: None,
    };
    assert_eq!(refusal("x = 2; 3", &at(0, 0)), not_joined);
    assert_eq!(
        refusal("x = 2; 3", &at(0, 1)),
        MutableErrorKind::Captured { name: "x".into() }
    );
    assert_eq!(
        refusal("x = 2; 3", &at(1, 0)),
        MutableErrorKind::InRegion { name: "x".into() }
    );
}

// 13.10.26r AC2: a joined arm that can leave through `return` or `?` cannot
// end by pairing, and is refused naming the exit.
#[test]
fn a_joined_arm_with_an_exit_is_refused() {
    for (body, exit) in [
        (
            "let mut x = 1; if c { x = 2; return 0; } else { }; x",
            "return",
        ),
        ("let mut x = 1; if c { x = 2; f()?; } else { }; x", "?"),
    ] {
        let kind = refusal(body, &OuterScope::EMPTY);
        let expected = MutableErrorKind::NotJoined {
            name: "x".into(),
            exit: Some(exit.into()),
        };
        assert_eq!(kind, expected, "{body}");
    }
}

// 13.10.26r AC2: `needs_lowering` finds an assignment in every join position
// and none outside one.
#[test]
fn needs_lowering_reads_join_positions_only() {
    let (stmts, tail) = body_of("let mut x = 1; x");
    assert!(!needs_lowering(&stmts, tail.as_ref()));
    let (stmts, tail) = body_of("let mut x = 1; if c { x = 2; 1 } else { 2 }");
    assert!(needs_lowering(&stmts, tail.as_ref()));
    let (stmts, tail) = body_of("let mut x = 1; let y = match p { _ => { x = 1; 2 } }; y");
    assert!(needs_lowering(&stmts, tail.as_ref()));
    let (stmts, tail) = body_of("let mut x = 1; f({ x = 2; 3 })");
    assert!(!needs_lowering(&stmts, tail.as_ref()));
}

fn mutable_kind(source: &str) -> MutableErrorKind {
    let errors = elab_err(source);
    errors
        .iter()
        .find_map(|error| match &error.kind {
            ElabErrorKind::Mutable(kind) => Some(kind.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a mutable-binding refusal: {errors:?}"))
}

// 13.10.26r AC2, through the elaborator: each refusal with its reason and name.
#[test]
fn the_elaborator_raises_each_refusal() {
    let id = "type Result<T, E> = Ok(T) | Err(E)\nfn id(n: Nat) -> Nat { n }\n";
    let cases = [
        (
            "fn f(p: Nat) -> Nat { p = 1; p }",
            MutableErrorKind::NotMut { name: "p".into() },
        ),
        (
            "fn f() -> Nat { let mut x = 1; let y = id({ x = 2; 3 }); x + y }",
            MutableErrorKind::NotJoined {
                name: "x".into(),
                exit: None,
            },
        ),
        (
            "fn f() -> Nat { let mut x = 1; let g = |y: Nat| x + y; g(1) }",
            MutableErrorKind::Captured { name: "x".into() },
        ),
        (
            "fn f() -> Nat { let mut x = 1; let g = |y: Nat| { x = y; y }; g(1) }",
            MutableErrorKind::Captured { name: "x".into() },
        ),
        (
            "fn f() -> Nat { let mut x = 1; region { x = 5; 1 } }",
            MutableErrorKind::InRegion { name: "x".into() },
        ),
        (
            "fn f() -> Nat { let mut x = 1; let y: Result<Nat, Nat> = try { x = 2; Ok(1) }; x }",
            MutableErrorKind::NotJoined {
                name: "x".into(),
                exit: None,
            },
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(mutable_kind(&format!("{id}{source}")), expected, "{source}");
    }
}

// 13.10.26r AC2: accepted — a lambda's own `let mut`, a lambda over an
// immutable shadow of a `mut`, and a region assigning its own `mut`, inside
// an `if` in the body too.
#[test]
fn the_elaborator_accepts_what_no_refusal_covers() {
    elab_ok("fn f() -> Nat { let g = |y: Nat| { let mut z = y; z = z + 1; z }; g(4) }");
    elab_ok("fn f() -> Nat { let mut x = 1; let x = x; let g = |y: Nat| x + y; g(1) }");
    elab_ok("fn f() -> Nat { region { let mut r = 1; if true { r = 2; }; r } }");
}

// 13.10.26r D3: an assignment is checked against the type the binding has.
#[test]
fn an_assignment_of_the_wrong_type_is_a_type_mismatch() {
    let errors = elab_err("fn f() -> Nat { let mut x = 1; x = true; x }");
    assert!(
        errors
            .iter()
            .any(|error| matches!(error.kind, ElabErrorKind::TypeMismatch { .. })),
        "{errors:?}"
    );
}

// 13.10.26r D3 ("the join is typed"): a `Nil` arm takes the join's type from
// the `let`'s annotation, the block's expected type, or the assigned binding.
#[test]
fn a_nil_arm_takes_the_joined_type() {
    let list = "type List<T> = Nil | Cons(T, List<T>)\n";
    elab_ok(&format!(
        "{list}fn f(c: Bool) -> List<Nat> {{ let mut n = 0; \
         let xs: List<Nat> = if c {{ n = 1; Nil }} else {{ Cons(5, Nil) }}; xs }}"
    ));
    elab_ok(&format!(
        "{list}fn f(c: Bool) -> List<Nat> {{ let mut n = 0; \
         if c {{ n = 1; Nil }} else {{ Cons(n, Nil) }} }}"
    ));
    elab_ok(&format!(
        "{list}fn f(c: Bool) -> List<Nat> {{ let mut n = 0; let mut xs: List<Nat> = Nil; \
         xs = if c {{ n = 1; Nil }} else {{ Cons(n, Nil) }}; xs }}"
    ));
}
