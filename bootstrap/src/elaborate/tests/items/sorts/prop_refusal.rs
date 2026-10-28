//! Q5's four twins and the `Prop`/`Type` refusal (ADR 4.10.26i D3, D7, AC 3).
//! Each is asserted by its own code, so a twin that fails for another reason
//! goes red.

use super::first_code;
use crate::parse;

// 4.10.26i AC3 (Q5 twin 1): a data type declared a proposition is E0067.
#[test]
fn a_data_body_declared_prop_is_refused() {
    assert_eq!(first_code("type Bad(n: Nat): Prop = Nat"), "E0067");
    // ⊤ is `0 == 0`, never `Unit`: a two-constructor enum is data too.
    assert_eq!(first_code("type Dir: Prop = Unit + Unit"), "E0067");
}

// 4.10.26i AC3 (Q5 twin 2): the `exists` binder is a term, not a type.
#[test]
fn an_escaping_binder_is_an_unknown_type() {
    assert_eq!(
        first_code("type Esc(n: Nat): Prop = exists k: Nat. k"),
        "E0002"
    );
}

// 4.10.26i AC3 (Q5 twin 3, D3): no term has type `Prop`, anywhere.
#[test]
fn prop_in_a_type_position_is_refused() {
    for source in [
        "fn f(p: Prop) -> Nat { 0 }",
        "fn r() -> Prop { sorry }",
        "fn l() -> Nat { let p: Prop = sorry; 0 }",
        "fn e(h: Eq<Prop, 0, 0>) -> Nat { 0 }",
    ] {
        assert_eq!(first_code(source), "E0066", "{source}");
    }
}

// 4.10.26i AC3 (D3): `Type` in a type position is the same refusal.
#[test]
fn type_in_a_type_position_is_refused() {
    assert_eq!(first_code("fn f(x: Type) -> Nat { 0 }"), "E0066");
}

// 4.10.26i AC3 (Q5 twin 4): a `Prop`-sorted type variable. 6.10.26h D1 gave
// it a spelling — the bound parses — so it is refused by elaboration, E0069.
#[test]
fn a_prop_sorted_type_variable_is_refused_by_elaboration() {
    let (_, errors) = parse("fn g<P: Prop>(h: P) -> P { h }");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(first_code("fn g<P: Prop>(h: P) -> P { h }"), "E0069");
}
