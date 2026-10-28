//! A family's arguments are expressions, re-read by the head's kind (ADR
//! 6.10.26d D1–D4, D6, AC 1, AC 3).
//!
//! Each accepted spelling is compared against the same statement written as
//! its expansion by hand, so the test pins what the kernel will see.

use super::families::{statement, LE};
use super::{def, first_code};
use crate::elaborate::tests::{elab_err, elab_ok};
use albite_core::Type;

/// The statement of `name` in `source`, after `Le`'s declaration.
fn statement_of(source: &str, name: &str) -> Type {
    let defs = elab_ok(&format!("{LE}{source}"));
    statement(&def(&defs, name).ty).clone()
}

/// `cited` and `by_hand` declare the same theorem `t`; their statements agree.
fn assert_same_statement(cited: &str, by_hand: &str) {
    assert_eq!(statement_of(cited, "t"), statement_of(by_hand, "t"));
}

// 6.10.26d AC1 (D1): `Le<m + 1, n>` against `Le(n, m)` substitutes both at
// once, so the argument naming the other parameter is not captured.
#[test]
fn expression_arguments_expand_simultaneously() {
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat) -> Le<m + 1, n> { sorry }",
        "pub theorem t(n: Nat, m: Nat) -> exists k: Nat. (m + 1) + k == n { sorry }",
    );
}

// 6.10.26d AC1 (D1): an argument naming the body's own binder stays free.
#[test]
fn expression_argument_naming_the_binder_is_not_captured() {
    let cited = statement_of(
        "pub theorem t(k: Nat, m: Nat) -> Le<k + 1, m> { sorry }",
        "t",
    );
    let Type::Sigma(binder, _, _) = &cited else {
        panic!("expected a Sigma, got {cited:?}");
    };
    assert_ne!(binder, "k", "the binder was not renamed: {cited:?}");
    let free = cited.free_term_vars();
    assert!(free.contains("k") && free.contains("m"), "{free:?}");
}

// 6.10.26d AC1 (D2): a numeral, `*` beside a literal, a call and parentheses
// each parse as a term and elaborate to the expansion.
#[test]
fn numeral_call_and_parenthesised_arguments_elaborate() {
    assert_same_statement(
        "pub theorem t(n: Nat) -> Le<0, n> { sorry }",
        "pub theorem t(n: Nat) -> exists k: Nat. 0 + k == n { sorry }",
    );
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat) -> Le<n * 2, m> { sorry }",
        "pub theorem t(n: Nat, m: Nat) -> exists k: Nat. (n * 2) + k == m { sorry }",
    );
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> Le<n * m + 1, j> { sorry }",
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> exists k: Nat. (n * m + 1) + k == j { sorry }",
    );
    // A call keeps its source span, which differs between the two spellings.
    let call = |source: &str| {
        let Type::Sigma(_, _, body) = statement_of(source, "t") else {
            panic!("expected a Sigma");
        };
        let Type::Eq(_, left, right) = *body else {
            panic!("expected an Eq");
        };
        (left.strip_spans(), right.strip_spans())
    };
    assert_eq!(
        call("fn f(n: Nat) -> Nat { n }\npub theorem t(n: Nat, m: Nat) -> Le<f(n), m> { sorry }"),
        call(
            "fn f(n: Nat) -> Nat { n }\n\
             pub theorem t(n: Nat, m: Nat) -> exists k: Nat. f(n) + k == m { sorry }"
        ),
    );
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat) -> Le<(n + 1), m> { sorry }",
        "pub theorem t(n: Nat, m: Nat) -> exists k: Nat. (n + 1) + k == m { sorry }",
    );
}

// 6.10.26d AC1 (D3): `n * m` and `n + m` parse as a product and a sum type,
// and a family re-reads them as `*` and `+`, precedence included.
#[test]
fn product_and_sum_arguments_are_reread_as_arithmetic() {
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> Le<n * m, j> { sorry }",
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> exists k: Nat. (n * m) + k == j { sorry }",
    );
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> Le<n + m, j> { sorry }",
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> exists k: Nat. (n + m) + k == j { sorry }",
    );
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> Le<n + m * j, n> { sorry }",
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> exists k: Nat. (n + (m * j)) + k == n { sorry }",
    );
    // Parentheses with no literal inside parse as a type, and re-read too.
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> Le<(n + m) * j, n> { sorry }",
        "pub theorem t(n: Nat, m: Nat, j: Nat) -> exists k: Nat. ((n + m) * j) + k == n { sorry }",
    );
}

// 6.10.26d AC1 (D4): `Lt` is declared over `Le` and stored as its expansion.
#[test]
fn lt_is_stored_as_le_at_the_successor() {
    assert_same_statement(
        "pub type Lt(n: Nat, m: Nat): Prop = Le<n + 1, m>\n\
         pub theorem t(n: Nat, m: Nat) -> Lt<n, m> { sorry }",
        "pub theorem t(n: Nat, m: Nat) -> exists k: Nat. (n + 1) + k == m { sorry }",
    );
}

// 6.10.26d AC1 (D4): a family cited above its declaration is E0012, with a
// note naming the order.
#[test]
fn a_family_cited_before_it_is_declared_names_the_order() {
    let errors = elab_err(&format!(
        "pub type Lt(n: Nat, m: Nat): Prop = Le<n + 1, m>\n{LE}"
    ));
    assert_eq!(errors[0].kind.code(), "E0012");
    let notes = format!("{:?}", errors[0].notes);
    assert!(notes.contains("declare it before citing it"), "{notes}");
}

// 6.10.26d AC1 (D6): a global value is an argument, as it is in an `Eq`.
#[test]
fn a_global_value_argument_is_accepted() {
    let cited = statement_of(
        "fn limit() -> Nat { 3 }\n\
         pub theorem g(m: Nat, h: Le<limit, m>) -> Le<limit, m> { h }",
        "g",
    );
    assert!(
        format!("{cited:?}").contains("Global(\"limit\")"),
        "{cited:?}"
    );
}

// 6.10.26d AC1 (D3): a name that is a type AND a term is read as the term,
// whether the term is a parameter or a global value.
#[test]
fn a_name_that_is_also_a_type_is_read_as_the_term() {
    let local = statement_of(
        "pub type Width = Nat\npub theorem g(Width: Nat, m: Nat, h: Le<Width, m>) -> Le<Width, m> { h }",
        "g",
    );
    assert!(local.free_term_vars().contains("Width"), "{local:?}");
    let global = statement_of(
        "pub type Count = Nat\nfn Count() -> Nat { 3 }\n\
         pub theorem g(m: Nat, h: Le<Count, m>) -> Le<Count, m> { h }",
        "g",
    );
    assert!(
        format!("{global:?}").contains("Global(\"Count\")"),
        "{global:?}"
    );
}

// The comparison helper is itself asserted: two different statements fail it.
#[test]
#[should_panic(expected = "assertion")]
fn assert_same_statement_refuses_different_statements() {
    assert_same_statement(
        "pub theorem t(n: Nat, m: Nat) -> Le<n, m> { sorry }",
        "pub theorem t(n: Nat, m: Nat) -> Le<m, n> { sorry }",
    );
}

// 6.10.26d AC1 (D3): a term in any other type application is E0023.
#[test]
fn a_term_where_a_type_belongs_is_refused() {
    assert_eq!(
        first_code("type Box<T> = MkBox(T)\nfn f(x: Box<0>) -> Nat { 0 }"),
        "E0023"
    );
    assert_eq!(first_code("fn f(x: Ref<0>) -> Nat { 0 }"), "E0023");
}

// 6.10.26d AC1 (D3): an argument is checked at the parameter's type, so a
// `Bool` literal is a mismatch; an unbound name is unbound, not a type.
#[test]
fn a_wrong_or_unbound_argument_keeps_its_code() {
    let wrong = format!("{LE}pub theorem g(m: Nat) -> Le<true, m> {{ sorry }}");
    assert_eq!(first_code(&wrong), "E0010");
    let unbound = format!("{LE}pub theorem g(m: Nat) -> Le<nowhere, m> {{ sorry }}");
    assert_eq!(first_code(&unbound), "E0001");
}

// A partial callee in a family argument (D1, E0063) is pinned in
// `driver/per_module/tests/kernel_gate/`, where the taint rule runs.

// 6.10.26d AC3 (D3): each refusal's message names what was written and what
// was expected.
#[test]
fn a_confusion_names_both_sides() {
    let type_for_term = elab_err(&format!(
        "{LE}pub theorem g(m: Nat) -> Le<Nat, m> {{ sorry }}"
    ));
    assert_eq!(
        type_for_term[0].message,
        "`Nat` is a type, and `Le`'s parameter `n` takes a `Nat` term"
    );
    let term_for_type = elab_err("fn f(x: Ref<0>) -> Nat { 0 }");
    assert_eq!(
        term_for_type[0].message,
        "`0` is a term, and a type is expected here"
    );
}

/// The first error's message for `fn f(x: <ty>)`, after `Box` and `Le`.
fn annotation_message(ty: &str) -> String {
    let source = format!("type Box<T> = MkBox(T)\n{LE}fn f(m: Nat, x: {ty}) -> Nat {{ 0 }}");
    elab_err(&source)[0].message.clone()
}

// 6.10.26d AC3 (D3): the written argument is named by what it is.
#[test]
fn a_confusion_describes_what_was_written() {
    let takes = "and `Le`'s parameter `n` takes a `Nat` term";
    assert_eq!(
        annotation_message("Le<Box<Nat>, m>"),
        format!("`Box<…>` is a type, {takes}")
    );
    assert_eq!(
        annotation_message("Le<Nat -> Nat, m>"),
        format!("this type is a type, {takes}")
    );
    let expected_type = "is a term, and a type is expected here";
    assert_eq!(
        annotation_message("Box<true>"),
        format!("`true` {expected_type}")
    );
    assert_eq!(
        annotation_message("Box<\"s\">"),
        format!("`\"s\"` {expected_type}")
    );
    assert_eq!(
        annotation_message("Box<m + 1>"),
        format!("this expression {expected_type}")
    );
}
