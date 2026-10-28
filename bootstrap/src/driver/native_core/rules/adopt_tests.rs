//! The adoption rule (ADR 6.10.26n D3, AC 3), over injected Core terms. The
//! embedded sources are checked beside the core's other rules, in
//! `native_core/tests.rs`.

use super::adopt::{adoptions, owned_definitions, AdoptHop, AdoptedArgument, Adoption};
use super::{check_core_rules, CoreRuleViolation, DefinitionFacts};
use albite_core::terms::Term;
use albite_core::types::Type;

fn var(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn lambda(param: &str, body: Term) -> Term {
    Term::Lambda(param.to_string(), Type::String, Box::new(body))
}

fn let_in(name: &str, definition: Term, body: Term) -> Term {
    Term::Let(
        name.to_string(),
        Type::Nat,
        Box::new(definition),
        Box::new(body),
    )
}

/// `head(args…)` as a curried application spine; `cover_tests.rs` reuses it.
pub(super) fn call(head: &str, args: Vec<Term>) -> Term {
    args.into_iter()
        .fold(Term::Global(head.to_string()), |function, arg| {
            Term::App(Box::new(function), Box::new(arg))
        })
}

/// `bytes::bytes_from_string_grown(adopted, string_len(right))`.
fn wrapper_call(adopted: Term) -> Term {
    call(
        "bytes::bytes_from_string_grown",
        vec![adopted, Term::StrLen(Box::new(var("right")))],
    )
}

/// `λleft. λright. <body>`.
fn owned_shape(body: Term) -> Term {
    lambda("left", lambda("right", body))
}

/// Run the rule over one definition named `name` with term `term`.
fn violations(name: &str, term: &Term) -> Vec<CoreRuleViolation> {
    let facts = DefinitionFacts {
        name: name.to_string(),
        adoptions: adoptions(term),
        ..DefinitionFacts::default()
    };
    check_core_rules(&[facts], &[], &[])
}

fn outside(name: &str) -> CoreRuleViolation {
    CoreRuleViolation::AdoptsOutsideOwned {
        definition: name.to_string(),
    }
}

fn used_after(name: &str) -> CoreRuleViolation {
    CoreRuleViolation::UsedAfterAdopt {
        definition: name.to_string(),
    }
}

/// D1's shape: `let n = string_len(left); if n == 0 { string_concat(left,
/// right) } else { let b = grow(left, …); into(b) }`.
fn d1_shape(adopted: Term, after: Term) -> Term {
    let fresh = call("string::string_concat", vec![var("left"), var("right")]);
    let grown = let_in("b", wrapper_call(adopted), after);
    owned_shape(let_in(
        "n",
        Term::StrLen(Box::new(var("left"))),
        Term::If(
            Box::new(Term::NatEq(Box::new(var("n")), Box::new(Term::NatLit(0)))),
            Box::new(fresh),
            Box::new(grown),
        ),
    ))
}

#[test]
fn only_the_owned_row_names_a_definition_that_may_adopt() {
    assert_eq!(owned_definitions(), vec!["string_concat_owned"]);
}

// 6.10.26n AC 3: D1's shape is accepted — an earlier `string_len(left)` and
// a use of `left` in the arm the adoption never reaches both pass.
#[test]
fn the_owned_shape_with_earlier_reads_is_accepted() {
    let term = d1_shape(var("left"), var("b"));
    assert_eq!(
        adoptions(&term),
        vec![Adoption {
            hop: AdoptHop::Wrapper,
            argument: AdoptedArgument::FirstParameter,
            used_after: false,
        }]
    );
    assert_eq!(violations("string::string_concat_owned", &term), vec![]);
}

// 6.10.26n AC 3: an adoption in a definition no owned row names is refused.
#[test]
fn an_adoption_in_a_non_owned_definition_is_refused() {
    let term = d1_shape(var("left"), var("b"));
    assert_eq!(
        violations("string_concat", &term),
        vec![outside("string_concat")]
    );
}

// 6.10.26n AC 3: adopting the second parameter is refused, as is a literal.
#[test]
fn an_adoption_of_a_non_first_parameter_is_refused() {
    let term = owned_shape(wrapper_call(var("right")));
    assert_eq!(
        adoptions(&term)[0].argument,
        AdoptedArgument::Other,
        "right is not the first parameter"
    );
    assert_eq!(
        violations("string_concat_owned", &term),
        vec![outside("string_concat_owned")]
    );
    let literal = owned_shape(wrapper_call(Term::StringLit("x".into())));
    assert_eq!(
        violations("string_concat_owned", &literal),
        vec![outside("string_concat_owned")]
    );
}

// 6.10.26n AC 3: a direct call to the extern from outside the wrapper is
// refused, even in the owned definition; inside the wrapper it is allowed.
#[test]
fn the_extern_is_allowed_only_inside_the_wrapper() {
    let direct = |param: &str| {
        lambda(
            param,
            lambda(
                "extra",
                Term::ExternCall(
                    "__c_tg_bytes_from_string_grown".to_string(),
                    vec![var(param), var("extra")],
                ),
            ),
        )
    };
    assert_eq!(
        adoptions(&direct("s"))[0].hop,
        AdoptHop::Extern,
        "the extern is a hop of its own"
    );
    assert_eq!(
        violations("bytes::bytes_from_string_grown", &direct("s")),
        vec![]
    );
    assert_eq!(
        violations("string_concat_owned", &direct("left")),
        vec![outside("string_concat_owned")]
    );
    assert_eq!(
        violations("bytes_new", &direct("s")),
        vec![outside("bytes_new")]
    );
    // The declaration's own stub holds the `extern_call`, and is allowed it.
    assert_eq!(
        violations("tg_bytes_from_string_grown", &direct("s")),
        vec![]
    );
}

// 6.10.26n AC 3: the extern called through its stub, by name, is the extern
// hop too — the wrapper's real shape — and refused outside the wrapper.
#[test]
fn the_extern_called_by_name_is_the_extern_hop() {
    let by_name = |param: &str| {
        lambda(
            param,
            lambda(
                "extra",
                call("tg_bytes_from_string_grown", vec![var(param), var("extra")]),
            ),
        )
    };
    let found = adoptions(&by_name("s"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].hop, AdoptHop::Extern);
    assert_eq!(found[0].argument, AdoptedArgument::FirstParameter);
    assert_eq!(violations("bytes_from_string_grown", &by_name("s")), vec![]);
    assert_eq!(
        violations("string_concat_owned", &by_name("left")),
        vec![outside("string_concat_owned")]
    );
}

// 6.10.26n AC 3: reading `left` after adopting it is refused.
#[test]
fn reading_the_first_parameter_after_adopting_it_is_refused() {
    let read_after = d1_shape(
        var("left"),
        Term::StrConcat(Box::new(var("left")), Box::new(var("b"))),
    );
    assert!(adoptions(&read_after)[0].used_after);
    assert_eq!(
        violations("string_concat_owned", &read_after),
        vec![used_after("string_concat_owned")]
    );
    // Also after the branch that adopted: the joined path carries it.
    let after_if = owned_shape(let_in(
        "x",
        Term::If(
            Box::new(Term::True),
            Box::new(wrapper_call(var("left"))),
            Box::new(Term::NatLit(0)),
        ),
        Term::StrLen(Box::new(var("left"))),
    ));
    assert!(adoptions(&after_if)[0].used_after);
}

// 6.10.26n AC 3: adopting an expression derived from `left` is refused.
#[test]
fn adopting_an_expression_derived_from_the_first_parameter_is_refused() {
    let derived = Term::StrSubstring(
        Box::new(var("left")),
        Box::new(Term::NatLit(1)),
        Box::new(Term::NatLit(2)),
    );
    let term = d1_shape(derived, var("b"));
    assert_eq!(
        adoptions(&term)[0].argument,
        AdoptedArgument::DerivedFromFirst
    );
    assert_eq!(
        violations("string_concat_owned", &term),
        vec![used_after("string_concat_owned")]
    );
}

// A binder reusing the first parameter's name shadows it: a later read of
// that name is not a read of the parameter.
#[test]
fn a_shadowing_binder_ends_the_first_parameter() {
    let term = owned_shape(let_in(
        "b",
        wrapper_call(var("left")),
        let_in("left", Term::StringLit("fresh".into()), var("left")),
    ));
    assert!(!adoptions(&term)[0].used_after);
    let in_lambda = owned_shape(let_in(
        "b",
        wrapper_call(var("left")),
        lambda("left", var("left")),
    ));
    assert!(!adoptions(&in_lambda)[0].used_after);
}

#[test]
fn the_violations_render_naming_the_definition() {
    assert!(outside("f").to_string().starts_with("f: adopts a string"));
    assert!(used_after("g")
        .to_string()
        .starts_with("g: adopts its first parameter"));
}
