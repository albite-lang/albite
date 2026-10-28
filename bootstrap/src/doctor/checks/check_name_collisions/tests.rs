//! Tests: the census — which names collide, how they are classified, and the
//! Core name each definition elaborates under (ADR 4.10.26j D8).
//!
//! Every test drives the pure `census` over a hand-built `ModuleInfo`, so
//! nothing here parses a file or elaborates one. Rendering is next door in
//! `report_tests.rs`; the end-to-end binding — that the check really does run
//! on a file the elaborator rejects — is the `name-collisions` row in
//! `check_tool_reachability`'s `PAIRINGS`.

use crate::ast::Visibility;

use super::census::{CollisionClass, ReexportHandling, Severity};
use super::report::render_human;
use super::select;
use super::test_support::TreeBuilder;

/// The shape ADR 7.8.26b hit three times — two private definitions of one
/// name — is now `scoped`: each keeps its own Core name, so neither shadows
/// the other (ADR 4.10.26j D8).
#[test]
fn two_private_definitions_are_scoped_with_their_own_core_names() {
    let tree = TreeBuilder::default()
        .define("elab::error::source", "string_slice", Visibility::Private)
        .define(
            "driver::pipeline::test::harness::rewrite",
            "string_slice",
            Visibility::Private,
        );

    let census = tree.census(ReexportHandling::Subtract);

    assert_eq!(census.collisions.len(), 1, "one name, two modules");
    let finding = &census.collisions[0];
    assert_eq!(finding.name, "string_slice");
    assert_eq!(finding.class, CollisionClass::Scoped);
    assert!(!finding.crosses_packages);
    let core_names: Vec<&str> = finding.sites.iter().map(|s| s.core_name.as_str()).collect();
    assert_eq!(
        core_names,
        [
            "_tg_6driver_8pipeline_4test_7harness_7rewrite_12string_slice",
            "_tg_4elab_5error_6source_12string_slice",
        ]
    );
}

/// Visibility no longer classifies: a `pub` pair, a `pub(crate)` pair and a
/// mixed pair are all `scoped`, which `--severity live` drops and `all` keeps.
#[test]
fn every_visibility_mix_is_scoped_and_only_severity_all_reports_it() {
    for (first, second) in [
        (Visibility::Public, Visibility::Public),
        (Visibility::Crate, Visibility::Crate),
        (Visibility::Crate, Visibility::Private),
        (Visibility::Public, Visibility::Private),
    ] {
        let census = TreeBuilder::default()
            .define("driver::util", "render", first)
            .define("elab::report", "render", second)
            .census(ReexportHandling::Subtract);
        assert_eq!(census.collisions[0].class, CollisionClass::Scoped);
        assert_eq!(select(&census, Severity::All).len(), 1);
        assert_eq!(
            select(&census, Severity::Live).len(),
            0,
            "scoped is information, not an error"
        );
    }
}

/// Class (c): the Tungsten identifier IS the C symbol, so an `extern` pair is a
/// duplicate link symbol module scoping cannot separate. It is the one live class.
#[test]
fn an_extern_pair_is_the_one_live_class() {
    let tree = TreeBuilder::default()
        .define_extern("driver::ffi::diagnostics", "tg_report", Visibility::Private)
        .define_extern(
            "driver::ffi::diagnostics_dev",
            "tg_report",
            Visibility::Public,
        );

    let census = tree.census(ReexportHandling::Subtract);

    assert_eq!(census.collisions[0].class, CollisionClass::ExternSymbol);
    assert!(CollisionClass::ExternSymbol.is_live());
    assert!(!CollisionClass::Scoped.is_live());
    assert_eq!(select(&census, Severity::Live).len(), 1);
}

/// A single `extern` definition colliding with a plain `fn` is NOT the link
/// class: there is only one C symbol, and the two values are scoped.
#[test]
fn one_extern_against_a_plain_fn_is_not_the_link_class() {
    let tree = TreeBuilder::default()
        .define_extern("driver::ffi", "tg_report", Visibility::Public)
        .define("elab::report", "tg_report", Visibility::Private);

    let census = tree.census(ReexportHandling::Subtract);

    assert_eq!(census.collisions[0].class, CollisionClass::Scoped);
}

/// D8: an extern pair in two packages is the build's own E0123, and says so.
#[test]
fn an_extern_pair_across_packages_names_e0123() {
    let tree = TreeBuilder::default()
        .define_extern("lib_a", "tg_x", Visibility::Public)
        .define_extern("lib_b", "tg_x", Visibility::Public)
        .in_package("lib_a", "lib_a")
        .in_package("lib_b", "lib_b");
    let census = tree.census(ReexportHandling::Subtract);
    assert!(census.collisions[0].crosses_packages);
    let rendered = render_human(&census, &select(&census, Severity::All), Severity::All);
    assert!(rendered.contains("E0123"), "{rendered}");

    let one_package = TreeBuilder::default()
        .define_extern("lib_a", "tg_x", Visibility::Public)
        .define_extern("lib_a::inner", "tg_x", Visibility::Public)
        .in_package("lib_a", "lib_a")
        .census(ReexportHandling::Subtract);
    assert!(!one_package.collisions[0].crosses_packages);
    let rendered = render_human(
        &one_package,
        &select(&one_package, Severity::All),
        Severity::All,
    );
    assert!(!rendered.contains("E0123"), "{rendered}");
    assert!(rendered.contains("rename one"), "{rendered}");
}

/// D1b: a re-exported name is ONE definition reachable by several paths, which
/// is `reexport-completeness`'s question. Subtracting the synthesized copies is
/// what keeps `driver::util`'s chain from reporting.
#[test]
fn a_reexport_chain_is_not_a_collision() {
    let tree = TreeBuilder::default()
        .define(
            "driver::util::strings::convert",
            "nat_to_string",
            Visibility::Public,
        )
        .reexport(
            "driver::util::strings",
            "nat_to_string",
            "driver::util::strings::convert",
        )
        .reexport("driver::util", "nat_to_string", "driver::util::strings");

    let subtracted = tree.census(ReexportHandling::Subtract);
    assert_eq!(
        subtracted.collisions.len(),
        0,
        "one definition, three reachable paths"
    );
    assert_eq!(subtracted.definitions_considered, 1);

    // The measurement arm (AC 1): keeping the copies is what makes the chain
    // look like a three-way collision, so the difference between the two runs
    // IS the re-export class.
    let kept = tree.census(ReexportHandling::Keep);
    assert_eq!(kept.collisions.len(), 1);
    assert_eq!(kept.collisions[0].sites.len(), 3);
    assert_eq!(kept.definitions_considered, 3);
}

/// §5's precise case: a module that genuinely defines `foo` AND re-exports a
/// `foo` records no provenance entry (the copy is guarded), so the real
/// definition cannot be subtracted away.
#[test]
fn a_definition_is_never_subtracted_by_a_same_named_reexport() {
    // The guard in `copy_contents_entries` means the target keeps its own
    // definition and records nothing — modelled here by defining only.
    let tree = TreeBuilder::default()
        .define("elab::report", "render", Visibility::Private)
        .define("driver::util", "render", Visibility::Private);

    let census = tree.census(ReexportHandling::Subtract);
    assert_eq!(census.collisions.len(), 1);
}
