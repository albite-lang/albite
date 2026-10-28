//! Tests: the report — the reach line, the severity scope, and the JSON.
//!
//! Split from `tests.rs` (the census) because the two halves fail differently:
//! a census bug reports the wrong findings, a report bug reports the right ones
//! unreadably — and `0 collisions` rendering like `0 modules examined` is a
//! report bug that no census assertion can see.

use crate::ast::Visibility;

use super::census::{ReexportHandling, Severity};
use super::report::{reach_line, render_human, render_json};
use super::select;
use super::test_support::TreeBuilder;

/// §2.1: `0 collisions` and `0 modules examined` are different outcomes. This
/// is the failure ADR 11.8.26c shipped and repaired in its own retrospective.
#[test]
fn a_clean_tree_and_an_empty_one_do_not_render_alike() {
    let clean = TreeBuilder::default()
        .define("elab::report", "render", Visibility::Private)
        .define("driver::util", "emit", Visibility::Private);
    let clean = clean.census(ReexportHandling::Subtract);

    let empty = TreeBuilder::default();
    let empty = empty.census(ReexportHandling::Subtract);

    assert_eq!(clean.collisions.len(), empty.collisions.len(), "both clean");
    assert_ne!(
        reach_line(&clean),
        reach_line(&empty),
        "and yet they must not read alike"
    );
    assert!(reach_line(&clean).contains("examined 2 module(s)"));
    assert!(reach_line(&empty).contains("examined 0 module(s)"));
    assert_ne!(
        render_human(&clean, &[], Severity::All),
        render_human(&empty, &[], Severity::All)
    );
}

/// A module registered but defining nothing still counts toward the reach line —
/// the number a reader uses to tell "clean" from "pointed at nothing".
#[test]
fn the_reach_line_counts_modules_definitions_and_names_separately() {
    let tree = TreeBuilder::default()
        .define("a", "one", Visibility::Public)
        .define("b", "one", Visibility::Public)
        .define("b", "two", Visibility::Public)
        .empty_module("c");

    let census = tree.census(ReexportHandling::Subtract);

    assert_eq!(census.modules_examined, 3, "the empty module counts");
    assert_eq!(census.names_considered, 2, "distinct names");
    assert_eq!(census.definitions_considered, 3, "definition sites");
}

/// The report prints every candidate, each beside its own Core name (ADR
/// 4.10.26j D8); no line claims a winner, because no definition wins.
#[test]
fn the_human_report_names_both_modules_with_their_core_names() {
    let tree = TreeBuilder::default()
        .define("elab::error::source", "string_slice", Visibility::Private)
        .define("harness::rewrite", "string_slice", Visibility::Private);
    let census = tree.census(ReexportHandling::Subtract);

    let rendered = render_human(&census, &select(&census, Severity::All), Severity::All);

    assert!(rendered.contains("[scoped]"));
    assert!(rendered.contains("(private)"));
    // Each Core name on its own module's line, which a bare `contains` could
    // not tell from two names swapped between the candidates.
    let line_of = |module: &str| {
        rendered
            .lines()
            .find(|line| line.trim_start().starts_with(module))
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        line_of("elab::error::source").contains("`_tg_4elab_5error_6source_12string_slice`"),
        "{rendered}"
    );
    assert!(
        line_of("harness::rewrite").contains("`_tg_7harness_7rewrite_12string_slice`"),
        "{rendered}"
    );
    assert!(!rendered.contains("wins"), "{rendered}");
    assert!(
        rendered.contains("advisory: exit is 0"),
        "the zero exit is explained where it could be misread"
    );
}

/// The advisory line is absent when there is nothing to misread.
#[test]
fn a_clean_report_carries_no_advisory() {
    let tree = TreeBuilder::default().define("a", "one", Visibility::Public);
    let census = tree.census(ReexportHandling::Subtract);

    let rendered = render_human(&census, &[], Severity::All);
    assert!(!rendered.contains("advisory"));
    assert!(rendered.contains("✓ No name collisions"));
}

/// The two `--severity` values must not render the same header, or a reader
/// cannot tell a filtered clean run from an unfiltered one.
#[test]
fn the_severity_scope_is_stated_in_the_report() {
    let tree = TreeBuilder::default().define("a", "one", Visibility::Public);
    let census = tree.census(ReexportHandling::Subtract);

    assert_ne!(
        render_human(&census, &[], Severity::All),
        render_human(&census, &[], Severity::Live)
    );
    assert!(render_human(&census, &[], Severity::Live).contains("live classes only"));
}

/// The JSON carries the same facts, so a caller can gate on findings the human
/// report deliberately does not gate on (D3).
#[test]
fn the_json_carries_the_core_names_the_classes_and_the_reach_counts() {
    let tree = TreeBuilder::default()
        .define("elab::error::source", "string_slice", Visibility::Private)
        .define("harness::rewrite", "string_slice", Visibility::Private);
    let census = tree.census(ReexportHandling::Subtract);

    let json: serde_json::Value = serde_json::from_str(&render_json(
        &census,
        &select(&census, Severity::All),
        Severity::All,
    ))
    .expect("valid JSON");

    assert_eq!(json["collision_count"], 1);
    assert_eq!(json["modules_examined"], 2);
    assert_eq!(json["names_considered"], 1);
    assert_eq!(json["definitions_considered"], 2);
    assert_eq!(json["severity"], "all");
    assert_eq!(json["collisions"][0]["class"], "scoped");
    assert_eq!(json["collisions"][0]["crosses_packages"], false);
    assert_eq!(
        json["collisions"][0]["sites"][1]["core_name"],
        "_tg_7harness_7rewrite_12string_slice"
    );
    assert_eq!(json["collisions"][0]["sites"][0]["visibility"], "private");
    assert_eq!(json["collisions"][0]["sites"][0]["extern_c"], false);
}

/// `--severity live` narrows the JSON too, header included.
#[test]
fn the_json_reports_the_severity_it_was_filtered_by() {
    let tree = TreeBuilder::default()
        .define("a", "one", Visibility::Public)
        .define("b", "one", Visibility::Public);
    let census = tree.census(ReexportHandling::Subtract);

    let json: serde_json::Value = serde_json::from_str(&render_json(
        &census,
        &select(&census, Severity::Live),
        Severity::Live,
    ))
    .expect("valid JSON");

    assert_eq!(json["severity"], "live");
    assert_eq!(json["collision_count"], 0);
    assert_eq!(
        json["names_considered"], 1,
        "the reach line is the whole run, not the filtered view"
    );
}

/// A name defined once is not a finding, however many modules exist.
#[test]
fn a_single_definition_is_not_a_collision() {
    let tree = TreeBuilder::default()
        .define("a", "one", Visibility::Private)
        .define("b", "two", Visibility::Private);

    let census = tree.census(ReexportHandling::Subtract);
    assert_eq!(census.collisions.len(), 0);
    assert_eq!(census.names_considered, 2);
}

/// Findings are ordered by name and sites by module path, so two runs over the
/// same tree cannot print different reports.
#[test]
fn the_report_order_is_deterministic() {
    let tree = TreeBuilder::default()
        .define("zeta", "beta", Visibility::Private)
        .define("alpha", "beta", Visibility::Private)
        .define("mid", "alpha", Visibility::Private)
        .define("other", "alpha", Visibility::Private);

    let census = tree.census(ReexportHandling::Subtract);

    let names: Vec<&str> = census.collisions.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta"]);
    let modules: Vec<String> = census.collisions[1]
        .sites
        .iter()
        .map(|s| s.module.to_string())
        .collect();
    assert_eq!(modules, vec!["alpha", "zeta"]);
}

/// Each class prints the remedy that actually applies to it, and no two are
/// alike: an `extern` pair must be renamed, and a scoped pair needs nothing.
#[test]
fn each_class_prints_its_own_remedy() {
    let extern_pair = TreeBuilder::default()
        .define_extern("driver::ffi::a", "tg_report", Visibility::Public)
        .define_extern("driver::ffi::b", "tg_report", Visibility::Public)
        .census(ReexportHandling::Subtract);
    let scoped = TreeBuilder::default()
        .define("elab::report", "render", Visibility::Private)
        .define("driver::util", "render", Visibility::Private)
        .census(ReexportHandling::Subtract);

    let rendered = |census: &_| render_human(census, &select(census, Severity::All), Severity::All);
    let (a, b) = (rendered(&extern_pair), rendered(&scoped));

    assert!(
        a.contains("the Albite identifier IS the C symbol"),
        "no visibility change helps a duplicate C symbol — {a}"
    );
    assert!(
        b.contains("information: each definition has its own Core name"),
        "a scoped pair is information — {b}"
    );
    assert!(b.contains("E0019"), "and names the bare-call refusal — {b}");
    assert_ne!(a, b, "two classes, two remedies");
}

/// The JSON says whether a collision crosses packages, the fact that turns an
/// `extern` pair into the build's E0123 (ADR 4.10.26j D8).
#[test]
fn the_json_says_whether_a_collision_crosses_packages() {
    let census = TreeBuilder::default()
        .define_extern("lib_a", "tg_x", Visibility::Public)
        .define_extern("lib_b", "tg_x", Visibility::Public)
        .in_package("lib_a", "lib_a")
        .in_package("lib_b", "lib_b")
        .census(ReexportHandling::Subtract);
    let json: serde_json::Value = serde_json::from_str(&render_json(
        &census,
        &select(&census, Severity::All),
        Severity::All,
    ))
    .expect("valid JSON");
    assert_eq!(json["collisions"][0]["class"], "extern-symbol");
    assert_eq!(json["collisions"][0]["crosses_packages"], true);
}

/// `pub(crate)` renders as itself in both surfaces. Neither arm was exercised
/// before, so a label swapped for `private` would have printed the wrong remedy
/// class alongside a correct classification.
#[test]
fn crate_visibility_is_rendered_distinctly_in_both_surfaces() {
    let census = TreeBuilder::default()
        .define("elab::report", "render", Visibility::Crate)
        .define("driver::util", "render", Visibility::Public)
        .census(ReexportHandling::Subtract);
    let reported = select(&census, Severity::All);

    let human = render_human(&census, &reported, Severity::All);
    assert!(human.contains("(pub(crate))"), "{human}");
    assert!(human.contains("(pub)"), "and the plain one is still itself");
    assert!(!human.contains("(private)"), "nothing here is private");

    let json: serde_json::Value =
        serde_json::from_str(&render_json(&census, &reported, Severity::All)).expect("valid JSON");
    let visibilities: Vec<&str> = json["collisions"][0]["sites"]
        .as_array()
        .expect("sites array")
        .iter()
        .map(|s| s["visibility"].as_str().expect("string"))
        .collect();
    assert!(visibilities.contains(&"crate"), "{visibilities:?}");
    assert!(visibilities.contains(&"pub"), "{visibilities:?}");
}
