//! Constructor ownership through the elaborator (ADR 4.10.26d AC1): a
//! constructor of another type is E0022 in every position a match trusts an
//! index, and the rule admits what was right.

use crate::elaborate::tests::{elab_err, elab_ok};

/// The two unrelated types 3.10.26l's fixture declares.
const WRONG_TYPES: &str = r#"
        type OptWrong = NadaWrong | AlgoWrong(Nat)
        type OtherWrong = PickWrong(Nat) | SkipWrong
"#;

/// The E0022 refusals in `errors`, which must be exactly one.
fn only_e0022(
    errors: &[crate::elaborate::error::ElabError],
) -> &crate::elaborate::error::ElabError {
    let refusals: Vec<_> = errors.iter().filter(|e| e.kind.code() == "E0022").collect();
    assert_eq!(refusals.len(), 1, "expected one E0022, got: {errors:?}");
    refusals[0]
}

/// The source text `err` underlines.
fn spanned<'s>(source: &'s str, err: &crate::elaborate::error::ElabError) -> &'s str {
    &source[err.span.start as usize..err.span.end as usize]
}

// 4.10.26d AC1: another type's constructor reads the scrutinee at its index.
#[test]
fn a_constructor_of_another_type_is_e0022() {
    let source = format!(
        "{WRONG_TYPES}
        fn pick(o: OptWrong) -> Nat {{ match o {{ PickWrong(k) => k, _ => 0 }} }}"
    );
    let errors = elab_err(&source);
    let err = only_e0022(&errors);
    assert!(err.message.contains("`OtherWrong`"), "{}", err.message);
    assert!(err.message.contains("(Unit + Nat)"), "{}", err.message);
    let at = spanned(&source, err);
    assert_eq!(at, "PickWrong", "the span is the first constructor pattern");
}

// 4.10.26d AC1: two types' constructors in one match, at the disagreeing arm.
#[test]
fn mixed_constructor_arms_are_e0022() {
    let source = format!(
        "{WRONG_TYPES}
        fn f(o: OptWrong) -> Nat {{ match o {{ AlgoWrong(n) => n, PickWrong(k) => k }} }}"
    );
    let errors = elab_err(&source);
    let err = only_e0022(&errors);
    assert_eq!(spanned(&source, err), "PickWrong");
    assert!(err.message.contains("`OtherWrong`"), "{}", err.message);
    let note = &err
        .notes
        .last()
        .expect("a note naming the match's type")
        .message;
    assert!(
        note.contains("`PickWrong` is a constructor of `OtherWrong`")
            && note.contains("`OptWrong`"),
        "{note}"
    );
}

// 4.10.26d AC1: a mixed arm past the context's last index once panicked in
// `infer_match_result_type` (P0.3); it is refused before inference.
#[test]
fn a_constructor_past_the_last_index_is_e0022_not_a_panic() {
    let errors = elab_err(
        r#"
        type OptWrong = NadaWrong | AlgoWrong(Nat)
        type ThreeWrong = TaWrong(Nat) | TbWrong(Nat) | TcWrong(Nat)
        fn g(o: OptWrong) -> Nat {
            let y = match o { AlgoWrong(n) => n, TcWrong(k) => k, _ => 0 };
            y
        }
    "#,
    );
    assert!(only_e0022(&errors).message.contains("`ThreeWrong`"));
}

// 4.10.26d AC1 (D1 a): only a NULLARY constructor's name is read as that
// constructor; a bare name of a payload constructor stays a binder, as a
// name no constructor owns does (3.10.26l).
#[test]
fn a_bare_payload_constructor_name_stays_a_binding() {
    elab_ok(&format!(
        "{WRONG_TYPES}
        fn f(o: OptWrong) -> Nat {{ match o {{ AlgoWrong(n) => n, PickWrong => 0 }} }}"
    ));
}

// 4.10.26d AC1: a binder spelled like another type's nullary constructor.
#[test]
fn a_nullary_constructor_of_another_type_is_e0022() {
    let source = format!(
        "{WRONG_TYPES}
        fn f(o: OptWrong) -> Nat {{ match o {{ AlgoWrong(n) => n, SkipWrong => 0 }} }}"
    );
    let errors = elab_err(&source);
    let err = only_e0022(&errors);
    assert_eq!(spanned(&source, err), "SkipWrong");
    assert!(err.message.contains("`OtherWrong`"), "{}", err.message);
}

// 4.10.26d AC1: nested, by both routes — one arm (`resolve_pattern_ctor`)
// and two arms under one outer constructor (`nested_match.rs`).
#[test]
fn a_nested_constructor_of_another_type_is_e0022() {
    for arms in [
        "Some(PickWrong(k)) => k, _ => 0",
        "Some(AlgoWrong(k)) => k, Some(PickWrong(k)) => k, _ => 0",
    ] {
        let source = format!(
            "{WRONG_TYPES}
            type Option<T> = None | Some(T)
            fn f(o: Option<OptWrong>) -> Nat {{ match o {{ {arms} }} }}"
        );
        let errors = elab_err(&source);
        let err = only_e0022(&errors);
        assert!(
            err.message.contains("`OtherWrong`"),
            "{arms}: {}",
            err.message
        );
        assert_eq!(spanned(&source, err), "PickWrong", "{arms}");
    }
}

// 4.10.26d AC1: an inner `match` over a payload is a match like any other.
#[test]
fn an_inner_match_on_a_payload_is_e0022() {
    let source = format!(
        "{WRONG_TYPES}
        type Option<T> = None | Some(T)
        fn f(o: Option<OptWrong>) -> Nat {{
            match o {{ Some(x) => match x {{ PickWrong(k) => k, _ => 0 }}, _ => 0 }}
        }}"
    );
    assert!(only_e0022(&elab_err(&source))
        .message
        .contains("`OtherWrong`"));
}

// 4.10.26d AC1: the rule is not too strict.
#[test]
fn a_generic_instance_still_matches() {
    elab_ok(
        r#"
        type Option<T> = None | Some(T)
        type Pair = MkPair(Nat, Bool)
        fn f(o: Option<Nat>) -> Nat { match o { Some(n) => n, None => 0 } }
        fn g(o: Option<Pair>) -> Nat { match o { Some(MkPair(n, _)) => n, _ => 0 } }
    "#,
    );
}

#[test]
fn a_recursive_type_still_matches() {
    elab_ok(
        r#"
        type List<T> = Nil | Cons(T, List<T>)
        fn head(xs: List<Nat>) -> Nat { match xs { Cons(h, _) => h, Nil => 0 } }
        fn first_of(xs: List<Nat>) -> Nat { match xs { Cons(s, _) => s, _ => 0 } }
        fn second(xs: List<Nat>) -> Nat { match xs { Cons(_, rest) => first_of(rest), _ => 0 } }
    "#,
    );
}

#[test]
fn a_flat_type_still_matches() {
    elab_ok(
        r#"
        type Tri = Lo | Mid(Nat) | Hi
        fn f(t: Tri) -> Nat { match t { Mid(n) => n, Lo => 0, Hi => 1 } }
    "#,
    );
}

// 4.10.26d AC1: an imported type reaches the check by its module path.
#[test]
fn an_imported_type_still_matches() {
    let _exclusive = crate::comparator::requests::TEST_EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::TempDir::new().expect("temp dir");
    std::fs::write(
        dir.path().join("shapes.tg"),
        "pub type Shape = Dot(Nat) | Blank\npub type Tri = Lo | Mid(Nat) | Hi\n",
    )
    .expect("write shapes");
    let main = dir.path().join("main.tg");
    std::fs::write(
        &main,
        "pub mod shapes;\nuse shapes::{Shape, Dot, Blank, Tri, Lo, Mid, Hi};\n\
         fn f(s: Shape) -> Nat { match s { Dot(n) => n, Blank => 0 } }\n\
         fn g(t: Tri) -> Nat { match t { Mid(n) => n, _ => 0 } }\n\
         fn main() -> Nat { f(Dot(1)) }\n",
    )
    .expect("write main");
    let project = crate::driver::elaborate_project(&main, false, 20, None);
    assert!(project.is_ok(), "an imported type's match must check clean");
    // The scrutinee is read, not admitted unread: a flat type's constructor
    // over the imported binary sum is still refused.
    std::fs::write(
        &main,
        "pub mod shapes;\nuse shapes::{Shape, Dot, Mid};\n\
         fn h(s: Shape) -> Nat { match s { Mid(n) => n, _ => 0 } }\n\
         fn main() -> Nat { h(Dot(1)) }\n",
    )
    .expect("rewrite main");
    let refused = crate::driver::elaborate_project(&main, false, 20, None);
    assert!(
        refused.is_err(),
        "another imported type's constructor must be refused"
    );
}

// 4.10.26d AC1 (P0.4): `let x: SameShape = o` checks today, so a same-shaped
// type's constructor is admitted too — every payload has its binder's type.
#[test]
fn a_same_shaped_type_follows_the_let_rule() {
    elab_ok(
        r#"
        type OptWrong = NadaWrong | AlgoWrong(Nat)
        type SameShape = SNone | SSome(Nat)
        fn by_let(o: OptWrong) -> SameShape { let x: SameShape = o; x }
        fn by_match(o: OptWrong) -> Nat { match o { SSome(k) => k, _ => 0 } }
    "#,
    );
}

/// §1.1's types for the type-parameter rows (ADR 13.10.26i).
const PARAM_TYPES: &str = r#"
        type Wrap = MkWrap(Nat)
        type Pair2 = MkP(Nat, Nat)
        type Two = One(Nat) | Zwei(Bool)
        type Opt<T> = Som(T) | Non
"#;

// 13.10.26i AC2: a constructor pattern over a type parameter is E0022 at the
// constructor — top-level (one-case and product), `if let`, nested under a
// generic payload, and the many-case form that was spanned at 1:1.
#[test]
fn a_constructor_pattern_over_a_type_parameter_is_e0022() {
    for (body, ctor, owner) in [
        (
            "fn g<T>(t: T) -> Nat { match t { MkWrap(h) => h } }",
            "MkWrap",
            "Wrap",
        ),
        (
            "fn p<T>(t: T) -> Nat { match t { MkP(a, b) => a + b } }",
            "MkP",
            "Pair2",
        ),
        (
            "fn i<T>(t: T) -> Nat { if let MkWrap(h) = t { h } else { 0 } }",
            "MkWrap",
            "Wrap",
        ),
        (
            "fn n<T>(o: Opt<T>) -> Nat { match o { Som(MkWrap(h)) => h, Non => 0 } }",
            "MkWrap",
            "Wrap",
        ),
        (
            "fn m<T>(t: T) -> Nat { match t { One(n) => n, Zwei(b) => 0 } }",
            "One",
            "Two",
        ),
    ] {
        let source = format!("{PARAM_TYPES}\n        {body}");
        let errors = elab_err(&source);
        let err = only_e0022(&errors);
        assert_eq!(spanned(&source, err), ctor, "{body}");
        assert!(
            err.message.contains(&format!("`{owner}`")),
            "{body}: {}",
            err.message
        );
        assert!(err.message.contains("`T`"), "{body}: {}", err.message);
    }
}

// 13.10.26i AC3: the transparent payload row, a concrete instance's nested
// newtype, and a binder over a generic payload all stay accepted.
#[test]
fn a_constructor_pattern_over_a_concrete_payload_still_matches() {
    elab_ok(&format!(
        "{PARAM_TYPES}
        fn bump(n: Nat) -> Nat {{ match n {{ MkWrap(h) => h + 1 }} }}
        fn inner(o: Opt<Wrap>) -> Nat {{ match o {{ Som(MkWrap(h)) => h, Non => 0 }} }}
        fn keep<T>(o: Opt<T>, d: T) -> T {{ match o {{ Som(x) => x, Non => d }} }}"
    ));
}

// 13.10.26i D1: only a type parameter IN SCOPE is refused. A μ-marker, or a
// variable no enclosing definition binds, keeps 4.10.26d D7's admission.
#[test]
fn only_a_type_parameter_in_scope_is_a_refused_scrutinee() {
    use albite_core::{Context, Type};
    let mut ctx = Context::new();
    let mut elaborator = crate::elaborate::Elaborator::new(&mut ctx);
    let param = Type::TyVar("T".to_string());
    assert!(
        !elaborator.scrutinee_is_type_parameter(&param),
        "not yet bound"
    );
    elaborator.env.push_type_var("T".to_string());
    assert!(elaborator.scrutinee_is_type_parameter(&param));
    let marker = Type::TyVar("α_List".to_string());
    assert!(!elaborator.scrutinee_is_type_parameter(&marker));
    assert!(!elaborator.scrutinee_is_type_parameter(&Type::Nat));
}
