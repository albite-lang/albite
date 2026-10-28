//! What the registry hands the engine as taint, beside what it retained.
//!
//! The carried set is every definition the Retain stream did not keep, and
//! both it and the retained definitions take their `#[partial]` mark from
//! `notes` — except that a repeated name is always marked (ADR 13.10.26m D5).

use crate::terms::termination::{CarriedDefs, TerminationAnnotation};

use super::TerminationRegistry;

impl TerminationRegistry {
    /// Every definition that was *not* retained, as the engine's carried set.
    ///
    /// The self-hosted compiler has no elaboration cache, so this channel
    /// carries no cache entry — it carries the definitions this seam reduced
    /// instead, which is the same thing from the engine's side: a name, its
    /// edges, and whether it is annotated or proof-relevant.
    pub(super) fn carried(&self) -> CarriedDefs {
        let mut carried = CarriedDefs::default();
        for (name, reached) in &self.mentions {
            if self.retained.contains_key(name) {
                continue;
            }
            carried.mentions.insert(name.clone(), reached.clone());
            if let Some(symbol) = self.foreign.get(name) {
                carried.foreign.insert(name.clone(), symbol.clone());
            }
            let note = self.notes.get(name);
            if note.is_some_and(|note| note.annotation.partial) || self.is_repeated(name) {
                carried.partial.insert(name.clone());
            }
            if note.is_some_and(|note| note.proof) {
                carried.proofs.insert(name.clone());
            }
        }
        carried
    }

    /// `annotation`, marked `#[partial]` when `name` is repeated (ADR
    /// 13.10.26m D5, P0.5).
    ///
    /// `notes` is last-writer-wins too, so a total `y::f` noted after a
    /// `#[partial]` `x::f` would clear the taint a theorem citing `x::f` must
    /// carry. Which definition a bare name meant cannot be known here, so a
    /// repeated name carries taint: a theorem is refused, never wrongly
    /// admitted.
    pub(super) fn taint_repeated(
        &self,
        name: &str,
        annotation: TerminationAnnotation,
    ) -> TerminationAnnotation {
        TerminationAnnotation {
            partial: annotation.partial || self.is_repeated(name),
            ..annotation
        }
    }
}
