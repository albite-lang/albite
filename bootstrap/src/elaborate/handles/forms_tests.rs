//! The use walk over every expression form (ADR 7.10.26t D4), built as AST
//! values: the forms a parsed fixture rarely reaches — an expression-level
//! `let`, `let … else`, `if let` and its chains, the proof forms, tuple and
//! or-patterns, a `match` whose every arm returns — each with the finding count
//! the rule gives it. Also the type wrappers "contains a handle" looks through.

use super::*;
use crate::ast::{
    Expr, FunctionDef, Ident, IfLetCondition, MatchArm, Motive, Path, Pattern, Stmt, TypeExpr,
    UnaryOp,
};
use crate::span::Span;

const HANDLE_API: &str = "pub type StringBuilder = MkStringBuilder(Nat)\n\
    fn push_str(sb: StringBuilder, s: String) -> StringBuilder { sb }\n\
    fn string_builder_len(sb: StringBuilder) -> Nat { 0 }\n\
    fn to_string(sb: StringBuilder) -> String { \"\" }\n\
    fn f(sb: StringBuilder) -> Nat { 0 }\n";

fn at() -> Span {
    Span::new(0, 0)
}

fn ident(name: &str) -> Ident {
    Ident::new(name, at())
}

fn var(name: &str) -> Expr {
    Expr::Path(Path::simple(ident(name)))
}

fn boxed(name: &str) -> Box<Expr> {
    Box::new(var(name))
}

fn unit() -> Box<Expr> {
    Box::new(Expr::Unit(at()))
}

fn call(function: &str, args: Vec<Expr>) -> Expr {
    Expr::App(boxed(function), args, at())
}

fn bind(name: &str) -> Pattern {
    Pattern::Var(ident(name))
}

fn arm(pattern: Pattern, guard: Option<Expr>, body: Expr) -> MatchArm {
    MatchArm {
        pattern,
        guard,
        body,
        span: at(),
    }
}

/// How many findings `f(sb: StringBuilder)` gets with `body` as its body.
fn findings_for(body: Expr) -> usize {
    let (parsed, errors) = crate::parser::parse(HANDLE_API);
    assert!(errors.is_empty(), "{errors:?}");
    let declarations = HandleDeclarations::from_items(&parsed.items);
    let mut function: FunctionDef = parsed
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(f) if f.name.name == "f" => Some(f),
            _ => None,
        })
        .expect("the fixture declares `f`");
    function.body = body;
    check_function(&function, &declarations, UsePolicy::AFFINE).len()
}

// 7.10.26t AC2: an expression-level `let` and a `let … else` consume their
// definition, and a tuple pattern binds no handle.
#[test]
fn a_let_and_a_let_else_consume_their_definition() {
    let let_then_reuse = Expr::Let(
        bind("x"),
        None,
        Box::new(call("push_str", vec![var("sb"), Expr::Unit(at())])),
        boxed("sb"),
        at(),
    );
    assert_eq!(findings_for(let_then_reuse), 1);
    let pair = Pattern::Tuple(vec![bind("a"), bind("b")], at());
    let let_else = Expr::LetElse(
        pair.clone(),
        None,
        boxed("sb"),
        Box::new(Expr::Return(None, at())),
        boxed("sb"),
        at(),
    );
    assert_eq!(findings_for(let_else), 1);
    let statement = Expr::Block(
        vec![Stmt::LetElse(
            pair,
            None,
            var("sb"),
            Expr::Return(None, at()),
            at(),
        )],
        Some(boxed("a")),
        at(),
    );
    assert_eq!(findings_for(statement), 0);
}

// 7.10.26t AC2: `if let` and an `if let` chain are alternatives like `if`; an
// or-pattern and a chain's bindings shadow nothing that is a handle.
#[test]
fn if_let_and_its_chains_take_the_maximum_of_their_arms() {
    let consume = || Box::new(call("to_string", vec![var("sb")]));
    let either = Pattern::Or(Box::new(bind("y")), Box::new(bind("y")), at());
    let if_let = Expr::IfLet(either, unit(), consume(), Some(consume()), at());
    assert_eq!(findings_for(if_let), 0);
    let conditions = vec![
        IfLetCondition::Bind(bind("y"), unit()),
        IfLetCondition::Guard(Box::new(Expr::BoolLiteral(true, at()))),
    ];
    let chain = Expr::IfLetChain(conditions, consume(), Some(consume()), at());
    assert_eq!(findings_for(chain), 0);
    let chain_then_reuse = Expr::Block(
        vec![Stmt::Expr(
            Expr::IfLetChain(vec![], consume(), None, at()),
            at(),
        )],
        Some(boxed("sb")),
        at(),
    );
    assert_eq!(findings_for(chain_then_reuse), 1);
}

// 7.10.26t AC2: the proof forms walk their parts in order; `have` and
// `assume` bind a non-handle.
#[test]
fn the_proof_forms_count_their_parts() {
    let ty = || TypeExpr::Unit(at());
    let have = Expr::Have(ident("h"), ty(), boxed("sb"), boxed("sb"), at());
    assert_eq!(findings_for(have), 1);
    let assume = Expr::Assume(ident("sb"), ty(), boxed("sb"), at());
    assert_eq!(findings_for(assume), 0);
    let subst = Expr::Subst(boxed("sb"), Motive::Expr(boxed("sb")), unit(), at());
    assert_eq!(findings_for(subst), 1);
    let natind = Expr::NatInd(Motive::Expr(unit()), boxed("sb"), boxed("sb"), unit(), at());
    assert_eq!(findings_for(natind), 1);
    let natrec = Expr::NatRec(Box::new(ty()), boxed("sb"), unit(), boxed("sb"), at());
    assert_eq!(findings_for(natrec), 1);
    assert_eq!(findings_for(Expr::Trans(boxed("sb"), boxed("sb"), at())), 1);
    assert_eq!(findings_for(Expr::Cong(boxed("sb"), unit(), at())), 0);
    assert_eq!(findings_for(Expr::Sym(boxed("sb"), at())), 0);
    assert_eq!(findings_for(Expr::Show(ty(), boxed("sb"), at())), 0);
    let negated = Expr::Unary(UnaryOp::Not, boxed("sb"), at());
    assert_eq!(
        findings_for(Expr::Field(Box::new(negated), ident("x"), at())),
        0
    );
}

// 7.10.26t AC2: a `match` every arm of which returns ends the path, so a use
// after it is on no path; a guard is walked with its arm.
#[test]
fn a_match_whose_every_arm_returns_ends_the_path() {
    let returns = Expr::Match(
        unit(),
        vec![
            arm(
                Pattern::Wildcard(at()),
                Some(Expr::BoolLiteral(true, at())),
                Expr::Return(Some(Box::new(call("to_string", vec![var("sb")]))), at()),
            ),
            arm(bind("z"), None, Expr::Return(None, at())),
        ],
        at(),
    );
    let after = Expr::Block(vec![Stmt::Expr(returns, at())], Some(boxed("sb")), at());
    assert_eq!(findings_for(after), 0);
}

// 7.10.26t AC2: an observer called through parentheses still reads its bare
// argument, and a non-variable argument is walked.
#[test]
fn an_observer_is_recognised_through_parentheses() {
    let paren = |inner: Expr| Expr::Paren(Box::new(inner), at());
    let observed = Expr::Block(
        vec![Stmt::Expr(
            Expr::App(
                Box::new(paren(var("string_builder_len"))),
                vec![paren(var("sb")), Expr::Unit(at())],
                at(),
            ),
            at(),
        )],
        Some(Box::new(call("to_string", vec![var("sb")]))),
        at(),
    );
    assert_eq!(findings_for(observed), 0);
}

// 7.10.26t AC1: "contains" looks through references, pointers, parentheses
// and quantifiers, and a parenthesised listed type is a handle parameter.
#[test]
fn contains_looks_through_the_type_wrappers() {
    let declarations = HandleDeclarations::default();
    let bytes = || Box::new(TypeExpr::Path(Path::simple(ident("Bytes"))));
    for wrapped in [
        TypeExpr::Ref(bytes(), at()),
        TypeExpr::Ptr(bytes(), at()),
        TypeExpr::Paren(bytes(), at()),
        TypeExpr::Forall(ident("T"), bytes(), at()),
        TypeExpr::Product(Box::new(TypeExpr::Unit(at())), bytes(), at()),
    ] {
        assert!(declarations.contains_handle(&wrapped), "{wrapped:?}");
    }
    assert!(is_listed_handle(&TypeExpr::Paren(bytes(), at())));
    assert!(!is_listed_handle(&TypeExpr::Ref(bytes(), at())));
    let empty = TypeExpr::Path(Path {
        segments: vec![],
        span: at(),
    });
    assert!(!declarations.contains_handle(&empty));
}

// The use walk over `let mut` and assignment (ADR 13.10.26r D6, AC 4): each
// block is lowered first, and handle-ness crosses a join through its
// `JoinMap`, so two consuming uses through assignments or a join are still
// refused and one is not.
mod assignments {
    use super::super::tests::findings;
    use super::super::HandleFindingKind;

    /// The handles a fixture's `f` uses after consuming them.
    fn reused(source: &str) -> Vec<String> {
        findings(source)
            .into_iter()
            .filter(|(kind, _, _)| *kind == HandleFindingKind::UsedAfterConsumed)
            .map(|(_, handle, _)| handle)
            .collect()
    }

    // 13.10.26r AC4: a handle held in a `let mut`, threaded by assignment inside
    // one arm of an `if`, and consumed once after it, is accepted.
    #[test]
    fn a_handle_threaded_through_a_join_and_consumed_once_is_accepted() {
        let source = "fn f(c: Bool) -> String {\n\
            let mut sb = string_builder_new();\n\
            if c { sb = push_str(sb, \"a\"); };\n\
            to_string(sb)\n\
        }";
        assert!(reused(source).is_empty(), "{:?}", findings(source));
    }

    // 13.10.26r AC4: two consuming uses through assignments are E0092, at the
    // second use.
    #[test]
    fn two_consuming_uses_through_assignments_are_refused() {
        let source = "fn f() -> String {\n\
            let mut sb = string_builder_new();\n\
            sb = push_str(sb, \"a\");\n\
            let t = to_string(sb);\n\
            to_string(sb)\n\
        }";
        assert_eq!(reused(source), ["sb"]);
    }

    // 13.10.26r AC4: a handle in a two-binding join, consumed twice after it, is
    // E0092; consumed once, accepted.
    #[test]
    fn a_handle_in_a_two_binding_join_is_still_tracked() {
        let twice = "fn f(c: Bool) -> String {\n\
            let mut sb = string_builder_new();\n\
            let mut n = 0;\n\
            if c { sb = push_str(sb, \"a\"); n = 1; } else { n = 2; };\n\
            let t = to_string(sb);\n\
            to_string(sb)\n\
        }";
        assert_eq!(reused(twice), ["sb"]);
        let once = "fn f(c: Bool) -> String {\n\
            let mut sb = string_builder_new();\n\
            let mut n = 0;\n\
            if c { sb = push_str(sb, \"a\"); n = 1; } else { n = 2; };\n\
            to_string(sb)\n\
        }";
        assert!(reused(once).is_empty(), "{:?}", findings(once));
    }

    // 13.10.26r AC4: a handle joined beside the construct's value, consumed twice
    // after the join, is E0092; consumed once, accepted.
    #[test]
    fn a_handle_joined_beside_a_value_is_still_tracked() {
        let twice = "fn f(c: Bool) -> String {\n\
            let mut sb = string_builder_new();\n\
            let k = if c { sb = push_str(sb, \"a\"); 1 } else { 2 };\n\
            let t = to_string(sb);\n\
            to_string(sb)\n\
        }";
        assert_eq!(reused(twice), ["sb"]);
        let once = "fn f(c: Bool) -> String {\n\
            let mut sb = string_builder_new();\n\
            let k = if c { sb = push_str(sb, \"a\"); 1 } else { 2 };\n\
            to_string(sb)\n\
        }";
        assert!(reused(once).is_empty(), "{:?}", findings(once));
    }

    // 13.10.26r D6: an arm that consumes the handle it hands back makes the joined
    // binding consumed, so a use after the join is refused.
    #[test]
    fn a_handle_consumed_in_an_arm_is_consumed_after_the_join() {
        let source = "fn f(c: Bool) -> String {\n\
            let mut sb = string_builder_new();\n\
            let mut s = \"\";\n\
            if c { sb = push_str(sb, \"a\"); } else { s = to_string(sb); };\n\
            to_string(sb)\n\
        }";
        assert_eq!(reused(source), ["sb"]);
    }
}
