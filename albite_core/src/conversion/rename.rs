//! Renaming apart (ADR 29.9.26c D2).
//!
//! `Term::substitute` checks shadowing but never renames a binder, so a free
//! variable carried into a callee is captured by a binder of the same name.
//! Conversion therefore renames every free variable to a spelling no binder
//! can have — [`RESERVED`] is a character the lexer rejects in identifiers and
//! no elaborator-generated binder (`@`, `α_`, `__…`) starts with — before
//! either evaluator runs. Binder-opening variables are `?<n>`: a digit never
//! starts an identifier, so they cannot collide with a renamed `?<name>`.
//!
//! Reserved spellings elsewhere, none of which starts with [`RESERVED`]:
//! `fresh_name`'s `′` suffix (capture-avoiding renames), and `#x`, the binder
//! of every motive the elaborator synthesises for `sym`/`trans`/`cong`
//! (ADR 1.10.26e P0.4) — `#` cannot start an identifier either.

use std::collections::BTreeSet;

use crate::Term;

/// The prefix every renamed or freshly opened variable carries.
pub(crate) const RESERVED: char = '?';

/// Rename every free variable of either side to `?<name>`, consistently
/// across both, so a substitution can never carry a source-named free
/// variable under a source-named binder.
#[must_use]
pub fn rename_apart(left: &Term, right: &Term) -> (Term, Term) {
    let free: BTreeSet<String> = left
        .free_vars()
        .into_iter()
        .chain(right.free_vars())
        .filter(|name| !name.starts_with(RESERVED))
        .collect();
    let rename = |side: &Term| {
        free.iter().fold(side.clone(), |term, name| {
            term.substitute(name, &Term::Var(format!("{RESERVED}{name}")))
        })
    };
    (rename(left), rename(right))
}

/// The binder-opening variables of one comparison, and the way back to
/// source names for a note.
#[derive(Debug, Default)]
pub struct Renaming {
    next_fresh: usize,
}

impl Renaming {
    /// A variable no binder and no renamed free variable can spell.
    pub(crate) fn fresh(&mut self) -> String {
        let name = format!("{RESERVED}{}", self.next_fresh);
        self.next_fresh += 1;
        name
    }

    /// Render `term` with every reserved name mapped back: `?n` shows as `n`,
    /// an opened binder `?0` as `_0`. Display only — the result is never
    /// reduced, so capture here is harmless.
    #[must_use]
    pub fn display(term: &Term) -> String {
        let reserved: BTreeSet<String> = term
            .free_vars()
            .into_iter()
            .filter(|name| name.starts_with(RESERVED))
            .collect();
        reserved
            .iter()
            .fold(term.clone(), |shown, name| {
                let source = &name[RESERVED.len_utf8()..];
                let source = if source.starts_with(|c: char| c.is_ascii_digit()) {
                    format!("_{source}")
                } else {
                    source.to_string()
                };
                shown.substitute(name, &Term::Var(source))
            })
            .to_string()
    }
}
