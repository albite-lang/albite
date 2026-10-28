//! Which theorems rest on an axiom or a hole (ADR 13.10.26g).
//!
//! `sorry_sites` answers "does this definition carry a hole?". A theorem that
//! *cites* an axiom, or a theorem whose proof is `sorry`, carries none of its
//! own and so reads as fully proved. This walk follows global references
//! transitively and names the **roots** each definition reaches: the axioms
//! and authored or unclassified holes it ultimately relies on.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::conversion::delta::global_names;
use crate::terms::analysis::SorrySite;
use crate::terms::{Term, TermSpan};

/// What makes a definition a root (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootClass {
    /// An `axiom`: an assumption, not a gap.
    Axiom,
    /// A definition carrying an authored or unclassified `sorry`.
    Sorry,
}

impl RootClass {
    /// Classify one definition, or `None` when it is not a root.
    ///
    /// Synthesised holes are lowering artefacts in branches a match already
    /// covers (ADR 18.9.26g), so a definition carrying only those is not one.
    #[must_use]
    pub fn of(term: &Term, item_span: Option<TermSpan>) -> Option<RootClass> {
        if item_span.is_some_and(|span| is_axiom(term, span)) {
            return Some(RootClass::Axiom);
        }
        term.sorry_sites()
            .iter()
            .any(|site| !matches!(site, SorrySite::Synthesised(_)))
            .then_some(RootClass::Sorry)
    }

    /// The name a report prints for this class.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RootClass::Axiom => "axiom",
            RootClass::Sorry => "sorry",
        }
    }
}

/// Whether `term` is an axiom's body: under its type and term abstractions, a
/// `sorry` spanned by the whole item (`elaborate_axiom`, ADR 18.9.26g).
///
/// An authored `{ sorry }` is spanned by the `sorry` token, not the item, so
/// it is not an axiom. The kernel census, `term-size` and `rests_on` share
/// this one test, so they agree on what an axiom is (ADR 13.10.26c D2).
#[must_use]
pub fn is_axiom(term: &Term, item_span: TermSpan) -> bool {
    let mut term = term;
    loop {
        match term {
            // A predicate-generic axiom is introduced by annotation (ADR 6.10.26h D5)
            Term::TyAbs(_, body) | Term::Lambda(_, _, body) | Term::Annot(body, _) => term = body,
            Term::Spanned(inner, span) if **inner == Term::Sorry => return *span == item_span,
            Term::Spanned(inner, _) => term = inner,
            _ => return false,
        }
    }
}

/// For each definition in `defs`, the roots it reaches through global
/// references, transitively; a definition reaching none is absent.
///
/// The walk passes through every definition, roots included, and never counts
/// a definition as resting on itself. A name `defs` does not hold is still
/// tested with `is_root` but not expanded. Callers choose which entries to
/// report: D1 keeps proofs that are not themselves axioms.
#[must_use]
pub fn rests_on(
    defs: &[(&str, &Term)],
    is_root: impl Fn(&str) -> bool,
) -> BTreeMap<String, BTreeSet<String>> {
    let references: Vec<(&str, BTreeSet<String>)> = defs
        .iter()
        .map(|(name, term)| (*name, global_names(term)))
        .collect();
    let borrowed: Vec<(&str, &BTreeSet<String>)> = references
        .iter()
        .map(|(name, cited)| (*name, cited))
        .collect();
    rests_on_refs(&borrowed, is_root)
}

/// [`rests_on`] over each definition's already-computed global references
/// (ADR 16.10.26g D1).
///
/// The self-host's holes seam reduces each term to its reference set while
/// streaming and drops the term, so it has sets, not terms; both compilers
/// still run this one closure. The sets are borrowed, so neither caller clones
/// one. A repeated name keeps its last set, as `rests_on`'s map always did.
#[must_use]
pub fn rests_on_refs(
    refs: &[(&str, &BTreeSet<String>)],
    is_root: impl Fn(&str) -> bool,
) -> BTreeMap<String, BTreeSet<String>> {
    let references: HashMap<&str, &BTreeSet<String>> = refs.iter().copied().collect();
    let mut resting = BTreeMap::new();
    for (name, _) in refs {
        let roots = reachable_roots(name, &references, &is_root);
        if !roots.is_empty() {
            resting.insert((*name).to_string(), roots);
        }
    }
    resting
}

/// The roots reachable from `start`, excluding `start` itself.
fn reachable_roots(
    start: &str,
    references: &HashMap<&str, &BTreeSet<String>>,
    is_root: &impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    let mut visited: BTreeSet<&str> = BTreeSet::from([start]);
    let mut queue: Vec<&str> = vec![start];
    let mut roots = BTreeSet::new();
    while let Some(current) = queue.pop() {
        let Some(cited) = references.get(current) else {
            continue;
        };
        for name in *cited {
            if !visited.insert(name.as_str()) {
                continue;
            }
            if is_root(name) {
                roots.insert(name.clone());
            }
            queue.push(name.as_str());
        }
    }
    roots
}

// Tests: tests.rs — the closure and the shared axiom test (ADR 13.10.26g AC2)
#[cfg(test)]
mod tests;
