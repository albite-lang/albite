//! A type parameter applied to arguments is refused, and a self-reference is
//! not (ADR 6.10.26b). A signature-level refusal stops collection for the
//! whole file, so these are Rust tests and one golden, not a `.tg` suite; the
//! must-fail twin is the acceptance of `type Ap<F, T> = Mk(F)`.

use crate::ast::{Ident, Path, TypeExpr};
use crate::elaborate::error::{ElabError, ElabErrorKind, SortErrorKind};
use crate::elaborate::tests::{elab, elab_ok};
use crate::elaborate::types::paths::{classify_self_reference_arguments, SelfReferenceArguments};
use crate::elaborate::CoreDef;
use crate::span::Span;

fn errors_of(source: &str) -> Vec<ElabError> {
    elab(source).err().unwrap_or_default()
}

/// The single error `source` raises, asserted to be the only one.
fn only_error(source: &str) -> ElabError {
    let errors = errors_of(source);
    assert_eq!(errors.len(), 1, "{source}: {errors:?}");
    errors.into_iter().next().unwrap()
}

/// Every error `source` raises is E0068 for `parameter`. A signature's types
/// are elaborated again with its body, so one site may report twice.
fn assert_parameter_refused(source: &str, parameter: &str) {
    let errors = errors_of(source);
    assert!(!errors.is_empty(), "{source} was accepted");
    for error in &errors {
        assert_eq!(error.kind.code(), "E0068", "{source}: {error:?}");
        let ElabErrorKind::Sort(SortErrorKind::TypeParameterApplied { name, own_params }) =
            &error.kind
        else {
            panic!("{source}: {error:?}");
        };
        assert_eq!(name, parameter);
        assert_eq!(*own_params, None);
        assert!(
            error
                .message
                .contains(&format!("type parameter `{parameter}`")),
            "{}",
            error.message
        );
    }
}

fn def_type(defs: &[CoreDef], name: &str) -> String {
    let def = defs
        .iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"));
    def.ty.to_string()
}

// 6.10.26b AC1: the refusal helper is not vacuous — it fails on a source the
// elaborator accepts, so the five refusal tests below cannot pass by accident.
#[test]
#[should_panic(expected = "was accepted")]
fn the_refusal_assertion_fails_on_an_accepted_source() {
    assert_parameter_refused("type Ap<F, T> = Mk(F)", "F");
}

// 6.10.26b AC1: the parked shape — a type definition's parameter applied.
#[test]
fn a_type_definitions_parameter_applied_is_refused() {
    assert_parameter_refused("type Ap<F, T> = Mk(F<T>)", "F");
}

// 6.10.26b AC1: a function signature's parameter applied.
#[test]
fn a_signatures_parameter_applied_is_refused() {
    assert_parameter_refused("fn g<F>(x: F<Nat>) -> F<Bool> { x }", "F");
}

// 6.10.26b AC1: a `forall` binder applied.
#[test]
fn a_forall_binder_applied_is_refused() {
    assert_parameter_refused("fn k(x: forall F. F<Nat>) -> Nat { 0 }", "F");
}

// 6.10.26b AC1: an alias's parameter applied.
#[test]
fn an_alias_parameter_applied_is_refused() {
    assert_parameter_refused("type K<F> = F<Nat>", "F");
}

// 6.10.26b AC1 (D2's order): the parameter `F` shadows the type's own name,
// so the application is a parameter's, not a recursive use's.
#[test]
fn a_parameter_shadowing_the_types_own_name_is_refused_as_a_parameter() {
    assert_parameter_refused("type F<F> = Mk(F<Nat>)", "F");
}

// 6.10.26b AC1: the must-fail twin — the first-order spelling of what the
// parked shape checked is accepted, so a refusal on every generic field fails.
#[test]
fn a_bare_parameter_field_is_accepted() {
    let defs = elab_ok("type Ap<F, T> = Mk(F)\nfn wrap(x: Nat) -> Ap<Nat, Bool> { Mk(x) }");
    assert_eq!(def_type(&defs, "wrap"), "(Nat → Nat)");
}

// 6.10.26b AC1: a uniform self-reference, applied and bare, elaborates as
// it did before: the use from another definition is the μ-encoded list.
#[test]
fn a_uniform_self_reference_is_accepted() {
    let list = "type List<T> = Nil | Cons(T, List<T>)\n";
    let bare = "type List<T> = Nil | Cons(T, List)\n";
    let user = "fn id_list(xs: List<Nat>) -> List<Nat> { xs }";
    let applied = def_type(&elab_ok(&format!("{list}{user}")), "id_list");
    let bare_form = def_type(&elab_ok(&format!("{bare}{user}")), "id_list");
    assert_eq!(
        applied,
        "(μα_List. (Unit + (Nat × α_List)) → μα_List. (Unit + (Nat × α_List)))"
    );
    assert_eq!(bare_form, applied);
}

// 6.10.26b AC1: a mutually recursive pair, each naming the other uniformly.
// The literal is the pre-change binary's own rendering (`info def`): a
// cluster's μ wraps the entry member's body only, so it reads as unclosed.
#[test]
fn a_mutually_recursive_pair_is_accepted() {
    let source = "type Tree<T> = Node(T, Forest<T>)\n\
                  type Forest<T> = FNil | FCons(Tree<T>, Forest<T>)\n\
                  fn id_tree(t: Tree<Nat>) -> Tree<Nat> { t }";
    assert_eq!(
        def_type(&elab_ok(source), "id_tree"),
        "(μα_Tree. μα_Forest. (Nat × α_Forest) → μα_Tree. μα_Forest. (Nat × α_Forest))"
    );
}

// 6.10.26b AC1: a generic record.
#[test]
fn a_generic_record_is_accepted() {
    let source = "type Pair<T> = { first: T, second: T }\n\
                  fn id_pair(p: Pair<Nat>) -> Pair<Nat> { p }";
    assert_eq!(
        def_type(&elab_ok(source), "id_pair"),
        "(Pair<Nat> → Pair<Nat>)"
    );
}

// 6.10.26b AC1 (D1): a refused type definition pops its scopes on the error
// path, so the next definition's use of it is an application, not a leaked
// self-reference; exactly one error is reported. A leaked `Ap` would read
// `Ap<Nat, Nat>` as a recursive use with other arguments: a second E0068.
#[test]
fn a_refused_definition_leaks_no_self_reference() {
    let source = "type Ap<F, T> = Mk(F<T>)\n\
                  type U = W(Ap<Nat, Nat>)\n\
                  fn h<G>(x: G) -> G { x }";
    let error = only_error(source);
    assert_eq!(error.kind.code(), "E0068");
}

// 6.10.26b AC1 (D1): a refused alias pops its parameter, so a later type of
// the same name applies normally.
#[test]
fn a_refused_alias_leaks_no_parameter() {
    let source = "type K<F> = F<Nat>\n\
                  type F<T> = MkF(T)\n\
                  fn k(x: F<Nat>) -> F<Nat> { x }";
    let error = only_error(source);
    assert_eq!(error.kind.code(), "E0068");
}

// 6.10.26b AC1 (D1): a function whose body fails pops its type parameters,
// so a later body's application of a same-named type is not refused.
#[test]
fn a_failed_function_body_leaks_no_type_parameter() {
    let source = "type Box<T> = B(T)\n\
                  fn bad<Box>(x: Box) -> Box { not_defined }\n\
                  fn good(x: Box<Nat>) -> Box<Nat> { let y: Box<Nat> = x; y }";
    let error = only_error(source);
    assert_ne!(error.kind.code(), "E0068", "{error:?}");
}

// 6.10.26b AC1 (D1): the same for a theorem's and an axiom's type parameters.
#[test]
fn a_failed_theorem_or_axiom_leaks_no_type_parameter() {
    for failing in [
        "theorem bad<Box>(x: Box) -> x == x { not_defined }",
        "axiom bad<Box>(x: Box, y: Missing) -> x == x",
    ] {
        let source = format!(
            "type Box<T> = B(T)\n{failing}\n\
             fn good(x: Box<Nat>) -> Box<Nat> {{ let y: Box<Nat> = x; y }}"
        );
        let errors = errors_of(&source);
        assert!(!errors.is_empty(), "{source}");
        assert!(
            errors.iter().all(|e| e.kind.code() != "E0068"),
            "{source}: {errors:?}"
        );
    }
}

// 6.10.26b AC2: a self-reference with other arguments is non-uniform
// recursion, refused under E0068 with the definition's own parameters.
#[test]
fn a_self_reference_with_other_arguments_is_refused() {
    let error = only_error("type Nest<T> = Leaf(T) | Node(Nest<Nat>)");
    assert_eq!(error.kind.code(), "E0068");
    let ElabErrorKind::Sort(SortErrorKind::TypeParameterApplied { name, own_params }) = &error.kind
    else {
        panic!("{error:?}");
    };
    assert_eq!(name, "Nest");
    assert_eq!(*own_params, Some(vec!["T".to_string()]));
    assert_eq!(
        error.message,
        "a recursive use of `Nest` must pass its own parameters, `<T>`"
    );
}

// 6.10.26b AC2: a self-reference with the wrong number of arguments is the
// definition's own arity, E0012.
#[test]
fn a_self_reference_with_the_wrong_arity_is_an_arity_mismatch() {
    let error = only_error("type L<T> = C(L<T, T>) | E");
    assert_eq!(error.kind.code(), "E0012");
    assert!(
        error
            .notes
            .iter()
            .any(|n| n.message == "`L` is defined with 1 type parameter(s)"),
        "{error:?}"
    );
}

fn bare(name: &str) -> TypeExpr {
    TypeExpr::Path(Path::simple(Ident::new(name, Span::new(0, 1))))
}

fn names(params: &[&str]) -> Vec<String> {
    params.iter().map(|p| p.to_string()).collect()
}

// 6.10.26b AC2: the classification itself, over injected arguments.
#[test]
fn self_reference_arguments_are_classified_by_position() {
    let own = names(&["A", "B"]);
    let classify = |args: &[TypeExpr]| classify_self_reference_arguments(&own, args);
    assert_eq!(
        classify(&[bare("A"), bare("B")]),
        SelfReferenceArguments::Own
    );
    assert_eq!(
        classify(&[bare("B"), bare("A")]),
        SelfReferenceArguments::Other
    );
    assert_eq!(
        classify(&[bare("A"), bare("Nat")]),
        SelfReferenceArguments::Other
    );
    assert_eq!(classify(&[bare("A")]), SelfReferenceArguments::WrongArity);
    assert_eq!(classify(&[]), SelfReferenceArguments::WrongArity);
    let none: Vec<String> = Vec::new();
    assert_eq!(
        classify_self_reference_arguments(&none, &[]),
        SelfReferenceArguments::Own
    );
}
