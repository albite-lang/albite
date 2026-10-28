//! Phase 3 tests: `sym`, `trans`, `cong` derived combinators.

use super::*;
use crate::ast::{Ident, LambdaParam, Path, Pattern};
use crate::elaborate::tests::elab_ok;

/// Build an annotated identity lambda `(|x| x : Nat -> Nat)`.
fn annotated_identity_nat() -> Expr {
    let x_ident = Ident {
        name: "x".into(),
        span: Span::new(1, 2),
    };
    let x_path = Path {
        segments: vec![x_ident.clone()],
        span: Span::new(4, 5),
    };
    Expr::Annot(
        Box::new(Expr::Lambda(
            vec![LambdaParam {
                pattern: Pattern::Var(x_ident),
                ty: None,
                span: Span::new(1, 2),
            }],
            Box::new(Expr::Path(x_path)),
            Span::new(0, 6),
        )),
        TypeExpr::Arrow(
            Box::new(TypeExpr::Path(Path {
                segments: vec![Ident {
                    name: "Nat".into(),
                    span: Span::new(0, 3),
                }],
                span: Span::new(0, 3),
            })),
            Box::new(TypeExpr::Path(Path {
                segments: vec![Ident {
                    name: "Nat".into(),
                    span: Span::new(0, 3),
                }],
                span: Span::new(0, 3),
            })),
            Span::new(0, 10),
        ),
        Span::new(0, 15),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// sym
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_sym_zero_eq_zero() {
    let proof = annotated_refl_zero_eq_zero();
    let sym_expr = Expr::Sym(Box::new(proof), Span::new(0, 20));
    let result = infer_expr(&sym_expr);
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    let (_, ty) = result.unwrap();
    assert_eq!(ty, Type::eq(Type::Nat, Term::Zero, Term::Zero));
}

#[test]
fn test_sym_not_equality() {
    let proof = Expr::BoolLiteral(true, Span::new(0, 4));
    let sym_expr = Expr::Sym(Box::new(proof), Span::new(0, 10));
    let result = infer_expr(&sym_expr);
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), ElabErrorKind::SubstExpectedEquality(_)),
        "expected SubstExpectedEquality"
    );
}

#[test]
fn test_sym_parser_round_trip() {
    let body = extract_body_expr("fn test() { sym(refl) }");
    assert!(
        matches!(body, Expr::Sym(_, _)),
        "expected Expr::Sym, got {:?}",
        body
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// trans
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_trans_zero_eq_zero() {
    let h1 = annotated_refl_zero_eq_zero();
    let h2 = annotated_refl_zero_eq_zero();
    let trans_expr = Expr::Trans(Box::new(h1), Box::new(h2), Span::new(0, 30));
    let result = infer_expr(&trans_expr);
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    let (_, ty) = result.unwrap();
    assert_eq!(ty, Type::eq(Type::Nat, Term::Zero, Term::Zero));
}

#[test]
fn test_trans_not_equality() {
    let h1 = Expr::BoolLiteral(true, Span::new(0, 4));
    let h2 = annotated_refl_zero_eq_zero();
    let trans_expr = Expr::Trans(Box::new(h1), Box::new(h2), Span::new(0, 20));
    let result = infer_expr(&trans_expr);
    assert!(result.is_err());
}

#[test]
fn test_trans_endpoint_mismatch() {
    // h1 : Eq(Nat, 0, 0), h2 : Eq(Nat, 1, 1) — h1's right (0) ≠ h2's left (1)
    let h1 = annotated_refl_zero_eq_zero();
    let h2 = annotated_refl_one_eq_one();
    let trans_expr = Expr::Trans(Box::new(h1), Box::new(h2), Span::new(0, 30));
    let result = infer_expr(&trans_expr);
    assert!(result.is_err());
    assert!(
        matches!(
            result.unwrap_err(),
            ElabErrorKind::TransEndpointMismatch { .. }
        ),
        "expected TransEndpointMismatch"
    );
}

#[test]
fn test_trans_parser_round_trip() {
    let body = extract_body_expr("fn test() { trans(refl, refl) }");
    assert!(matches!(body, Expr::Trans(_, _, _)), "expected Trans");
}

// ─────────────────────────────────────────────────────────────────────────────
// cong
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_cong_succ_zero_eq_zero() {
    let f_expr = annotated_identity_nat();
    let proof = annotated_refl_zero_eq_zero();
    let cong_expr = Expr::Cong(Box::new(f_expr), Box::new(proof), Span::new(0, 30));

    let ctx = Box::leak(Box::new(Context::new()));
    let mut elab = Elaborator::new(ctx);
    let result = elab.infer(&cong_expr);
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    let (_, ty) = result.unwrap();
    assert!(
        matches!(ty, Type::Eq(_, _, _)),
        "expected Eq type, got {:?}",
        ty
    );
}

#[test]
fn test_cong_not_function() {
    let f = Expr::IntLiteral(42, Span::new(0, 2));
    let proof = annotated_refl_zero_eq_zero();
    let cong_expr = Expr::Cong(Box::new(f), Box::new(proof), Span::new(0, 20));
    let result = infer_expr(&cong_expr);
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), ElabErrorKind::CongExpectedFunction(_)),
        "expected CongExpectedFunction"
    );
}

#[test]
fn test_cong_nested() {
    // cong(f, cong(f, refl)) — verify combinator composition
    let f = annotated_identity_nat();
    let proof = annotated_refl_zero_eq_zero();
    let inner = Expr::Cong(Box::new(f.clone()), Box::new(proof), Span::new(0, 20));
    let outer = Expr::Cong(Box::new(f), Box::new(inner), Span::new(0, 30));
    let result = infer_expr(&outer);
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    let (_, ty) = result.unwrap();
    assert!(
        matches!(ty, Type::Eq(_, _, _)),
        "expected Eq type, got {:?}",
        ty
    );
}

#[test]
fn test_cong_parser_round_trip() {
    let body = extract_body_expr("fn test() { cong(refl, refl) }");
    assert!(matches!(body, Expr::Cong(_, _, _)), "expected Cong");
}

// ─────────────────────────────────────────────────────────────────────────────
// The stored motive (ADR 1.10.26e D3, AC 2)
// ─────────────────────────────────────────────────────────────────────────────

const REWRITES: &str = "
fn sym_ab(a: Nat, b: Nat, h: Eq<Nat, a, b>) -> Eq<Nat, b, a> { sym(h) }
fn trans_abc(a: Nat, b: Nat, c: Nat, h1: Eq<Nat, a, b>, h2: Eq<Nat, b, c>) -> Eq<Nat, a, c> { trans(h1, h2) }
fn cong_ab(a: Nat, b: Nat, h: Eq<Nat, a, b>) -> Eq<Nat, a + 1, b + 1> { cong(|x: Nat| x + 1, h) }
";

/// The motive of the first `Subst` in `name`'s elaborated term.
pub(super) fn first_subst_motive(defs: &[crate::elaborate::CoreDef], name: &str) -> Type {
    fn find(term: &Term) -> Option<Type> {
        if let Term::Subst(_, motive, _, _) = term {
            return Some(motive.clone());
        }
        let mut found = None;
        term.for_each_subterm(|child| {
            if found.is_none() {
                found = find(child);
            }
        });
        found
    }
    let def = defs
        .iter()
        .find(|def| def.name == name)
        .expect("definition");
    find(&def.term.term).expect("a Subst in the term")
}

/// `motive`'s body with its `Pi` binder renamed to `probe`, so two motives
/// compare up to binder renaming.
fn motive_body_at_probe(motive: &Type) -> (Type, Type) {
    let Type::Pi(binder, domain, body) = motive else {
        panic!("expected a Pi motive, got {motive}");
    };
    let probe = Term::var("probe");
    ((**domain).clone(), body.substitute_term(binder, &probe))
}

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `(λx:Nat. x + 1) at`, as `cong`'s `f` elaborates.
fn plus_one_at(at: Term) -> Term {
    let plus_one = Term::lambda(
        "x",
        Type::Nat,
        Term::nat_add(Term::var("x"), Term::succ(Term::Zero)),
    );
    Term::app(plus_one, at)
}

// 1.10.26e AC2
#[test]
fn sym_states_its_motive() {
    let motive = first_subst_motive(&elab_ok(REWRITES), "sym_ab");
    let expected = eq_nat(Term::var("probe"), Term::var("a"));
    assert_eq!(motive_body_at_probe(&motive), (Type::Nat, expected));
}

// 1.10.26e AC2
#[test]
fn trans_states_its_motive() {
    let motive = first_subst_motive(&elab_ok(REWRITES), "trans_abc");
    let expected = eq_nat(Term::var("a"), Term::var("probe"));
    assert_eq!(motive_body_at_probe(&motive), (Type::Nat, expected));
}

// 1.10.26e AC2
#[test]
fn cong_states_its_motive() {
    let motive = first_subst_motive(&elab_ok(REWRITES), "cong_ab");
    let expected = eq_nat(plus_one_at(Term::var("a")), plus_one_at(Term::var("probe")));
    assert_eq!(motive_body_at_probe(&motive), (Type::Nat, expected));
}

// 1.10.26e AC2 (P0.4): `#` cannot start an identifier, the spelling is not
// conversion's `?` opener, and it carries no `′` rename mark.
#[test]
fn the_synthesised_motive_binder_is_not_a_source_name() {
    let Type::Pi(binder, _, _) = first_subst_motive(&elab_ok(REWRITES), "sym_ab") else {
        panic!("sym stores a Pi motive");
    };
    assert_eq!(binder, "#x");
    let (tokens, _) = crate::lexer::Lexer::new(&binder).tokenize();
    assert_ne!(
        tokens.first().map(|token| token.kind),
        Some(crate::token::TokenKind::Ident),
        "the lexer must not open an identifier on the binder"
    );
    assert!(!binder.starts_with('?'));
    assert!(!binder.contains('′'));
}

// 1.10.26e AC2: each elaborated rewrite types in the kernel's proof mode as
// the type the elaborator gave it.
#[test]
fn elaborated_rewrites_type_in_the_kernel() {
    let defs = elab_ok(REWRITES);
    for name in ["sym_ab", "trans_abc", "cong_ab"] {
        let def = defs
            .iter()
            .find(|def| def.name == name)
            .expect("definition");
        assert_eq!(
            albite_core::typecheck::check_proof(&Context::new(), &def.term.term, &def.ty),
            Ok(()),
            "{name}"
        );
    }
}
