//! The theorems that rest on an axiom or sorry (ADR 13.10.26g).
//!
//! Shared by `check`'s verdict clause, which counts them, and `doctor check
//! sorry-sites`, which names them: one population, one classification.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use albite_core::terms::analysis::{rests_on, RootClass};
use albite_core::Term;

use crate::elaborate::{CoreDef, DefTerminationMeta};

/// Each `theorem` and `lemma` resting on an axiom or sorry, with its roots
/// classified (D1, D2). Proofs are told from `fn`s the way the kernel census
/// tells them, by `termination_meta`'s `is_proof`; an axiom is a root, never
/// a resting theorem.
#[must_use]
pub fn resting_theorems<S: BuildHasher>(
    defs: &[CoreDef],
    termination_meta: &HashMap<String, DefTerminationMeta, S>,
) -> BTreeMap<String, Vec<(String, RootClass)>> {
    let classes: HashMap<&str, RootClass> = defs
        .iter()
        .filter_map(|def| {
            RootClass::of(&def.term.term, def.term.span).map(|class| (def.name.as_str(), class))
        })
        .collect();
    // The common case — no hole anywhere — costs one walk, not a closure.
    if classes.is_empty() {
        return BTreeMap::new();
    }
    let terms: Vec<(&str, &Term)> = defs
        .iter()
        .map(|def| (def.name.as_str(), &def.term.term))
        .collect();
    let is_proof = |name: &str| termination_meta.get(name).is_some_and(|meta| meta.is_proof);
    rests_on(&terms, |name| classes.contains_key(name))
        .into_iter()
        .filter(|(name, _)| is_proof(name) && classes.get(name.as_str()) != Some(&RootClass::Axiom))
        .map(|(name, roots)| {
            let classified = roots
                .into_iter()
                .map(|root| {
                    let class = classes[root.as_str()];
                    (root, class)
                })
                .collect();
            (name, classified)
        })
        .collect()
}
