//! What an elaboration-cache entry has to remember about admission
//! (ADR 29.6.26e §2.5).
//!
//! Split from the adapter because it answers a different question. `mod.rs`
//! asks "what does the engine conclude about these terms"; this asks "what
//! survives when there are no terms" — a module served from the signature
//! cache contributes no `CoreDef`s, so its verdict and its place in the taint
//! graph have to be carried rather than recomputed.

use std::collections::HashMap;
use std::path::Path;

use albite_core::terms::termination::{globals_in_type, CarriedDefs, OccurrenceGraph};
use serde::{Deserialize, Serialize};

use super::location::{DefinitionSites, ProofSite};
use super::TerminationInput;
use crate::elaborate::{CoreDef, DefTerminationMeta, ElabError};

/// Everything a cache entry has to remember about a module's admission
/// (ADR 29.6.26e §2.5).
///
/// A module served from the elaboration cache contributes no `CoreDef`s, so
/// without this its rejections would silently disappear on the second run and
/// its `#[partial]` definitions would drop out of the taint graph. Terms are
/// *not* stored: descent was decided when the module was fresh and a value SCC
/// never spans modules, so only the names and edges taint needs are kept.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedTermination {
    /// This module's rejections, rendered, each with whether it is
    /// proof-relevant — enough to re-decide gating at a different enforcement
    /// level than the one that wrote the entry.
    pub failures: Vec<(ElabError, bool)>,
    /// Definition → the definitions it mentions, in its body **or its
    /// statement** (ADR 3.10.26c D5): an `Eq` type embeds terms, and a proof
    /// whose body is `refl` may reach a constant through its statement alone.
    pub mentions: Vec<(String, Vec<String>)>,
    /// Which of them are annotated `#[partial]`.
    pub partial: Vec<String>,
    /// Which of them are proof-relevant.
    pub proofs: Vec<String>,
    /// Which of them call foreign code, with the symbol (ADR 3.10.26c D5).
    pub foreign: Vec<(String, String)>,
    /// Where each proof-relevant definition is (ADR 3.10.26r D2), so a warm
    /// rejection points where a cold one does. Last, so the layout change is
    /// an append; the key's prefix still moved.
    pub sites: Vec<ProofSite>,
}

impl CachedTermination {
    /// Compute a module's cacheable termination facts from its own output.
    #[must_use]
    pub fn from_module(defs: &[CoreDef], meta: &HashMap<String, DefTerminationMeta>) -> Self {
        let input = TerminationInput::from_meta(meta);
        let report = input.check(defs);
        let graph =
            OccurrenceGraph::build(defs.iter().map(|def| (def.name.as_str(), &def.term.term)));
        CachedTermination {
            failures: input.rejections(&report, &DefinitionSites::of_defs(defs)),
            mentions: defs
                .iter()
                .map(|def| {
                    let mut reached = graph.mentions(&def.name);
                    reached.extend(globals_in_type(&def.ty));
                    (def.name.clone(), reached.into_iter().collect())
                })
                .collect(),
            partial: meta
                .iter()
                .filter(|(_, entry)| entry.attrs.partial)
                .map(|(name, _)| name.clone())
                .collect(),
            proofs: meta
                .iter()
                .filter(|(_, entry)| entry.is_proof)
                .map(|(name, _)| name.clone())
                .collect(),
            foreign: report.foreign_seeds.into_iter().collect(),
            sites: defs
                .iter()
                .filter(|def| input.is_proof(&def.name))
                .map(|def| ProofSite {
                    name: def.name.clone(),
                    span: def.span,
                    file: None,
                })
                .collect(),
        }
    }

    /// Stamp the file a walk served this entry from (ADR 3.10.26r D2) onto
    /// every carried site and every carried rejection that has none — a
    /// rejection rendered when its module was fresh had no file either (D1
    /// sets it at the accumulator, which a module's own check never reaches).
    pub fn locate_in(&mut self, file: &Path) {
        for site in &mut self.sites {
            site.file.get_or_insert_with(|| file.to_path_buf());
        }
        for (error, _) in &mut self.failures {
            error.file_path.get_or_insert_with(|| file.to_path_buf());
        }
    }

    /// Fold another module's facts into this accumulation.
    pub fn absorb(&mut self, other: CachedTermination) {
        self.failures.extend(other.failures);
        self.mentions.extend(other.mentions);
        self.partial.extend(other.partial);
        self.proofs.extend(other.proofs);
        self.foreign.extend(other.foreign);
        self.sites.extend(other.sites);
    }

    /// Whether nothing was carried.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
            && self.mentions.is_empty()
            && self.partial.is_empty()
            && self.proofs.is_empty()
            && self.foreign.is_empty()
            && self.sites.is_empty()
    }

    /// The engine's view of these definitions.
    #[must_use]
    pub fn as_carried(&self) -> CarriedDefs {
        CarriedDefs {
            mentions: self
                .mentions
                .iter()
                .map(|(name, reached)| (name.clone(), reached.iter().cloned().collect()))
                .collect(),
            partial: self.partial.iter().cloned().collect(),
            foreign: self.foreign.iter().cloned().collect(),
            proofs: self.proofs.iter().cloned().collect(),
        }
    }
}
