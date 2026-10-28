//! Tests: bootstrap/src/elaborate/termination/cache.rs
//!
//! The warm-cache half of the gate adapter, split from the sibling module when
//! the combined file approached the size limit. The seam mirrors the source's:
//! everything here is about `CachedTermination` — what a module contributes to
//! it, what survives the warm/cold boundary, and when a carried fact still
//! counts as something to report. The adapter's own conversion, rendering and
//! enforcement tests stay next door.
//!
//! Fixtures (`spin`, `meta`, `partial_meta`, `proof_meta`) come from the parent
//! test module rather than being duplicated.

use std::collections::HashMap;

use albite_core::terms::termination::{FailureReason, TerminationFailure, TerminationReport};
use albite_core::terms::Term;
use albite_core::types::Type;
use serde::{Deserialize, Serialize};

use super::{def, meta, partial_meta, proof_meta, spin};
use crate::cache::FULL_OUTPUT_SCHEMA_VERSION;
use crate::elaborate::termination::{
    has_nothing_to_report, CachedTermination, DefinitionSites, Enforcement, TerminationInput,
};
use crate::elaborate::{CoreDef, ElabError};

#[test]
fn cached_facts_carry_a_modules_rejections_and_its_taint() {
    let defs = [spin("spin")];
    let entries = meta(&[("spin", partial_meta())]);
    let cached = CachedTermination::from_module(&defs, &entries);

    assert!(cached.failures.is_empty(), "a partial def is not rejected");
    assert_eq!(cached.partial, vec!["spin".to_string()]);
    assert_eq!(cached.mentions.len(), 1);
    assert!(!cached.is_empty());

    let carried = cached.as_carried();
    assert!(carried.partial.contains("spin"));
    assert!(carried.mentions.contains_key("spin"));
}

#[test]
fn a_cached_rejection_is_remembered_with_its_proof_relevance() {
    let defs = [spin("recursive_proof")];
    let entries = meta(&[("recursive_proof", proof_meta())]);
    let cached = CachedTermination::from_module(&defs, &entries);

    assert_eq!(cached.failures.len(), 1);
    assert!(cached.failures[0].1, "a rejected proof is proof-relevant");
    assert_eq!(cached.proofs, vec!["recursive_proof".to_string()]);
}

#[test]
fn taint_crosses_the_warm_cold_boundary_through_the_carried_facts() {
    // `spin` is `#[partial]` and lives in a cached module; the proof is fresh.
    let cached =
        CachedTermination::from_module(&[spin("spin")], &meta(&[("spin", partial_meta())]));
    let fresh = [def(
        "thm",
        Type::Void,
        Term::App(
            Box::new(Term::Global("spin".to_string())),
            Box::new(Term::Unit),
        ),
    )];
    let input = TerminationInput::from_meta(&meta(&[("thm", proof_meta())]));

    let blind = input.check(&fresh);
    assert!(
        blind.is_clean(),
        "without the carried facts the taint is invisible — this is the hole"
    );

    let carried = input.check_with_carried(&fresh, &cached.as_carried());
    assert_eq!(carried.failures.len(), 1);
    assert_eq!(carried.failures[0].function, "thm");
}

#[test]
fn absorbing_module_facts_accumulates_every_field() {
    let mut project = CachedTermination::default();
    assert!(project.is_empty());

    project.absorb(CachedTermination::from_module(
        &[spin("spin")],
        &meta(&[("spin", partial_meta())]),
    ));
    project.absorb(CachedTermination::from_module(
        &[spin("recursive_proof")],
        &meta(&[("recursive_proof", proof_meta())]),
    ));

    assert_eq!(project.partial, vec!["spin".to_string()]);
    assert_eq!(project.proofs, vec!["recursive_proof".to_string()]);
    assert_eq!(project.failures.len(), 1);
    assert_eq!(project.mentions.len(), 2);
}

#[test]
fn a_clean_report_with_no_carried_rejections_has_nothing_to_report() {
    let report = TerminationInput::from_meta(&HashMap::new()).check(&[]);

    assert!(has_nothing_to_report(
        &report,
        &CachedTermination::default()
    ));
}

#[test]
fn a_carried_rejection_is_still_something_to_report_on_a_clean_run() {
    // The fresh side is clean because the rejected module came from cache and
    // contributed no terms — reading only `report.is_clean()` drops it.
    let carried = CachedTermination::from_module(
        &[spin("recursive_proof")],
        &meta(&[("recursive_proof", proof_meta())]),
    );
    let report = TerminationInput::from_meta(&HashMap::new()).check(&[]);

    assert!(report.is_clean());
    assert!(!has_nothing_to_report(&report, &carried));
}

#[test]
fn a_fresh_rejection_is_something_to_report_even_with_nothing_carried() {
    let report = TerminationInput::from_meta(&HashMap::new()).check(&[spin("spin")]);

    assert!(!has_nothing_to_report(
        &report,
        &CachedTermination::default()
    ));
}

#[test]
fn cached_facts_are_empty_only_when_every_field_is() {
    assert!(CachedTermination::default().is_empty());

    let mut only_partial = CachedTermination::default();
    only_partial.partial.push("spin".to_string());
    assert!(!only_partial.is_empty());

    let mut only_proofs = CachedTermination::default();
    only_proofs.proofs.push("thm".to_string());
    assert!(!only_proofs.is_empty());

    let mut only_mentions = CachedTermination::default();
    only_mentions.mentions.push(("f".to_string(), Vec::new()));
    assert!(!only_mentions.is_empty());

    let mut only_foreign = CachedTermination::default();
    only_foreign
        .foreign
        .push(("g".to_string(), "g".to_string()));
    assert!(!only_foreign.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// Foreign calls and statement edges (ADR 3.10.26c D5)
// ─────────────────────────────────────────────────────────────────────────────

/// `extern "C" fn g() -> Nat`, as the elaborator emits it.
pub(super) fn foreign_wrapper() -> CoreDef {
    def(
        "g",
        Type::Nat,
        Term::ExternCall("__c_g".to_string(), Vec::new()),
    )
}

/// `axiom ax(): Eq<Nat, target, target>` — it reaches `target` through its
/// statement alone, which is the shape a `refl` body leaves too.
pub(super) fn statement_only_axiom(target: &str) -> CoreDef {
    let global = || Box::new(Term::Global(target.to_string()));
    def(
        "ax",
        Type::Eq(Box::new(Type::Nat), global(), global()),
        Term::Unit,
    )
}

/// The reasons `ax` is rejected for on a cold run over both modules, and on a
/// warm one where each module was cached on its own and nothing is fresh.
fn cold_and_warm(
    wrapper: CoreDef,
    wrapper_meta: &[(&str, crate::elaborate::DefTerminationMeta)],
) -> (Vec<FailureReason>, Vec<FailureReason>) {
    let reasons = |report: TerminationReport| -> Vec<FailureReason> {
        report
            .failures
            .into_iter()
            .map(|failure| failure.reason)
            .collect()
    };
    let axiom = statement_only_axiom(&wrapper.name);
    let proof_entries = meta(&[("ax", proof_meta())]);
    let mut all_entries = meta(wrapper_meta);
    all_entries.extend(proof_entries.clone());

    let cold = TerminationInput::from_meta(&all_entries).check(&[wrapper.clone(), axiom.clone()]);

    // Cached apart, the axiom's module never sees the wrapper: the statement
    // edge it carries is the only route left.
    let mut carried = CachedTermination::from_module(&[wrapper], &meta(wrapper_meta));
    carried.absorb(CachedTermination::from_module(&[axiom], &proof_entries));
    let warm =
        TerminationInput::from_meta(&HashMap::new()).check_with_carried(&[], &carried.as_carried());
    (reasons(cold), reasons(warm))
}

#[test]
fn a_statement_edge_is_carried_beside_the_body_edges() {
    let cached = CachedTermination::from_module(
        &[statement_only_axiom("g")],
        &meta(&[("ax", proof_meta())]),
    );

    assert_eq!(
        cached.mentions,
        vec![("ax".to_string(), vec!["g".to_string()])]
    );
}

#[test]
fn a_foreign_seed_is_carried_with_its_symbol() {
    let cached = CachedTermination::from_module(&[foreign_wrapper()], &HashMap::new());

    assert_eq!(cached.foreign, vec![("g".to_string(), "g".to_string())]);
    assert!(cached.as_carried().foreign.contains_key("g"));
}

// 3.10.26c AC6: a proof whose foreign dependency is in its statement only,
// cached apart from the wrapper, is rejected warm exactly as cold — and the
// same holds for a `#[partial]` dependency.
#[test]
fn a_statement_only_foreign_dependency_is_rejected_warm_as_cold() {
    let (cold, warm) = cold_and_warm(foreign_wrapper(), &[]);

    assert_eq!(
        cold,
        vec![FailureReason::ForeignInProof {
            extern_symbol: "g".to_string(),
            via: vec!["g".to_string()],
        }]
    );
    assert_eq!(warm, cold);
}

#[test]
fn a_statement_only_partial_dependency_is_rejected_warm_as_cold() {
    let (cold, warm) = cold_and_warm(spin("spin"), &[("spin", partial_meta())]);

    assert_eq!(
        cold,
        vec![FailureReason::PartialInProof {
            tainted: "spin".to_string(),
            via: Vec::new(),
        }]
    );
    assert_eq!(warm, cold);
}

/// `CachedTermination` as schema 5 wrote it: no `foreign`.
#[derive(Serialize, Deserialize)]
struct SchemaFiveTermination {
    failures: Vec<(ElabError, bool)>,
    mentions: Vec<(String, Vec<String>)>,
    partial: Vec<String>,
    proofs: Vec<String>,
}

// 3.10.26c AC6: an entry written before the foreign seed existed is not read.
// The keys moved (full-output schema 6, signature prefix v3), and a blob that
// reached the decoder anyway is refused rather than read with no seeds.
// Later bumps (7: 3.10.26m D8) keep the keys moved, so the bound is a floor.
#[test]
fn a_schema_five_entry_is_not_read() {
    assert!(FULL_OUTPUT_SCHEMA_VERSION >= 6);
    let old = SchemaFiveTermination {
        failures: Vec::new(),
        mentions: vec![("g".to_string(), Vec::new())],
        partial: Vec::new(),
        proofs: Vec::new(),
    };
    let bytes = bincode::serialize(&old).expect("serializes");

    assert!(bincode::deserialize::<CachedTermination>(&bytes).is_err());
}

// 3.10.26c AC9: a foreign rejection is proof-relevant by its reason alone —
// the gate splits on it and the cache's flag records it — so it fails the
// build under `--termination proofs`.
#[test]
fn a_foreign_rejection_gates_at_the_proofs_only_level() {
    let failure = TerminationFailure {
        function: "not_registered_as_a_proof".to_string(),
        group: Vec::new(),
        span: None,
        reason: FailureReason::ForeignInProof {
            extern_symbol: "g".to_string(),
            via: vec!["g".to_string()],
        },
    };
    let input = TerminationInput::from_meta(&HashMap::new());
    assert!(input.is_proof_relevant(&failure));

    let report = TerminationReport {
        failures: vec![failure],
        ..TerminationReport::default()
    };
    let split = input.partition_errors(
        &report,
        &DefinitionSites::default(),
        Enforcement::ProofsOnly,
    );
    assert_eq!(split.gating.len(), 1);
    assert!(split.reported.is_empty());
    assert_eq!(split.gating[0].kind.code(), "E0063");
}

#[test]
fn a_cached_foreign_rejection_records_that_it_is_proof_relevant() {
    let cached = CachedTermination::from_module(
        &[foreign_wrapper(), statement_only_axiom("g")],
        &meta(&[("ax", proof_meta())]),
    );

    assert_eq!(cached.failures.len(), 1);
    assert!(cached.failures[0].1);
    assert_eq!(cached.failures[0].0.kind.code(), "E0063");
}
