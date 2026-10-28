//! `ring`'s tests (ADR 4.10.26g AC 2).
//!
//! This file holds the pure half: the normal form and the plan, over
//! hand-built Core — the normal form here, the plan in `plan.rs`.
//! `elaboration.rs` runs the tactic through the elaborator and the kernel,
//! refusals included.

mod elaboration;
mod plan;
mod power;

use albite_core::Term;

use super::normal_form::{polynomial, render, summand_key, AtomOrder, SummandKey};

pub(super) fn var(name: &str) -> Term {
    Term::var(name)
}

pub(super) fn add(left: Term, right: Term) -> Term {
    Term::nat_add(left, right)
}

pub(super) fn mul(left: Term, right: Term) -> Term {
    Term::nat_mul(left, right)
}

/// `k` as the elaborator spells it: `succ^k zero` (P0.2).
pub(super) fn num(k: u64) -> Term {
    (0..k).fold(Term::Zero, |n, _| Term::Succ(Box::new(n)))
}

/// `f(x)`, an application: an atom to `ring`.
pub(super) fn apply(f: &str, x: Term) -> Term {
    Term::app(Term::global(f), x)
}

fn atoms_of(left: &Term, right: &Term) -> AtomOrder {
    let mut atoms = AtomOrder::default();
    atoms.extend_from(left);
    atoms.extend_from(right);
    atoms
}

/// The rendered normal form of `term` over the goal's atoms.
fn spelled(term: &Term, atoms: &AtomOrder) -> String {
    render(&polynomial(term, atoms).expect("indexed"), atoms)
}

// ADR 4.10.26g AC2
#[test]
fn atoms_are_ordered_by_first_appearance() {
    let (n, m, k) = (var("n"), var("m"), var("k"));
    let left = add(mul(m.clone(), n.clone()), k.clone());
    let right = add(k.clone(), mul(n.clone(), var("z")));
    let atoms = atoms_of(&left, &right);
    let order: Vec<Option<usize>> = [&m, &n, &k, &var("z")]
        .iter()
        .map(|atom| atoms.index(atom))
        .collect();
    assert_eq!(order, [Some(0), Some(1), Some(2), Some(3)]);
    // A factor's key is its atom's index; a monomial sorts by degree, then
    // atoms; the constant sorts last.
    assert_eq!(
        summand_key(&mul(n.clone(), m.clone()), &atoms),
        Some(SummandKey::Monomial(2, vec![0, 1]))
    );
    assert!(summand_key(&k, &atoms) < summand_key(&mul(m.clone(), m), &atoms));
    assert_eq!(summand_key(&num(3), &atoms), Some(SummandKey::Constant));
    assert!(summand_key(&k, &atoms) < Some(SummandKey::Constant));
    // A sum is not a summand yet: an earlier phase has work there.
    assert_eq!(summand_key(&add(n, k), &atoms), None);
}

// ADR 4.10.26g AC2: D2's one spelling, whichever way a polynomial is written.
#[test]
fn equal_polynomials_have_one_spelling() {
    let (n, m, k) = (var("n"), var("m"), var("k"));
    let factored = mul(add(n.clone(), m.clone()), k.clone());
    let spread = add(mul(k.clone(), m.clone()), mul(n.clone(), k.clone()));
    let atoms = atoms_of(&factored, &spread);
    assert_eq!(polynomial(&factored, &atoms), polynomial(&spread, &atoms));
    assert_eq!(spelled(&factored, &atoms), "n * k + m * k");
    // Spelling (i): a coefficient is that many copies, and the constant —
    // its numerals summed — comes last.
    let doubled = add(mul(num(2), n.clone()), add(num(2), num(3)));
    let atoms = atoms_of(&doubled, &n);
    assert_eq!(spelled(&doubled, &atoms), "n + n + 5");
    // Degree first: the square after the linear term.
    let square = add(mul(n.clone(), n.clone()), n.clone());
    assert_eq!(spelled(&square, &atoms_of(&square, &n)), "n + n * n");
    assert_eq!(spelled(&num(0), &AtomOrder::default()), "0");
    // A large coefficient is spelled with its numeral on the right.
    let many = mul(n.clone(), Term::NatLit(1000));
    assert_eq!(spelled(&many, &atoms_of(&many, &n)), "n * 1000");
    // The boundary: sixteen copies are still spelled out, seventeen are not.
    let at_most = mul(n.clone(), Term::NatLit(16));
    assert_eq!(
        spelled(&at_most, &atoms_of(&at_most, &n)),
        vec!["n"; 16].join(" + ")
    );
    let past = mul(n.clone(), Term::NatLit(17));
    assert_eq!(spelled(&past, &atoms_of(&past, &n)), "n * 17");
}

// ADR 4.10.26g AC2: anything but `+`, `*` and a numeral is an atom (D1).
#[test]
fn a_non_arithmetic_subterm_is_an_atom() {
    let (n, m) = (var("n"), var("m"));
    let sub_one = Term::nat_sub(n.clone(), num(1));
    let call = apply("f", add(n.clone(), m.clone()));
    let left = add(sub_one.clone(), call.clone());
    let right = add(call.clone(), sub_one.clone());
    let atoms = atoms_of(&left, &right);
    assert_eq!(atoms.index(&sub_one), Some(0));
    assert_eq!(atoms.index(&call), Some(1));
    // The atom's insides are not `ring`'s: `n` and `m` are not atoms here.
    assert_eq!(atoms.index(&n), None);
    assert_eq!(polynomial(&left, &atoms), polynomial(&right, &atoms));
    // Its numerals print as decimals.
    assert_eq!(spelled(&left, &atoms), "(n - 1) + (global:f (n + m))");
    // `(n - 1) + 1 = n` is false at 0, and the polynomials say so.
    let shifted = add(sub_one, num(1));
    let atoms = atoms_of(&shifted, &n);
    assert_ne!(polynomial(&shifted, &atoms), polynomial(&n, &atoms));
}
