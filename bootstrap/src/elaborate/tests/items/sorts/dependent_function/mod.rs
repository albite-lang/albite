//! `(x: A) -> B` elaborates to a `Pi`, or to an arrow when `x` is unused (ADR
//! 7.10.26n AC 2, D2, D3).
//!
//! A refusal in a signature stops collection for the whole file, so the
//! refusals the `.tg` twin cannot hold side by side are pinned here.

use super::{def, elab_ok, first_code};
use crate::ast::{Item, TypeExpr};
use crate::elaborate::Elaborator;
use albite_core::{Context, Term, Type};

/// The elaborated type of `pa`'s first parameter: the domain of its outer
/// layer, whichever of `Arrow` or `Pi` that is.
fn first_domain(ty: &Type) -> &Type {
    match ty {
        Type::Arrow(domain, _) | Type::Pi(_, domain, _) => domain,
        other => panic!("expected a function type, got {other}"),
    }
}

fn eq_on(name: &str) -> Type {
    Type::eq(Type::Nat, Term::var(name), Term::var(name))
}

// 7.10.26n AC2 (D2): a used binder is a `Pi` in a parameter's type.
#[test]
fn a_used_binder_is_a_pi() {
    let defs =
        elab_ok("theorem pa(h: (n: Nat) -> Eq<Nat, n, n>, k: Nat) -> Eq<Nat, k, k> { h(k) }");
    assert_eq!(
        first_domain(&def(&defs, "pa").ty),
        &Type::pi("n", Type::Nat, eq_on("n"))
    );
}

// 7.10.26n AC2 (D2): an unused binder is structurally an arrow.
#[test]
fn an_unused_binder_is_an_arrow() {
    let defs = elab_ok("fn f(g: (n: Nat) -> Nat) -> Nat { g(0) }");
    assert_eq!(
        first_domain(&def(&defs, "f").ty),
        &Type::arrow(Type::Nat, Type::Nat)
    );
}

// 7.10.26n AC2 (D2): a binder shadows a parameter of the same name, and the
// result's `n` stays the outer parameter.
#[test]
fn a_binder_shadows_a_parameter_of_the_same_name() {
    let defs = elab_ok("theorem f<P: Nat -> Prop>(n: Nat, h: (n: Nat) -> P<n>) -> P<n> { h(n) }");
    let rendered = def(&defs, "f").ty.to_string();
    assert!(rendered.contains("(n: Nat) → P<n>"), "{rendered}");
}

// 7.10.26n AC2 (D2): the binder's scope is popped when the codomain fails,
// so the error leaks no local and no depth.
#[test]
fn the_scope_is_popped_after_an_error() {
    let (file, errors) =
        crate::parser::parse("fn f(h: (n: Nat) -> Eq<Nat, n, Bogus>) -> Nat { 0 }");
    assert!(errors.is_empty(), "{errors:?}");
    let Item::Function(func) = &file.items[0] else {
        panic!("expected a function");
    };
    let annotation: &TypeExpr = &func.params[0].ty;
    let ctx = Box::leak(Box::new(Context::new()));
    let mut elab = Elaborator::new(ctx);
    assert!(elab.elab_type(annotation).is_err());
    assert!(elab.env.lookup_local("n").is_none());
    assert_eq!(elab.depth, 0);
}

// 7.10.26n AC2 (D2): a binder is a term, so used as a type it is unknown.
#[test]
fn a_binder_used_as_a_type_is_unknown() {
    assert_eq!(first_code("fn f(g: (T: Nat) -> T) -> Nat { 0 }"), "E0002");
}

// 7.10.26n AC2 (D2): a used binder over a proof is E0065.
#[test]
fn a_used_binder_over_a_proof_is_refused() {
    assert_eq!(
        first_code(
            "theorem bad(h: (e: Eq<Nat, 0, 0>) -> Eq<Eq<Nat, 0, 0>, e, e>) -> Eq<Nat, 0, 0> { refl }"
        ),
        "E0065"
    );
}

// 7.10.26n AC2 (D2, D8): a declared field holding a `Pi` is E0025, in a
// record and in a constructor payload.
#[test]
fn a_dependent_field_is_refused() {
    assert_eq!(
        first_code("type Holder = { f: (n: Nat) -> Eq<Nat, n, n> }"),
        "E0025"
    );
    assert_eq!(
        first_code(
            "type Opt<T> = | NoneOf | SomeOf(T)\n\
             type Box2 = | MkBox(Opt<(n: Nat) -> Eq<Nat, n, n>>)"
        ),
        "E0025"
    );
}

// 7.10.26n AC2 (D2): a field whose binder is unused is a plain arrow, and is
// accepted.
#[test]
fn a_field_with_an_unused_binder_is_accepted() {
    elab_ok("type Holder = { f: (n: Nat) -> Nat }");
}

// 7.10.26n AC2 (D2): a `Pi` reaching a payload by instantiation is admitted.
#[test]
fn a_pi_by_instantiation_is_admitted() {
    let defs = elab_ok(
        "type Opt<T> = | NoneOf | SomeOf(T)\n\
         fn wrap() -> Opt<(n: Nat) -> Eq<Nat, n, n>> { SomeOf(|m: Nat| refl) }",
    );
    assert!(def(&defs, "wrap").ty.to_string().contains("(n: Nat)"));
}

// 7.10.26n AC2 (D3): a lambda checked against a `Pi` takes the binder's
// domain, and its body is checked with the parameter's name substituted.
#[test]
fn a_lambda_is_checked_against_a_pi() {
    let defs = elab_ok(
        "theorem aze(h: (n: Nat) -> Eq<Nat, n + 0, n>, k: Nat) -> Eq<Nat, k + 0, k> { h(k) }\n\
         theorem lift(k: Nat) -> Eq<Nat, k + 0, k> { aze(|m: Nat| refl, k) }",
    );
    def(&defs, "lift");
    // A lambda against a type that is no function is still refused.
    assert_eq!(first_code("fn f() -> Nat { |m: Nat| m }"), "E0014");
}
