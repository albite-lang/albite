//! A dependent pair's `let` pattern refuses what it must (ADR 6.10.26c D1,
//! D3, D4, D7): each refusal pinned by its code, `mustfail_pair_pattern.tg`'s
//! three cases among them. The lowerings are in `pair_pattern.rs`.

use super::first_code;
use super::pair_pattern::{admitted, LE};
use crate::elaborate::tests::elab_err;

/// D3's note, as `blocks_tuples/sigma.rs` writes it: the text, not only the
/// code, tells this E0011 apart.
const ESCAPING_WITNESS_NOTE: &str =
    "the type of this block depends on the unpacked witness; state the expected type";

// 6.10.26c AC1 (D4): a `Sigma` inside a `Product` pattern stays E0014, and
// two `let`s spell it.
#[test]
fn a_sigma_inside_a_product_pattern_is_refused_but_two_lets_work() {
    assert_eq!(
        first_code(&format!(
            "{LE}fn nested(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ let ((k, e), y) = (h, 3); k }}"
        )),
        "E0014"
    );
    admitted(
        "fn split(n: Nat, m: Nat, h: Le<n, m>) -> Nat { let (p, y) = (h, 3); let (k, e) = p; k + y }",
        "split",
    );
}

// 6.10.26c AC1 (D4): `match` on a dependent pair stays refused, with the code
// it had before this ADR (E0100; D7 expected E0014, recorded in the Outcome).
#[test]
fn a_match_tuple_pattern_over_a_sigma_is_refused() {
    assert_eq!(
        first_code(&format!(
            "{LE}fn matched(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ match h {{ (k, e) => k }} }}"
        )),
        "E0100"
    );
}

// 6.10.26c AC1, AC3 (D1): three parts over a `Sigma` is E0014 with D1's message.
#[test]
fn three_parts_over_a_sigma_are_refused() {
    let errors = elab_err(&format!(
        "{LE}pub theorem three(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> {{ let (k, e, z) = h; (k, e) }}"
    ));
    assert_eq!(errors[0].kind.code(), "E0014");
    assert!(
        errors[0]
            .to_string()
            .contains("a dependent pair has two parts"),
        "{}",
        errors[0]
    );
}

// 6.10.26c AC3 (D7): the rebuilt pair at the wrong witness is a mismatch.
#[test]
fn a_rebuilt_pair_at_the_wrong_witness_is_refused() {
    assert_eq!(
        first_code(&format!(
            "{LE}pub theorem wrong(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> {{ let (k, e) = h; (k + 1, e) }}"
        )),
        "E0010"
    );
}

/// The first error of `body` (elaborated after `LE`) is D3's E0011 and note.
fn assert_escaping_witness(body: &str) {
    let errors = elab_err(&format!(
        "{LE}fn esc(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ let unpacked = {{ {body} }}; 0 }}"
    ));
    assert_eq!(errors[0].kind.code(), "E0011", "{body}");
    assert!(
        errors[0]
            .notes
            .iter()
            .any(|n| n.message == ESCAPING_WITNESS_NOTE),
        "{body}: {:?}",
        errors[0].notes
    );
}

// 6.10.26c AC1, AC3 (D3): an inferred block type naming `k` escapes.
#[test]
fn an_escaping_witness_is_refused() {
    assert_escaping_witness("let (k, e) = le_rebuilt(n, m, h); e");
}

// 6.10.26c AC1 (D3): with the witness unbound, a block typed by the evidence
// alone escapes too — its type names `fst(__tup)`, through `e`'s own type.
#[test]
fn an_escaping_evidence_is_refused() {
    assert_escaping_witness("let (_, e) = le_rebuilt(n, m, h); e");
}

// The escape helper is not vacuous: a block that does not escape fails it.
#[test]
#[should_panic(expected = "Expected elaboration error")]
fn the_escape_helper_fails_on_a_block_that_does_not_escape() {
    assert_escaping_witness("let (k, e) = le_rebuilt(n, m, h); 0");
}

// 6.10.26c (D2): a witness name the pair's body already uses is refused
// rather than captured by `B[x := k]`.
#[test]
fn a_witness_name_the_pair_mentions_is_refused() {
    assert_eq!(
        first_code(&format!(
            "{LE}fn capture(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ let (n, e) = h; n }}"
        )),
        "E0021"
    );
}

// 6.10.26c AC1 (D1): the witness is a name or `_`, and the evidence is a
// name, `_` or a tuple; any other pattern part is E0021.
#[test]
fn an_unsupported_pattern_part_is_refused() {
    assert_eq!(
        first_code(&format!(
            "{LE}fn tuple_witness(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ let ((a, b), e) = h; 0 }}"
        )),
        "E0021"
    );
    assert_eq!(
        first_code(&format!(
            "{LE}fn literal_evidence(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ let (k, 0) = h; k }}"
        )),
        "E0021"
    );
}
