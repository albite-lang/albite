//! The termination mirror's protocol, as a state machine (ADR 19.8.26d §2.2).
//!
//! Separate from the externs so the protocol is testable without a C boundary,
//! and separate from [`super::wire`] so the *shape* of a definition stream and
//! the *rendering* of a failure cannot be confused for one another.
//!
//! ## Why three streams and not one
//!
//! P0 measured the one-stream design — every definition's `Term` and `Type`
//! materialized and held live — at **+587 MiB**, against D1's +250 MiB
//! ceiling, of which 91.6% was terms held for an analysis that needs a fifth
//! of them. So the stream reduces instead:
//!
//! ```text
//! reset ( note_item )* ( declare )* ( add_def )* plan ( add_def )* check
//! ```
//!
//! - **declare** fixes the node set. It has to be complete before any mention
//!   set is computed, because `OccurrenceGraph::build_knowing` filters
//!   occurrences against the names it recognises: a definition reduced before
//!   its callees were declared would record edges to none of them.
//! - **`add_def`** in [`Phase::Reduce`] materializes, reduces to the name sets
//!   taint and the graph need, and **drops** the payload.
//! - **plan** runs Tarjan over those name sets and marks the members of every
//!   recursive component. Descent is the one consumer that needs a real term,
//!   and only for those.
//! - **`add_def`** in [`Phase::Retain`] materializes the same stream again and
//!   keeps only the planned definitions. Re-streaming is cheaper than a
//!   name-keyed re-lookup over the caller's list, and the peak was already
//!   paid: transient materialization is a high-water mark, not a sum.
//!
//! A recursive group with a `#[partial]` member is planned like any other even
//! though descent skips it, because it still **seeds taint** for its
//! non-annotated members — the arm that makes "every recursive group" the
//! right retention set rather than "every checked group".
//!
//! The δ discharge adds one more retention set, the obligations' mention
//! closure (ADR 7.10.26u, [`delta`]).

pub(crate) mod delta;
mod taint;

use std::collections::{BTreeMap, BTreeSet};

use crate::terms::termination::{
    analyze_with_carried, foreign_call_in, globals_in_type, is_recursive, tarjan_scc, DefRole,
    DefView, OccurrenceGraph, TerminationAnnotation, TerminationFailure,
};
use crate::terms::Term;
use crate::types::Type;

use super::nominal_roots::{
    nominalize_parameters, nominally_encoded_names, DeclaredKind, DeclaredType,
};
use super::wire::ProtocolError;

/// Which stream the registry is reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Names are still being declared and payloads reduced.
    Reduce,
    /// The plan is fixed; payloads are retained if they are in it.
    Retain,
    /// The analysis has run.
    Checked,
}

/// What an item's attributes and kind said about it.
///
/// Only the exceptions are noted, exactly as the bootstrap's
/// `record_termination_meta` does: an absent entry is an unannotated
/// executable definition, which is the overwhelmingly common case.
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemNote {
    pub annotation: TerminationAnnotation,
    pub proof: bool,
}

/// One retained definition, owned for as long as the analysis needs it.
#[derive(Debug)]
struct RetainedDef {
    ty: Type,
    term: Term,
    annotation: TerminationAnnotation,
    role: DefRole,
}

/// The ambient environment the externs drive.
#[derive(Debug, Default)]
pub(crate) struct TerminationRegistry {
    phase: PhaseState,
    /// Every definition name, fixed before any reduction (see module docs).
    declared: BTreeSet<String>,
    /// Per-item attributes and proof-relevance, keyed by name.
    notes: BTreeMap<String, ItemNote>,
    /// name → what it mentions, for every definition. The reduced payload.
    mentions: BTreeMap<String, BTreeSet<String>>,
    /// name → the foreign symbol its payload calls, for every definition whose
    /// payload calls one (ADR 3.10.26q D4). Reduced beside `mentions`.
    foreign: BTreeMap<String, String>,
    /// The recursive-component members, fixed by [`TerminationRegistry::plan`].
    planned: BTreeSet<String>,
    /// The planned definitions' payloads, live only from `plan` to `check`.
    retained: BTreeMap<String, RetainedDef>,
    /// The last analysis's rejections.
    failures: Vec<TerminationFailure>,
    /// Every type declaration the collection passes noted, by name (ADR
    /// 3.9.26f). Last-write-wins, like `notes`.
    declared_types: BTreeMap<String, DeclaredType>,
    /// Type names the bootstrap encodes nominally — recursive-cluster members
    /// and flat `Adt`s — fixed from `declared_types` by `plan`.
    nominal_types: BTreeSet<String>,
    /// name → parameter position → the type name its annotation spelled.
    declared_parameter_types: BTreeMap<String, BTreeMap<usize, String>>,
    /// What the δ discharge retains, and the admission it reads.
    delta: delta::DeltaRetention,
}

/// `Default` for [`Phase`] as a named wrapper, so the registry can derive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhaseState(Phase);

impl Default for PhaseState {
    fn default() -> Self {
        PhaseState(Phase::Reduce)
    }
}

impl TerminationRegistry {
    /// Discard the previous pass entirely — definitions, plan and verdict.
    pub(crate) fn reset(&mut self) {
        *self = TerminationRegistry::default();
    }

    /// The phase the registry is in.
    pub(crate) fn phase(&self) -> Phase {
        self.phase.0
    }

    /// Record one item's attributes and role.
    ///
    /// Last-write-wins, like the positivity registry's `defs` map: body
    /// elaboration can re-enter an item, and overwriting with the newest
    /// reading keeps the environment the best-available one rather than the
    /// first-seen one (R1).
    pub(crate) fn note_item(&mut self, name: &str, note: ItemNote) -> Result<(), ProtocolError> {
        self.expect(Phase::Reduce, "note_item")?;
        self.notes.insert(name.to_string(), note);
        Ok(())
    }

    /// Record one type declaration's kind and constructor count.
    ///
    /// Clears the references a previous note recorded: every collection pass
    /// re-notes the types it collects, and the newest reading replaces the
    /// older one wholesale rather than unioning two readings' edges.
    ///
    /// Allowed in **any** phase. A type note is not part of the definition
    /// stream's grammar — collection passes run wherever the driver runs them,
    /// and [`TerminationRegistry::reset`] is what discards a previous program.
    pub(crate) fn note_type(&mut self, name: &str, kind: DeclaredKind, constructors: usize) {
        self.declared_types.insert(
            name.to_string(),
            DeclaredType {
                kind,
                constructors,
                references: BTreeSet::new(),
            },
        );
    }

    /// Record that `name`'s declared body spells `referenced`.
    pub(crate) fn note_type_reference(
        &mut self,
        name: &str,
        referenced: &str,
    ) -> Result<(), ProtocolError> {
        let declared = self
            .declared_types
            .get_mut(name)
            .ok_or_else(|| ProtocolError::UndeclaredType(name.to_string()))?;
        declared.references.insert(referenced.to_string());
        Ok(())
    }

    /// The names `plan` fixed, for the seam's own tests.
    #[cfg(test)]
    pub(crate) fn nominal_types_for_test(&self) -> BTreeSet<String> {
        self.nominal_types.clone()
    }

    /// Record the type name one parameter's annotation spelled.
    ///
    /// Last-write-wins per position, like [`TerminationRegistry::note_item`].
    pub(crate) fn note_parameter_type(
        &mut self,
        name: &str,
        position: usize,
        type_name: &str,
    ) -> Result<(), ProtocolError> {
        self.expect(Phase::Reduce, "note_parameter_type")?;
        self.declared_parameter_types
            .entry(name.to_string())
            .or_default()
            .insert(position, type_name.to_string());
        Ok(())
    }

    /// Declare one definition name.
    pub(crate) fn declare(&mut self, name: &str) -> Result<(), ProtocolError> {
        self.expect(Phase::Reduce, "declare")?;
        self.declared.insert(name.to_string());
        Ok(())
    }

    /// Offer one definition's payload, in whichever phase the registry is in.
    ///
    /// A name that was never declared is a protocol error rather than a new
    /// node: the node set is what every mention set was filtered against, and
    /// admitting a late one would give it edges the earlier definitions could
    /// not have recorded to it.
    pub(crate) fn add_def(
        &mut self,
        name: &str,
        ty: Type,
        mut term: Term,
    ) -> Result<(), ProtocolError> {
        if !self.declared.contains(name) {
            return Err(ProtocolError::UndeclaredDefinition(name.to_string()));
        }
        match self.phase() {
            Phase::Reduce => {
                let reached = self.reduce(name, &ty, &term);
                // A replaced entry is a second definition of one bare name:
                // two modules' values the Core terms cannot tell apart (ADR
                // 13.10.26m D5), so δ must never unfold it.
                if self.mentions.insert(name.to_string(), reached).is_some() {
                    self.note_repeated_definition(name);
                }
                self.record_foreign_call(name, &term);
                Ok(())
            }
            Phase::Retain => {
                self.retain_for_delta(name, &term);
                if self.planned.contains(name) {
                    // Only descent reads binder types, and only retained
                    // definitions reach descent — so the reduce stream never
                    // pays for the rewrite.
                    if let Some(declared) = self.declared_parameter_types.get(name) {
                        nominalize_parameters(&mut term, declared, &self.nominal_types);
                    }
                    let note = self.notes.get(name).cloned().unwrap_or_default();
                    let annotation = self.taint_repeated(name, note.annotation);
                    self.retained.insert(
                        name.to_string(),
                        RetainedDef {
                            ty,
                            term,
                            annotation,
                            role: if note.proof {
                                DefRole::Proof
                            } else {
                                DefRole::Executable
                            },
                        },
                    );
                }
                Ok(())
            }
            Phase::Checked => Err(ProtocolError::AfterCheck("add_def")),
        }
    }

    /// Reduce one payload to the name sets the graph and taint need.
    ///
    /// The occurrence half goes through the engine's own
    /// `OccurrenceGraph::build_knowing` rather than a hand-rolled walk, so this
    /// side cannot drift from the side that decides the verdict. The type half
    /// is what `MentionIndex::build` adds for an `Eq` witness's embedded terms.
    fn reduce(&self, name: &str, ty: &Type, term: &Term) -> BTreeSet<String> {
        let graph = OccurrenceGraph::build_knowing(std::iter::once((name, term)), &self.declared);
        let mut reached = graph.mentions(name);
        reached.extend(globals_in_type(ty));
        reached
    }

    /// Record the foreign symbol `term` calls, through the engine's own
    /// predicate (ADR 3.10.26q D3/D4).
    ///
    /// Replaced, never unioned, like the mention set: a newer payload with no
    /// foreign call clears an older one's seed.
    fn record_foreign_call(&mut self, name: &str, term: &Term) {
        match foreign_call_in(term) {
            Some(symbol) => self.foreign.insert(name.to_string(), symbol),
            None => self.foreign.remove(name),
        };
    }

    /// Close the reduction stream and decide what must be retained.
    ///
    /// Returns how many definitions the plan holds — the descent plan and the
    /// δ discharge's closure — which is how many the caller's second stream
    /// will actually keep.
    ///
    /// **`with_descent` selects which half of the analysis the caller is
    /// mirroring.** Taint needs only the name sets pass 1 already reduced;
    /// descent needs real terms, and it is the caller — not this side — that
    /// knows whether its terms carry what descent reads. The self-hosted
    /// elaborator's do not yet (ADR 19.8.26d §7), so it passes `false` and no
    /// definition is retained for descent.
    pub(crate) fn plan(&mut self, with_descent: bool) -> Result<usize, ProtocolError> {
        self.expect(Phase::Reduce, "plan")?;
        if with_descent {
            self.nominal_types = nominally_encoded_names(&self.declared_types);
            let adjacency: BTreeMap<String, BTreeSet<String>> = self.mentions.clone();
            for component in tarjan_scc(&adjacency) {
                if is_recursive(&component, &adjacency) {
                    self.planned.extend(component);
                }
            }
        }
        let delta_only = self.plan_delta();
        self.phase = PhaseState(Phase::Retain);
        Ok(self.planned.len() + delta_only)
    }

    /// Run the analysis and record its rejections. Returns the count.
    pub(crate) fn check(&mut self) -> Result<usize, ProtocolError> {
        self.expect(Phase::Retain, "check")?;
        let views: BTreeMap<String, DefView<'_>> = self
            .retained
            .iter()
            .map(|(name, def)| {
                (
                    name.clone(),
                    DefView {
                        ty: &def.ty,
                        term: &def.term,
                        annotation: &def.annotation,
                        role: def.role,
                    },
                )
            })
            .collect();
        let report = analyze_with_carried(&views, &self.carried());
        self.failures = report.failures;
        self.record_admission(report.admission);
        self.phase = PhaseState(Phase::Checked);
        Ok(self.failures.len())
    }

    /// The rejections the last [`TerminationRegistry::check`] found.
    pub(crate) fn failures(&self) -> &[TerminationFailure] {
        &self.failures
    }

    /// Refuse a call made in the wrong phase.
    fn expect(&self, want: Phase, call: &'static str) -> Result<(), ProtocolError> {
        if self.phase() == want {
            Ok(())
        } else {
            Err(ProtocolError::WrongPhase {
                call,
                found: self.phase(),
            })
        }
    }
}
