//! Numeral-insensitive comparison after δ (ADR 28.9.26d D8).
//!
//! A literal `2` may be elaborated as a `Succ` chain while a computed `2` comes
//! back as whatever `nat_smart` chose, so every closed numeral subterm is
//! rewritten to one spelling before the discharged sides are compared. The
//! elaboration-time fast path keeps its byte-identical `==`.

use crate::eval::term_to_nat;
use crate::Term;

/// Rewrite every closed numeral subterm (`Zero`/`Succ` chain or `NatLit`) of
/// `term` to `Term::nat_smart(n)`.
#[must_use]
pub fn canonicalize_numerals(term: &Term) -> Term {
    let mut canonical = term.clone();
    rewrite_in_place(&mut canonical);
    canonical
}

fn rewrite_in_place(term: &mut Term) {
    if let Some(n) = term_to_nat(term) {
        *term = Term::nat_smart(n as u64);
        return;
    }
    // `Term::subterms_mut` is exhaustive, so a new variant cannot be a
    // numeral this pass silently skips (moved to core by ADR 29.9.26d).
    for child in term.subterms_mut() {
        rewrite_in_place(child);
    }
}
