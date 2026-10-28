//! `exists` elaborates to a dependent pair (ADR 4.10.26h AC 3, D3, D5).
//!
//! The binder refusals reject a signature, which stops collection for the
//! whole file, so they are pinned here rather than in a `.tg` suite. The
//! folder is named for 4.10.26i's sort judgment, whose cases join it.

mod dependent_function;
mod disjunction;
mod families;
mod family_arguments;
mod pair_pattern;
mod pair_pattern_refusals;
mod predicate_binders;
mod prop_refusal;
mod refutation;

use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::CoreDef;
use albite_core::{Term, Type};

fn def<'d>(defs: &'d [CoreDef], name: &str) -> &'d CoreDef {
    defs.iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"))
}

fn first_code(source: &str) -> &'static str {
    elab_err(source)[0].kind.code()
}

/// `term` with every span and `Annot` peeled off, down to its first non-λ.
fn under_lambdas(term: &Term) -> &Term {
    match term {
        Term::Lambda(_, _, body) | Term::Spanned(body, _) => under_lambdas(body),
        other => other,
    }
}

const ZERO_WITNESS: &str =
    "theorem zero_witness(n: Nat) -> exists k: Nat. n + k == n { (0, refl) }";

// 4.10.26h AC3: the statement is `Π n. Σ k:Nat. Eq<Nat, n + k, n>`.
#[test]
fn an_exists_statement_is_a_sigma() {
    let defs = elab_ok(ZERO_WITNESS);
    let Type::Pi(_, _, statement) = &def(&defs, "zero_witness").ty else {
        panic!("expected a Pi");
    };
    assert!(
        matches!(&**statement, Type::Sigma(k, dom, body)
            if k == "k" && **dom == Type::Nat && matches!(**body, Type::Eq(..))),
        "{statement:?}"
    );
}

// 4.10.26h AC3 (D5): the proof is the pair annotated with its `Sigma`.
#[test]
fn a_tuple_checked_against_an_exists_is_an_annotated_pair() {
    let defs = elab_ok(ZERO_WITNESS);
    let core = under_lambdas(&def(&defs, "zero_witness").term.term);
    let Term::Annot(pair, Type::Sigma(..)) = core else {
        panic!("expected Annot(_, Sigma), got {core:?}");
    };
    assert!(matches!(pair.strip_spans(), Term::Pair(..)));
}

// 4.10.26h AC3: an unannotated tuple stays a product, so nothing changed for it.
#[test]
fn a_tuple_checked_against_a_product_is_unchanged() {
    let defs = elab_ok("fn p() -> (Nat, Bool) { (0, true) }");
    let core = under_lambdas(&def(&defs, "p").term.term);
    assert!(matches!(core.strip_spans(), Term::Pair(..)), "{core:?}");
}

// 4.10.26h AC3 (D5): the binder is a term, so used as a type it is unknown.
#[test]
fn an_exists_binder_used_as_a_type_is_unknown() {
    assert_eq!(
        first_code("theorem esc(n: Nat) -> exists k: Nat. k { 0 }"),
        "E0002"
    );
}

// 4.10.26h AC3 (D3, D8): a binder ranging over a proof is E0065.
#[test]
fn an_exists_binder_of_a_proof_is_refused() {
    assert_eq!(
        first_code("theorem bad() -> exists h: Eq<Nat, 0, 0>. Eq<Nat, 0, 0> { (refl, refl) }"),
        "E0065"
    );
    // `Prop` itself no longer elaborates: it is a sort, E0066 (4.10.26i D3).
    assert_eq!(
        first_code("theorem bad() -> exists h: Prop. Eq<Nat, 0, 0> { (refl, refl) }"),
        "E0066"
    );
}

// 4.10.26h AC3 (D8): a wrong witness is a refused `refl`, E0071.
#[test]
fn a_wrong_witness_is_a_refused_refl() {
    assert_eq!(
        first_code("theorem wrong(n: Nat) -> exists k: Nat. n + k == n { (1, refl) }"),
        "E0071"
    );
}

// 4.10.26h AC3 (D8): a proof that is no tuple is a mismatch.
#[test]
fn a_non_tuple_proof_of_an_exists_is_a_mismatch() {
    assert_eq!(
        first_code("theorem bare(n: Nat) -> exists k: Nat. n + k == n { refl }"),
        "E0070"
    );
    assert_eq!(
        first_code("theorem num(n: Nat) -> exists k: Nat. n + k == n { 0 }"),
        "E0010"
    );
}
