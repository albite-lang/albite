//! A bare nullary constructor name routes a match to the constructor
//! matcher (ADR 6.10.26k D2): it means what its `()` spelling means, and it is
//! refused where that spelling is.

use albite_core::{eval, Term, Type};

use crate::elaborate::error::ElabError;
use crate::elaborate::tests::{elab_err, elab_ok};

/// The elaborated Core term of definition `name` in `source`, spans erased:
/// a bare arm is two characters shorter than its twin.
fn core_of(source: &str, name: &str) -> Term {
    elab_ok(source)
        .into_iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
        .term
        .term
        .strip_spans()
}

fn nat_value(term: &Term) -> u64 {
    match term {
        Term::Zero => 0,
        Term::Succ(inner) => 1 + nat_value(inner),
        Term::NatLit(n) => *n,
        other => panic!("not a Nat value: {other}"),
    }
}

/// Asserts the bare-armed function elaborates to the same Core as its twin
/// with `()` on every constructor arm.
fn assert_core_matches_parenthesised(types: &str, bare: &str, parenthesised: &str) {
    let bare_core = core_of(&format!("{types}\nfn under_test{bare}"), "under_test");
    let twin_core = core_of(
        &format!("{types}\nfn under_test{parenthesised}"),
        "under_test",
    );
    assert_eq!(bare_core, twin_core, "bare: {bare}");
}

fn codes(errors: &[ElabError]) -> Vec<&'static str> {
    errors.iter().map(|e| e.kind.code()).collect()
}

/// The source text `err` underlines.
fn spanned<'s>(source: &'s str, err: &ElabError) -> &'s str {
    &source[err.span.start as usize..err.span.end as usize]
}

const COLOR: &str = "type Color = Red | Green | Blue";
const YES_NO: &str = "type B = Yes | No";

// 6.10.26k AC1's must-fail twin: the comparison refuses two different Cores,
// so the twins above are not passing vacuously.
#[test]
#[should_panic(expected = "bare:")]
fn the_core_comparison_refuses_a_different_twin() {
    assert_core_matches_parenthesised(
        YES_NO,
        "(b: B) -> Nat { match b { Yes => 1, No => 2 } }",
        "(b: B) -> Nat { match b { Yes() => 2, No() => 1 } }",
    );
}

// 6.10.26k AC1: a flat three-constructor type, bare in every arm.
#[test]
fn a_flat_bare_match_is_its_parenthesised_twin() {
    assert_core_matches_parenthesised(
        COLOR,
        "(c: Color) -> Bool { match c { Red => true, Green => true, Blue => false } }",
        "(c: Color) -> Bool { match c { Red() => true, Green() => true, Blue() => false } }",
    );
}

// 6.10.26k AC1: a binary sum, in and out of declaration order.
#[test]
fn a_binary_bare_match_is_its_parenthesised_twin_in_either_order() {
    assert_core_matches_parenthesised(
        YES_NO,
        "(b: B) -> Nat { match b { Yes => 10, No => 3 } }",
        "(b: B) -> Nat { match b { Yes() => 10, No() => 3 } }",
    );
    assert_core_matches_parenthesised(
        YES_NO,
        "(b: B) -> Nat { match b { No => 3, Yes => 10 } }",
        "(b: B) -> Nat { match b { No() => 3, Yes() => 10 } }",
    );
}

// 6.10.26k AC1: a mixed type, a bare nullary arm and a catch-all.
#[test]
fn a_mixed_bare_match_is_its_parenthesised_twin() {
    assert_core_matches_parenthesised(
        "type W = Wa(Nat) | Wb",
        "(w: W) -> Nat { match w { Wb => 2, x => 1 } }",
        "(w: W) -> Nat { match w { Wb() => 2, x => 1 } }",
    );
}

// 6.10.26k AC1: a μ-recursive scrutinee goes through `unfold_scrutinee_type`.
#[test]
fn a_recursive_bare_match_is_its_parenthesised_twin() {
    assert_core_matches_parenthesised(
        "type List<T> = Nil | Cons(T, List<T>)",
        "(xs: List<Nat>) -> Nat { match xs { Nil => 0, _ => 1 } }",
        "(xs: List<Nat>) -> Nat { match xs { Nil() => 0, _ => 1 } }",
    );
}

// 6.10.26k AC1: a one-constructor nullary type. Spelled with a leading `|`:
// `type One = One` is the recursive alias E0060 refuses.
#[test]
fn a_single_constructor_bare_match_is_its_parenthesised_twin() {
    assert_core_matches_parenthesised(
        "type Single = | One",
        "(o: Single) -> Nat { match o { One => 1 } }",
        "(o: Single) -> Nat { match o { One() => 1 } }",
    );
}

// 6.10.26k AC1 (D2 b): no expected type, and the index-0 arm is bare.
#[test]
fn an_unannotated_bare_match_infers_its_result_type() {
    assert_core_matches_parenthesised(
        COLOR,
        "(c: Color) -> Nat { let r = match c { Red => 1, Green => 2, Blue => 3 }; r }",
        "(c: Color) -> Nat { let r = match c { Red() => 1, Green() => 2, Blue() => 3 }; r }",
    );
}

// 6.10.26k AC1, the must-fail twin: an out-of-order match is read by name.
// Before D2 the fallback bound the arms by position, `f(Yes()) == 3`.
#[test]
fn an_out_of_order_bare_match_computes_by_name() {
    let source = format!("{YES_NO}\nfn f(b: B) -> Nat {{ match b {{ No => 3, Yes => 10 }} }}");
    let weigh = core_of(&source, "f");
    let yes_no_encoding = Type::sum(Type::Unit, Type::Unit);
    let on = |value: Term| nat_value(&eval(&Term::app(weigh.clone(), value)));
    assert_eq!(
        on(Term::inl(yes_no_encoding.clone(), Term::Unit)),
        10,
        "f(Yes())"
    );
    assert_eq!(on(Term::inr(yes_no_encoding, Term::Unit)), 3, "f(No())");
}

// 6.10.26k AC2: a bare match that misses a constructor is non-exhaustive.
#[test]
fn a_bare_match_missing_a_constructor_is_e0020() {
    let errors = elab_err(&format!(
        "{COLOR}\nfn f(c: Color) -> Bool {{ match c {{ Red => true, Green => true }} }}"
    ));
    assert_eq!(codes(&errors), vec!["E0020"]);
}

// 6.10.26k AC2: another type's nullary constructor, bare, is E0022.
#[test]
fn a_bare_constructor_of_another_type_is_e0022() {
    let errors = elab_err(&format!(
        "{COLOR}\n{YES_NO}\nfn f(c: Color) -> Nat {{ match c {{ Red => 1, Yes => 2, _ => 3 }} }}"
    ));
    assert_eq!(codes(&errors), vec!["E0022"]);
}

// 6.10.26k AC2, D2 case (ii): two arms over a binary sum, once read by
// position, now name `Color`'s constructor.
#[test]
fn a_foreign_bare_binder_over_a_binary_sum_is_e0022() {
    let source = format!(
        "{COLOR}\ntype P = L(Nat) | R(Nat)\nfn f(p: P) -> Nat {{ match p {{ Red => 1, x => 2 }} }}"
    );
    let errors = elab_err(&source);
    assert_eq!(codes(&errors), vec!["E0022"]);
    assert_eq!(spanned(&source, &errors[0]), "Red");
}

// 6.10.26k AC2: a `Nat` match keeps a bare capitalised arm as a binder.
#[test]
fn a_nat_match_keeps_a_bare_capitalised_binder() {
    elab_ok(&format!(
        "{COLOR}\nfn f(n: Nat) -> Nat {{ match n {{ 0 => 1, Red => Red }} }}"
    ));
}

// 6.10.26k AC2: a `String` scrutinee keeps E0100 for a plain binder, and a
// binder spelled like a nullary constructor is that constructor (D2 case i).
#[test]
fn a_string_scrutinee_is_e0100_or_e0022_by_the_binder() {
    let plain = elab_err(&format!(
        "{COLOR}\nfn f(s: String) -> Nat {{ match s {{ other => 1 }} }}"
    ));
    assert_eq!(codes(&plain), vec!["E0100"]);
    let named = elab_err(&format!(
        "{COLOR}\nfn f(s: String) -> Nat {{ match s {{ Red => 1 }} }}"
    ));
    assert_eq!(codes(&named), vec!["E0022"]);
}

// 6.10.26k AC2, D2 (c): a bare name for this type's constructor with fields
// is `Wb()`'s arity error, spanned on the name, not a first-arm catch-all.
#[test]
fn a_bare_constructor_with_fields_is_an_arity_error() {
    for arms in [
        "Wb => 2, Wa => 1",
        "Wb => 2, Wa(x) => 1",
        "Wb() => 2, Wa(x) => 1",
    ] {
        let source =
            format!("type W = Wa(Nat) | Wb(Nat)\nfn f(w: W) -> Nat {{ match w {{ {arms} }} }}");
        let errors = elab_err(&source);
        assert_eq!(codes(&errors), vec!["E0012"], "arms: {arms}");
        assert!(
            spanned(&source, &errors[0]).starts_with("Wb"),
            "arms: {arms}"
        );
    }
}

// 6.10.26k AC2, D2 (c): only the scrutinee's own constructors with fields are
// refused bare. Spelled like another type's, the name stays a binder: over a
// `String`, today's E0100; over a two-constructor sum that is not `W`'s
// encoding, today's two-arm match, rather than E0022.
#[test]
fn a_bare_name_for_a_foreign_constructor_with_fields_stays_a_binder() {
    let errors =
        elab_err("type W = Wa(Nat) | Wb(Nat)\nfn f(s: String) -> Nat { match s { Wa => 1 } }");
    assert_eq!(codes(&errors), vec!["E0100"]);
    elab_ok(
        "type W = Wa(Nat) | Wb(Nat)\ntype P = L(String) | R(String)\n\
         fn f(p: P) -> Nat { match p { Wa => 1, other => 2 } }",
    );
}

// 6.10.26k D2 (a): a leading plain binder is a catch-all, never the arm the
// match takes its type from; the constructor arm after it still decides.
#[test]
fn a_leading_binder_is_not_the_owner_arm() {
    // `elab_ok_with_warnings` panics on an error, which is the refusal a
    // binder taken as the owner would raise (undefined constructor `x`).
    let (_, warnings) = crate::elaborate::tests::elab_ok_with_warnings(
        "type W = Wa(Nat) | Wb\nfn f(w: W) -> Nat { match w { x => 1, Wb() => 2 } }",
    );
    assert_eq!(
        warnings.len(),
        1,
        "the arm after the catch-all: {warnings:?}"
    );
}

// 6.10.26k AC2, D2 case (iii): a bare name for a private constructor of
// another module is the private-item refusal its `()` spelling gets.
#[test]
fn a_bare_private_constructor_is_refused_as_its_parenthesised_twin() {
    let _exclusive = crate::comparator::requests::TEST_EXCLUSIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::TempDir::new().expect("temp dir");
    std::fs::write(dir.path().join("helper.tg"), "type Color = Red | Blue\n")
        .expect("write helper");
    let main = dir.path().join("main.tg");
    for arm in ["Red", "Red()"] {
        std::fs::write(
            &main,
            format!(
                "mod helper;\ntype Mine = Ay | Bee\n\
                 fn f(m: Mine) -> Nat {{ match m {{ {arm} => 1, _ => 2 }} }}\n\
                 fn main() -> Nat {{ f(Ay()) }}\n"
            ),
        )
        .expect("write main");
        // The pipeline error carries no code; the record (E0016, "constructor
        // `Red` is private") is on stderr. Before D2 the bare arm was a binder
        // and this program checked clean.
        let refused = crate::driver::elaborate_project(&main, false, 20, None);
        assert!(refused.is_err(), "`{arm}` must be refused, not bound");
    }
}
