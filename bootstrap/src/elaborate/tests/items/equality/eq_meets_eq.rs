//! An `Eq` meets an `Eq` in check mode by endpoint conversion (ADR 30.9.26e
//! AC 6, D5), and only there: a refusal keeps its message byte for byte.

use crate::elaborate::conversion::eq_fallback;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::ElabError;
use albite_core::{Term, Type};

/// `0 + n == n` by induction, and `n + 0 == n` by `refl`: the two statements
/// every case below instantiates.
const LEMMAS: &str = "fn zero_add(n: Nat) -> Eq<Nat, 0 + n, n> { \
    natind(|k: Nat| Eq<Nat, 0 + k, k>, refl, fn(k: Nat, ih) => cong(|x: Nat| x + 1, ih), n) }\n\
    fn add_zero(n: Nat) -> Eq<Nat, n + 0, n> { refl }";

fn with_lemmas(rest: &str) -> String {
    format!("{LEMMAS}\n{rest}")
}

fn only_error(source: &str) -> ElabError {
    let errors = elab_err(source);
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors.into_iter().next().unwrap()
}

// ── Accepted: the three check-mode sites a citation reaches ─────────────────

// 30.9.26e AC6: a `let` annotation that differs from the proof by computation.
#[test]
fn an_eq_meets_an_eq_at_a_let_annotation() {
    elab_ok(&with_lemmas(
        "fn at(n: Nat) -> Bool { let _q: Eq<Nat, n + 0, 0 + n> = sym(zero_add(n)); true }",
    ));
}

// 30.9.26e AC6: an argument whose parameter is an `Eq`.
#[test]
fn an_eq_meets_an_eq_at_a_function_argument() {
    elab_ok(&with_lemmas(
        "fn takes(n: Nat, p: Eq<Nat, n + 0, 0 + n>) -> Bool { true }\n\
         fn at(n: Nat) -> Bool { takes(n, sym(zero_add(n))) }",
    ));
}

// 30.9.26e AC6: a body that is a bare citation, against its declared return.
#[test]
fn an_eq_meets_an_eq_at_a_declared_return() {
    elab_ok(&with_lemmas(
        "fn at(n: Nat) -> Eq<Nat, n + 0, 0 + n> { sym(zero_add(n)) }",
    ));
}

// ── Refused: the endpoints or the base type do not meet ─────────────────────

// 30.9.26e AC6: an endpoint that does not convert keeps today's E0010.
#[test]
fn an_eq_with_a_wrong_endpoint_is_refused() {
    let error = only_error(&with_lemmas(
        "fn at(n: Nat) -> Bool { let _q: Eq<Nat, n + 1, 0 + n> = sym(zero_add(n)); true }",
    ));
    assert_eq!(error.kind.code(), "E0010");
    assert_eq!(
        error.message,
        "expected `Eq Nat (n + (succ zero)) (zero + n)`, found `Eq Nat n (zero + n)`"
    );
}

// 30.9.26e AC6: a different base type is refused before any endpoint is compared.
#[test]
fn an_eq_over_a_different_base_type_is_refused() {
    let error = only_error(
        "fn yes() -> Eq<Bool, true, true> { refl }\n\
         fn at() -> Bool { let _q: Eq<Nat, 0, 0> = yes(); true }",
    );
    assert_eq!(error.kind.code(), "E0010");
    assert_eq!(
        error.message,
        "expected `Eq Nat zero zero`, found `Eq Bool true true`"
    );
}

// 30.9.26e AC6: known-refused, flips with full Phase 2b — an `Eq` nested in a
// product is compared structurally.
// 7.10.26i AC3, D3: still refused — its endpoints are open (`n + 0`), and only
// a closed nested pair converts. It flips only with full Phase 2b.
#[test]
fn a_product_of_eqs_is_still_compared_structurally() {
    let error = only_error(&with_lemmas(
        "fn both(n: Nat) -> (Eq<Nat, n + 0, n>, Eq<Nat, n + 0, n>) { (add_zero(n), add_zero(n)) }\n\
         fn at(n: Nat) -> Bool { let _p: (Eq<Nat, n, n>, Eq<Nat, n, n>) = both(n); true }",
    ));
    assert_eq!(error.kind.code(), "E0010");
}

// 7.10.26i AC1, D4: known-refused — a product of CLOSED `Eq`s is narrowed out,
// because `albite1` refuses it too (P0.3, E0001), so accepting it here would
// open a single-compiler verdict. Parked with the self-host as its owner.
#[test]
fn a_product_of_closed_eqs_is_refused_on_both_compilers() {
    let error = only_error(
        "fn both() -> (Eq<Nat, 2 + 3, 5>, Nat) { (refl, 0) }\n\
         fn at() -> Bool { let _p: (Eq<Nat, 5, 5>, Nat) = both(); true }",
    );
    assert_eq!(error.kind.code(), "E0010");
}

// ── A closed sum in a family argument meets its numeral (7.10.26i) ──────────

const LE: &str = "type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n\
    fn le_add(n: Nat, m: Nat) -> Le<n, n + m> { (m, refl) }";

fn with_le(rest: &str) -> String {
    format!("{LE}\n{rest}")
}

// 7.10.26i AC1: at a `let` annotation, and at a function argument.
#[test]
fn a_closed_sum_in_a_family_argument_meets_its_numeral() {
    elab_ok(&with_le(
        "fn at() -> Bool { let _h: Le<2, 5> = le_add(2, 3); true }",
    ));
    elab_ok(&with_le(
        "fn takes(h: Le<2, 5>) -> Bool { true }\n\
         fn at() -> Bool { takes(le_add(2, 3)) }",
    ));
}

// 7.10.26i AC1: unequal numerals keep today's E0010.
#[test]
fn a_closed_sum_at_a_wrong_numeral_is_refused() {
    let error = only_error(&with_le(
        "fn at() -> Bool { let _h: Le<2, 6> = le_add(2, 3); true }",
    ));
    assert_eq!(error.kind.code(), "E0010");
}

// ── Only when BOTH sides are `Eq` ───────────────────────────────────────────

// 30.9.26e AC6: an inferred `Eq` against a `Nat`, and the reverse, keep
// today's exact message.
#[test]
fn an_eq_against_a_non_eq_keeps_its_message() {
    let error = only_error(&with_lemmas(
        "fn at(n: Nat) -> Bool { let _x: Nat = add_zero(n); true }",
    ));
    assert_eq!(error.kind.code(), "E0010");
    assert_eq!(error.message, "expected `Nat`, found `Eq Nat (n + zero) n`");
    let error = only_error("fn at(n: Nat) -> Bool { let _q: Eq<Nat, n, n> = n; true }");
    assert_eq!(error.kind.code(), "E0010");
    assert_eq!(error.message, "expected `Eq Nat n n`, found `Nat`");
}

// 30.9.26e AC6: a non-`Eq` mismatch is untouched.
#[test]
fn a_non_eq_mismatch_keeps_its_message() {
    let error = only_error("fn at() -> Bool { let _x: Nat = true; true }");
    assert_eq!(error.message, "expected `Nat`, found `Bool`");
}

// 30.9.26e AC6: the retry gate is `&&`, never `||` — one `Eq` side is not enough.
#[test]
fn eq_fallback_needs_both_sides_to_be_eq() {
    let eq = Type::Eq(
        Box::new(Type::Nat),
        Box::new(Term::Zero),
        Box::new(Term::Zero),
    );
    assert!(eq_fallback(&eq, &eq));
    assert!(!eq_fallback(&eq, &Type::Nat));
    assert!(!eq_fallback(&Type::Nat, &eq));
    assert!(!eq_fallback(&Type::Nat, &Type::Nat));
}

// 30.9.26e AC6: after the extraction, `check_case_against` still sends a case
// whose dependent motive is not a bare `Eq` (here a product holding one) to
// plain `check` — inferring `(refl, 0)` would fail, and `check_eq_against`
// would refuse a product as "not both `Eq`".
#[test]
fn a_natind_case_under_a_non_eq_motive_is_checked_not_converted() {
    elab_ok(
        "fn pairs(n: Nat) -> Bool { \
         let _p: (Eq<Nat, n + 0, n>, Nat) = \
         natind(|k: Nat| (Eq<Nat, k + 0, k>, Nat), (refl, 0), fn(j: Nat, ih) => (refl, 1), n); \
         true }",
    );
}

// 30.9.26e AC6: a poisoned side passes through, so its fault is reported once;
// any other non-`Eq` side is the caller's mismatch.
#[test]
fn check_eq_against_passes_poison_through() {
    let ctx = Box::leak(Box::new(albite_core::Context::new()));
    let mut elab = crate::elaborate::Elaborator::new(ctx);
    let eq = Type::Eq(
        Box::new(Type::Nat),
        Box::new(Term::Zero),
        Box::new(Term::Zero),
    );
    let refused =
        |_: &crate::elaborate::Elaborator| -> ElabError { panic!("poison must not be refused") };
    assert!(elab.check_eq_against(&Type::Error, &eq, refused).is_ok());
    assert!(elab.check_eq_against(&eq, &Type::Error, refused).is_ok());
    let mismatch = |_: &crate::elaborate::Elaborator| {
        ElabError::type_mismatch(crate::span::Span::new(0, 0), Type::Nat, Type::Bool)
    };
    assert!(elab.check_eq_against(&Type::Bool, &eq, mismatch).is_err());
}
