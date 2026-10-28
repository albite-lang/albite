//! Tests: bootstrap/src/elaborate/termination/location.rs, and the location
//! halves of `cache.rs` and `merge_carried` (ADR 3.10.26r).
//!
//! Every rejection here is asked the same two questions — which file, which
//! span — cold and warm, because a gate that names the right proof at the
//! wrong place is how the entry file came to be blamed for `b.tg`'s proofs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::cache_tests::{foreign_wrapper, statement_only_axiom};
use super::{meta, partial_meta, proof_meta, spin};
use crate::elaborate::termination::{
    merge_carried, CachedTermination, DefinitionSites, ProofSite, TerminationInput,
};
use crate::elaborate::{CoreDef, DefTerminationMeta, ElabError, ElabErrorKind};
use crate::span::Span;

const AXIOM_SPAN: Span = Span::new(30, 70);

/// The axiom, at a span distinct from the fixtures' shared one, so a test can
/// tell "the proof's span" from "some definition's span".
fn axiom_over(target: &str) -> CoreDef {
    CoreDef {
        span: AXIOM_SPAN,
        ..statement_only_axiom(target)
    }
}

/// The one rejection's file and span.
fn location_of(rejections: &[(ElabError, bool)]) -> (Option<PathBuf>, Span) {
    assert_eq!(rejections.len(), 1, "expected exactly one rejection");
    (rejections[0].0.file_path.clone(), rejections[0].0.span)
}

/// §1's separate-module case: the seed in `a.tg`, the axiom in `b.tg`, cold
/// over both, then warm with each module cached on its own.
fn separate_modules_cold_and_warm(
    seed: CoreDef,
    seed_meta: &[(&str, DefTerminationMeta)],
) -> (Vec<(ElabError, bool)>, Vec<(ElabError, bool)>) {
    let (a, b) = (Path::new("a.tg"), Path::new("b.tg"));
    let axiom = axiom_over(&seed.name);
    let mut all_meta = meta(seed_meta);
    all_meta.insert("ax".to_string(), proof_meta());

    let seed_defs = [seed];
    let axiom_defs = [axiom];
    let mut cold_sites = DefinitionSites::default();
    cold_sites.add_module(a, &seed_defs);
    cold_sites.add_module(b, &axiom_defs);
    let both = [seed_defs[0].clone(), axiom_defs[0].clone()];
    let input = TerminationInput::from_meta(&all_meta);
    let cold = input.rejections(&input.check(&both), &cold_sites);

    let mut carried = CachedTermination::from_module(&seed_defs, &meta(seed_meta));
    carried.locate_in(a);
    let mut axiom_entry =
        CachedTermination::from_module(&axiom_defs, &meta(&[("ax", proof_meta())]));
    axiom_entry.locate_in(b);
    carried.absorb(axiom_entry);
    let mut warm_sites = DefinitionSites::default();
    warm_sites.add_carried(&carried);
    let warm_input = TerminationInput::from_meta(&HashMap::new());
    let report = warm_input.check_with_carried(&[], &carried.as_carried());
    let recomputed = warm_input.rejections(&report, &warm_sites);
    let warm = merge_carried(recomputed, carried.failures);
    (cold, warm)
}

// 3.10.26r D1: the file comes from the module group, not the entry.
#[test]
fn a_rejection_names_the_file_its_definition_came_from() {
    let defs = [spin("spin")];
    let mut sites = DefinitionSites::of_defs(&defs);
    sites.add_module(Path::new("b.tg"), &defs);
    let input = TerminationInput::from_meta(&HashMap::new());

    let rejections = input.rejections(&input.check(&defs), &sites);

    assert_eq!(
        location_of(&rejections),
        (Some(PathBuf::from("b.tg")), Span::new(10, 20))
    );
}

#[test]
fn a_definition_outside_every_module_group_keeps_its_span_and_no_file() {
    let defs = [spin("spin")];
    let input = TerminationInput::from_meta(&HashMap::new());

    let rejections = input.rejections(&input.check(&defs), &DefinitionSites::of_defs(&defs));

    assert_eq!(location_of(&rejections), (None, Span::new(10, 20)));
}

#[test]
fn a_fresh_definitions_location_outranks_a_carried_site_of_the_same_name() {
    let defs = [spin("thm")];
    let mut carried = CachedTermination::default();
    carried.sites.push(ProofSite {
        name: "thm".to_string(),
        span: Span::new(1, 2),
        file: Some(PathBuf::from("stale.tg")),
    });
    let mut sites = DefinitionSites::of_defs(&defs);
    sites.add_module(Path::new("fresh.tg"), &defs);
    sites.add_carried(&carried);

    assert_eq!(
        sites.get("thm"),
        Some((Span::new(10, 20), Some(Path::new("fresh.tg"))))
    );
}

#[test]
fn a_cache_entry_records_a_site_for_each_proof_and_only_the_proofs() {
    let defs = [spin("spin"), axiom_over("spin")];
    let cached = CachedTermination::from_module(
        &defs,
        &meta(&[("spin", partial_meta()), ("ax", proof_meta())]),
    );

    assert_eq!(
        cached.sites,
        vec![ProofSite {
            name: "ax".to_string(),
            span: AXIOM_SPAN,
            file: None,
        }]
    );
}

#[test]
fn locating_an_entry_stamps_its_sites_and_rejections_but_keeps_a_file_already_set() {
    let mut cached = CachedTermination::from_module(
        &[spin("recursive_proof")],
        &meta(&[("recursive_proof", proof_meta())]),
    );
    let mut elsewhere = ElabError::new(
        Span::new(0, 1),
        ElabErrorKind::UndefinedVariable("x".into()),
    );
    elsewhere.file_path = Some(PathBuf::from("kept.tg"));
    cached.failures.push((elsewhere, false));

    cached.locate_in(Path::new("b.tg"));

    assert_eq!(cached.sites[0].file, Some(PathBuf::from("b.tg")));
    assert_eq!(cached.failures[0].0.file_path, Some(PathBuf::from("b.tg")));
    assert_eq!(
        cached.failures[1].0.file_path,
        Some(PathBuf::from("kept.tg"))
    );
}

#[test]
fn proof_sites_alone_make_a_carried_entry_non_empty_and_are_absorbed() {
    let mut only_sites = CachedTermination::default();
    only_sites.sites.push(ProofSite {
        name: "ax".to_string(),
        span: AXIOM_SPAN,
        file: None,
    });
    assert!(!only_sites.is_empty());

    let mut project = CachedTermination::default();
    project.absorb(only_sites);
    assert_eq!(project.sites.len(), 1);
}

// 3.10.26r AC2: a proof cached apart from its seed is rejected warm with the
// same span and file as cold — for a `#[partial]` seed and a foreign one.
#[test]
fn a_warm_rejection_keeps_the_proofs_location_over_a_partial_seed() {
    let (cold, warm) = separate_modules_cold_and_warm(spin("spin"), &[("spin", partial_meta())]);

    assert_eq!(
        location_of(&cold),
        (Some(PathBuf::from("b.tg")), AXIOM_SPAN)
    );
    assert_eq!(location_of(&warm), location_of(&cold));
}

#[test]
fn a_warm_rejection_keeps_the_proofs_location_over_a_foreign_seed() {
    let (cold, warm) = separate_modules_cold_and_warm(foreign_wrapper(), &[]);

    assert_eq!(
        location_of(&cold),
        (Some(PathBuf::from("b.tg")), AXIOM_SPAN)
    );
    assert_eq!(location_of(&warm), location_of(&cold));
}

// 3.10.26r AC3: the axiom and the extern it names cached together — the
// replayed rejection and the recomputed one are one error, located.
#[test]
fn a_same_module_warm_rejection_prints_once_at_the_proof() {
    let mut carried = CachedTermination::from_module(
        &[foreign_wrapper(), axiom_over("g")],
        &meta(&[("ax", proof_meta())]),
    );
    carried.locate_in(Path::new("b.tg"));
    assert_eq!(
        carried.failures.len(),
        1,
        "the module's own check rejects it"
    );
    let mut sites = DefinitionSites::default();
    sites.add_carried(&carried);
    let input = TerminationInput::from_meta(&HashMap::new());
    let recomputed = input.rejections(
        &input.check_with_carried(&[], &carried.as_carried()),
        &sites,
    );
    assert_eq!(recomputed.len(), 1, "the carried proof is recomputed too");

    let merged = merge_carried(recomputed, carried.failures);

    assert_eq!(
        location_of(&merged),
        (Some(PathBuf::from("b.tg")), AXIOM_SPAN)
    );
    assert!(merged[0].1, "the merged rejection stays proof-relevant");
}

fn rejection_of(proof: &str, file: Option<&str>) -> (ElabError, bool) {
    let mut error = ElabError::new(
        Span::new(5, 9),
        ElabErrorKind::PartialInProof {
            proof: proof.to_string(),
            tainted: "spin".to_string(),
        },
    );
    error.file_path = file.map(PathBuf::from);
    (error, true)
}

#[test]
fn two_proofs_rejected_for_the_same_reason_stay_two() {
    let merged = merge_carried(
        vec![rejection_of("first", Some("b.tg"))],
        vec![rejection_of("second", Some("b.tg"))],
    );

    assert_eq!(merged.len(), 2);
}

#[test]
fn of_a_pair_the_located_recomputed_rejection_is_kept() {
    let mut recomputed = rejection_of("ax", Some("recomputed.tg"));
    recomputed.0.span = Span::new(1, 1);

    let merged = merge_carried(
        vec![recomputed],
        vec![rejection_of("ax", Some("carried.tg"))],
    );

    assert_eq!(
        location_of(&merged),
        (Some(PathBuf::from("recomputed.tg")), Span::new(1, 1))
    );
}

#[test]
fn of_a_pair_the_carried_rejection_replaces_an_unlocated_recomputed_one() {
    let merged = merge_carried(
        vec![rejection_of("ax", None)],
        vec![rejection_of("ax", Some("carried.tg"))],
    );

    assert_eq!(
        location_of(&merged),
        (Some(PathBuf::from("carried.tg")), Span::new(5, 9))
    );
}

#[test]
fn a_different_reason_for_the_same_proof_is_not_merged() {
    let mut other_reason = rejection_of("ax", Some("b.tg"));
    other_reason.0.message = "a different reason".to_string();

    let merged = merge_carried(vec![rejection_of("ax", Some("b.tg"))], vec![other_reason]);

    assert_eq!(merged.len(), 2);
}

#[test]
fn errors_the_termination_gate_does_not_raise_are_never_merged() {
    let unrelated = || {
        (
            ElabError::new(
                Span::new(0, 1),
                ElabErrorKind::UndefinedVariable("x".into()),
            ),
            false,
        )
    };

    assert_eq!(merge_carried(vec![unrelated()], vec![unrelated()]).len(), 2);
}

#[test]
fn two_carried_rejections_are_never_merged_with_each_other() {
    let merged = merge_carried(
        Vec::new(),
        vec![
            rejection_of("ax", Some("b.tg")),
            rejection_of("ax", Some("b.tg")),
        ],
    );

    assert_eq!(merged.len(), 2);
}

#[test]
fn a_cannot_prove_termination_pair_is_keyed_by_its_function() {
    let unproven = |function: &str| {
        (
            ElabError::new(
                Span::new(0, 1),
                ElabErrorKind::CannotProveTermination {
                    function: function.to_string(),
                    headline: "no decreasing argument".to_string(),
                },
            ),
            true,
        )
    };

    assert_eq!(
        merge_carried(vec![unproven("f")], vec![unproven("f")]).len(),
        1
    );
    assert_eq!(
        merge_carried(vec![unproven("f")], vec![unproven("g")]).len(),
        2
    );
}

#[test]
fn a_foreign_pair_is_keyed_by_its_proof() {
    let foreign = |proof: &str| {
        (
            ElabError::new(
                Span::new(0, 1),
                ElabErrorKind::ForeignInProof {
                    proof: proof.to_string(),
                    via: vec!["g".to_string()],
                    extern_symbol: "g".to_string(),
                },
            ),
            true,
        )
    };

    assert_eq!(
        merge_carried(vec![foreign("ax")], vec![foreign("ax")]).len(),
        1
    );
    assert_eq!(
        merge_carried(vec![foreign("ax")], vec![foreign("bx")]).len(),
        2
    );
}
