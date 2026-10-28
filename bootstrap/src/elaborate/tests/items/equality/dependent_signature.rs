//! A signature that mentions its parameters (ADR 30.9.26d AC 6, AC 2).
//!
//! Signature collection binds a signature's parameters left to right, folds a
//! mentioned one into a `Pi`, and application substitutes the argument into
//! the result.

use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::CoreDef;
use albite_core::{Term, Type};

const ADD_ZERO_LEFT: &str = "fn add_zero_left(n: Nat) -> Eq<Nat, 0 + n, n> { \
    natind(|k: Nat| Eq<Nat, 0 + k, k>, refl, fn(k: Nat, ih) => cong(|x: Nat| x + 1, ih), n) }";

fn def<'d>(defs: &'d [CoreDef], name: &str) -> &'d CoreDef {
    defs.iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
}

fn codes(source: &str) -> Vec<&'static str> {
    elab_err(source).iter().map(|e| e.kind.code()).collect()
}

/// The first error's code; asserting `codes(..) == [..]` would also pin the
/// cascade, which is not what these cases test.
fn first_code(source: &str) -> &'static str {
    codes(source)[0]
}

// ── Signature collection builds a Pi only when the result mentions a parameter

// 30.9.26d AC6: a dependent function signature is a `Pi`, a plain one an `Arrow`.
#[test]
fn dependent_function_signature_is_a_pi() {
    let defs = elab_ok(&format!("{ADD_ZERO_LEFT}\nfn plain(n: Nat) -> Nat {{ n }}"));
    assert!(
        matches!(&def(&defs, "add_zero_left").ty, Type::Pi(x, dom, _) if x == "n" && **dom == Type::Nat)
    );
    assert_eq!(def(&defs, "plain").ty, Type::arrow(Type::Nat, Type::Nat));
}

// 30.9.26d AC6: theorems and axioms get the same fold.
#[test]
fn dependent_theorem_and_axiom_signatures_are_pis() {
    let defs = elab_ok(
        "theorem t(n: Nat): Eq<Nat, n, n> { refl }\n\
         axiom a(n: Nat): Eq<Nat, n, n>\n\
         axiom plain(n: Nat): Eq<Nat, 0, 0>",
    );
    assert!(matches!(def(&defs, "t").ty, Type::Pi(..)));
    assert!(matches!(def(&defs, "a").ty, Type::Pi(..)));
    assert!(matches!(def(&defs, "plain").ty, Type::Arrow(..)));
}

// 30.9.26d AC6: a hypothesis over an earlier parameter binds left to right.
#[test]
fn a_parameter_type_mentions_an_earlier_parameter() {
    let defs = elab_ok("fn flip(n: Nat, h: Eq<Nat, n, 0>) -> Eq<Nat, 0, n> { sym(h) }");
    let Type::Pi(outer, _, rest) = &def(&defs, "flip").ty else {
        panic!("outer layer is not a Pi");
    };
    assert_eq!(outer, "n");
    // `h` is not mentioned by the result, so its layer stays an arrow.
    assert!(matches!(**rest, Type::Arrow(..)));
}

// 30.9.26d AC6: an axiom's dependent parameter keeps its name in the Core
// term; a non-dependent one keeps `_`.
#[test]
fn an_axioms_dependent_parameter_keeps_its_name() {
    let defs = elab_ok("axiom ax(n: Nat, h: Eq<Nat, n, 0>): Eq<Nat, 0, n>");
    let Term::Lambda(first, _, body) = &def(&defs, "ax").term.term else {
        panic!("axiom term is not a lambda");
    };
    assert_eq!(first, "n");
    assert!(matches!(body.as_ref(), Term::Lambda(second, _, _) if second == "_"));
}

// ── Application substitutes

// 30.9.26d AC6: a citation checks against the substituted instance.
#[test]
fn application_returns_the_substituted_codomain() {
    elab_ok(&format!(
        "{ADD_ZERO_LEFT}\nfn at() -> Bool {{ let _p: Eq<Nat, 0 + 3, 3> = add_zero_left(3); true }}"
    ));
}

// 30.9.26e AC6: 30.9.26d's known-refused control, flipped by 30.9.26e D5 —
// the citation's `0 + 3` converts to the annotation's `3`.
#[test]
fn a_citation_converts_at_its_annotation() {
    elab_ok(&format!(
        "{ADD_ZERO_LEFT}\nfn at() -> Bool {{ let _p: Eq<Nat, 3, 3> = add_zero_left(3); true }}"
    ));
}

// 30.9.26d AC6: generic instantiation peels a `Pi`.
#[test]
fn generic_instantiation_peels_a_pi() {
    elab_ok(
        "fn refl_at<T>(x: T) -> Eq<T, x, x> { refl }\n\
         fn at() -> Bool { let _r: Eq<Nat, 5, 5> = refl_at(5); true }",
    );
}

// 30.9.26d AC1: a swapped citation substitutes without capture.
#[test]
fn a_swapped_citation_is_not_captured() {
    elab_ok(
        "fn two(n: Nat, m: Nat) -> Eq<Nat, n + m, n + m> { refl }\n\
         fn swapped(n: Nat, m: Nat) -> Eq<Nat, m + n, m + n> { two(m, n) }",
    );
}

// 30.9.26d AC6: `elaborate_axiom` binds a hypothesis over an earlier parameter.
#[test]
fn an_axiom_hypothesis_over_an_earlier_parameter_is_cited() {
    elab_ok(
        "axiom ax(n: Nat, h: Eq<Nat, n, 0>): Eq<Nat, 0, n>\n\
         fn at() -> Bool { let _a: Eq<Nat, 0, 0> = ax(0, refl); true }",
    );
}

// ── AC 2's refusals

// 30.9.26d AC2: a citation at the wrong argument is E0010.
#[test]
fn a_citation_at_the_wrong_argument_is_refused() {
    let source = format!(
        "{ADD_ZERO_LEFT}\nfn at() -> Bool {{ let _p: Eq<Nat, 0 + 4, 4> = add_zero_left(3); true }}"
    );
    assert_eq!(first_code(&source), "E0010");
}

// 30.9.26d AC2: a name that is no parameter of this signature is E0001.
#[test]
fn a_name_from_no_parameter_is_refused() {
    assert_eq!(
        first_code("fn g(m: Nat) -> Eq<Nat, 0 + n, n> { refl }"),
        "E0001"
    );
}

// 30.9.26d AC2: parameters scope left to right.
#[test]
fn a_later_parameter_is_not_in_scope_earlier() {
    assert_eq!(
        first_code("fn h(p: Eq<Nat, n, n>, n: Nat) -> Nat { 0 }"),
        "E0001"
    );
}

// 30.9.26d AC2: a term binder never becomes a type (D7's control), re-pointed
// from `p: Prop` to `p: Nat` when `Prop` stopped being a type (4.10.26i D4).
#[test]
fn a_term_parameter_is_never_a_type() {
    assert_eq!(first_code("fn f(p: Nat, h: p) -> Nat { 0 }"), "E0002");
}

// 4.10.26i AC3 (D3, D4): a `Prop` parameter is refused as a sort, dependent or
// not — 30.9.26d's E0100 for the dependent one and its acceptance of the plain
// one both flipped.
#[test]
fn a_prop_parameter_is_refused_as_a_sort() {
    assert_eq!(
        first_code("fn pd(p: Prop) -> Eq<Prop, p, p> { refl }"),
        "E0066"
    );
    assert_eq!(first_code("fn f(p: Prop) -> Nat { 0 }"), "E0066");
}

// 30.9.26d AC2: known-refused control — flips when signature collection
// defers an `Eq` argument that names a later global (D9, parked).
#[test]
fn a_signature_citing_a_later_global_is_refused() {
    assert_eq!(
        first_code(
            "fn uses(m: Nat) -> Eq<Nat, add(2, 0), 2> { refl }\n\
             fn add(a: Nat, b: Nat) -> Nat { a + b }"
        ),
        "E0001"
    );
    elab_ok(
        "fn add(a: Nat, b: Nat) -> Nat { a + b }\n\
         fn uses(m: Nat) -> Eq<Nat, add(2, 0), add(2, 0)> { refl }",
    );
}

// 30.9.26d AC6: a signature that fails inside `bind_params_then` pops its
// scope, so the next signature cannot see its parameter.
#[test]
fn a_failed_signature_does_not_leak_its_parameters() {
    let errors = elab_err(
        "fn first(n: Nat, bad: NoSuchType) -> Nat { n }\n\
         fn second(m: Nat) -> Eq<Nat, n, n> { refl }",
    );
    assert!(errors.iter().any(|e| e.kind.code() == "E0002"));
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0001"),
        "`n` leaked from `first` into `second`: {errors:?}"
    );
}

// 30.9.26d AC6: a generic axiom's binder names are read under its `∀`.
#[test]
fn a_generic_axioms_dependent_parameter_keeps_its_name() {
    let defs = elab_ok("axiom same<T>(x: T): Eq<T, x, x>");
    let Term::TyAbs(_, body) = &def(&defs, "same").term.term else {
        panic!("generic axiom term is not a type abstraction");
    };
    assert!(matches!(body.as_ref(), Term::Lambda(binder, _, _) if binder == "x"));
}

// 30.9.26d D8: a message that names a `Pi` prints its binder (the formatter
// ended in `unreachable!()` before), and a non-function piped is still E0013.
#[test]
fn a_pi_is_printed_in_a_mismatch_and_a_non_function_pipe_is_refused() {
    let errors = elab_err(&format!(
        "{ADD_ZERO_LEFT}\nfn at() -> Nat {{ let x: Nat = add_zero_left; x }}"
    ));
    assert_eq!(errors[0].kind.code(), "E0010");
    assert!(
        errors[0].message.contains("(n: Nat) →"),
        "{}",
        errors[0].message
    );
    assert_eq!(first_code("fn at() -> Nat { 3 |> 4 }"), "E0013");
}
