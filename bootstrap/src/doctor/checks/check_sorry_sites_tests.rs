//! Tests for `doctor check sorry-sites` (ADR 18.9.26g AC1, ADR 13.10.26g AC4/AC5).
//!
//! The rows come from a real elaboration of the golden fixture: the claim is
//! that the elaborator marks authored holes and the lowering leaves its own
//! bare, so a hand-built `ProjectOutput` would assert nothing about either.

use std::path::Path;
use std::process::ExitCode;

use crate::driver;

use super::{
    census, cmd_check_sorry_sites, render, RestingRoot, RestingTheorem, SorryCensus, SorryRow,
};

const FIXTURE: &str = include_str!("../../../../tests/golden/check/sorry_counts.tg");
const AXIOM_CHAIN: &str =
    include_str!("../../../../tests/golden/check/theorem_rests_on_an_axiom.tg");
const SORRY_CHAIN: &str = "theorem holey(): 0 == 1 { sorry }\n\
                           theorem uses_holey(): 0 == 1 { holey() }\n";
const CLEAN: &str = "theorem fine(): 1 == 1 { refl }\n\
                     theorem uses_fine(): 1 == 1 { fine() }\n";

/// Write each `(file, source)` into `dir` and census the first.
fn census_of(dir: &Path, files: &[(&str, &str)]) -> SorryCensus {
    for (name, source) in files {
        std::fs::write(dir.join(name), source).unwrap();
    }
    let entry = dir.join(files[0].0);
    let project = driver::elaborate_project(&entry, false, 20, None).expect("fixture elaborates");
    census(&project)
}

fn single_file_census(source: &str) -> SorryCensus {
    let dir = tempfile::tempdir().unwrap();
    census_of(dir.path(), &[("main.tg", source)])
}

fn resting(name: &str, roots: &[(&str, &'static str)]) -> RestingTheorem {
    RestingTheorem {
        name: name.to_string(),
        roots: roots
            .iter()
            .map(|(root, class)| RestingRoot {
                name: (*root).to_string(),
                class,
            })
            .collect(),
    }
}

fn fixture_census() -> SorryCensus {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sorry_counts.tg");
    std::fs::write(&path, FIXTURE).unwrap();
    let project = driver::elaborate_project(&path, false, 20, None).expect("fixture elaborates");
    census(&project)
}

fn row<'a>(found: &'a SorryCensus, name: &str) -> &'a SorryRow {
    found
        .rows
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row for {name}: {found:#?}"))
}

/// 1-based line of the first fixture line containing `needle`.
fn fixture_line(needle: &str) -> usize {
    FIXTURE
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("fixture has no line containing {needle}"))
        + 1
}

// 18.9.26g AC1: the authored `sorry` is named at its file:line:col.
#[test]
fn an_authored_sorry_is_named_at_its_line() {
    let found = fixture_census();
    let authored = row(&found, "authored_hole");
    assert_eq!(authored.authored.len(), 1, "{authored:?}");
    assert!(authored.synthesised.is_empty() && authored.unclassified == 0);
    let line = fixture_line("    sorry");
    assert!(
        authored.authored[0].ends_with(&format!("sorry_counts.tg:{line}:5")),
        "{authored:?}"
    );
}

// 18.9.26g AC1: an axiom's hole is authored.
#[test]
fn an_axiom_is_one_authored_hole() {
    let found = fixture_census();
    let axiom = row(&found, "trusted");
    assert_eq!(axiom.authored.len(), 1, "{axiom:?}");
    assert!(axiom.synthesised.is_empty() && axiom.unclassified == 0);
}

// 13.10.26u AC4: the nested patterns that were 18.9.26g's absurd-branch and
// unreachable-arm examples leave no hole, so they carry no row. The labels
// themselves stay covered by the `render` tests below and by `albite_core`'s
// term-level `sorry_sites_*` tests.
#[test]
fn a_nested_pattern_leaves_no_synthesised_hole() {
    let found = fixture_census();
    for clean in ["singleton_head", "tri_payload"] {
        assert!(
            found.rows.iter().all(|r| r.name != clean),
            "{clean}: {found:#?}"
        );
    }
}

// 18.9.26g AC1: `==` with no equality primitive plants one unclassified hole.
#[test]
fn equality_without_a_primitive_is_unclassified() {
    let found = fixture_census();
    let eq = row(&found, "same_option");
    assert_eq!(eq.unclassified, 1, "{eq:?}");
    assert!(eq.authored.is_empty() && eq.synthesised.is_empty());
}

// 18.9.26g AC1: an authored hole inside an inner match is named; since
// 13.10.26u the match around it plants none of its own.
#[test]
fn an_authored_hole_in_an_inner_match_is_the_only_one() {
    let found = fixture_census();
    let mixed = row(&found, "mixed_hole");
    assert_eq!(mixed.authored.len(), 1, "{mixed:?}");
    assert!(mixed.synthesised.is_empty(), "{mixed:?}");
}

// 18.9.26g AC1: totals sum the rows, and every examined definition is counted.
#[test]
fn totals_sum_the_rows() {
    let found = fixture_census();
    assert_eq!(found.rows.len(), 4, "{found:#?}");
    assert!(found.examined >= found.rows.len());
    let authored: usize = found.rows.iter().map(|r| r.authored.len()).sum();
    let synthesised: usize = found.rows.iter().map(|r| r.synthesised.len()).sum();
    let unclassified: usize = found.rows.iter().map(|r| r.unclassified).sum();
    assert_eq!(
        (found.authored, found.synthesised, found.unclassified),
        (authored, synthesised, unclassified)
    );
}

#[test]
fn the_report_names_each_row_and_the_totals() {
    let found = SorryCensus {
        examined: 4,
        rows: vec![SorryRow {
            name: "f".to_string(),
            authored: vec!["a.tg:2:5".to_string()],
            synthesised: vec!["absurd branch"],
            unclassified: 1,
        }],
        authored: 1,
        synthesised: 1,
        unclassified: 1,
        rests_on: vec![],
    };
    let text = render(&found, "a.tg");
    assert!(
        text.starts_with("⚠ 1 of 4 definition(s) carry a sorry in a.tg:"),
        "{text}"
    );
    assert!(text.contains("  f\n"), "{text}");
    assert!(text.contains("authored      a.tg:2:5"), "{text}");
    assert!(text.contains("synthesised   absurd branch"), "{text}");
    assert!(text.contains("unclassified  1"), "{text}");
    assert!(
        text.contains("totals: 1 authored, 1 synthesised, 1 unclassified"),
        "{text}"
    );
}

#[test]
fn a_clean_report_says_how_many_it_examined() {
    let found = SorryCensus {
        examined: 0,
        rows: vec![],
        authored: 0,
        synthesised: 0,
        unclassified: 0,
        rests_on: vec![],
    };
    assert_eq!(
        render(&found, "a.tg"),
        "✓ no definition carries a sorry in a.tg (0 definition(s) examined)\n"
    );
}

#[test]
fn a_row_with_no_unclassified_hole_prints_no_unclassified_line() {
    let found = SorryCensus {
        examined: 1,
        rows: vec![SorryRow {
            name: "g".to_string(),
            authored: vec![],
            synthesised: vec!["unreachable pattern arm"],
            unclassified: 0,
        }],
        authored: 0,
        synthesised: 1,
        unclassified: 0,
        rests_on: vec![],
    };
    let text = render(&found, "a.tg");
    assert!(!text.contains("unclassified  "), "{text}");
}

// A file that does not elaborate is a failure, not an empty census: findings
// exit 0, so the exit code is the only thing that tells the two apart. (A
// path that does not exist is exit 2 since ADR 15.10.26a D4, so the fixture
// is a real file that fails to type-check.)
#[test]
fn a_file_that_does_not_elaborate_exits_failure() {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("broken.tg");
    std::fs::write(&broken, "pub fn broken() -> Nat { true }\n").unwrap();
    assert_eq!(
        cmd_check_sorry_sites(&broken, false, false),
        ExitCode::FAILURE
    );
}

#[test]
fn the_fixture_exits_success_in_both_formats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sorry_counts.tg");
    std::fs::write(&path, FIXTURE).unwrap();
    assert_eq!(
        cmd_check_sorry_sites(&path, false, false),
        ExitCode::SUCCESS
    );
    assert_eq!(cmd_check_sorry_sites(&path, true, false), ExitCode::SUCCESS);
}

// 13.10.26g AC4: one hop, two hops and a tactic proof each rest on the axiom,
// and the axiom itself is not listed.
#[test]
fn every_theorem_citing_an_axiom_is_named_with_its_root() {
    let found = single_file_census(AXIOM_CHAIN);
    assert_eq!(
        found.rests_on,
        vec![
            resting("bad2", &[("ax", "axiom")]),
            resting("bad3", &[("ax", "axiom")]),
            resting("bad4", &[("ax", "axiom")]),
        ]
    );
}

// 13.10.26g AC4: a theorem citing a `sorry` proof rests on it, classed `sorry`.
#[test]
fn a_theorem_citing_a_sorry_proof_rests_on_it() {
    let found = single_file_census(SORRY_CHAIN);
    assert_eq!(
        found.rests_on,
        vec![resting("uses_holey", &[("holey", "sorry")])]
    );
}

// 13.10.26g D1: a `fn` calling an axiom is walked through but never listed —
// only the theorem citing it is.
#[test]
fn a_program_definition_is_walked_through_not_listed() {
    let found = single_file_census(
        "axiom ax(): Void\n\
         fn helper() -> Void { ax() }\n\
         theorem via_helper(): Void { helper() }\n",
    );
    assert_eq!(
        found.rests_on,
        vec![resting("via_helper", &[("ax", "axiom")])]
    );
}

// 13.10.26g AC4: the clean control rests on nothing.
#[test]
fn a_clean_citation_rests_on_nothing() {
    assert!(single_file_census(CLEAN).rests_on.is_empty());
}

// 13.10.26g AC4: the 18.9.26g fixture's axiom is cited by no theorem.
#[test]
fn the_sorry_counts_fixture_has_no_resting_theorem() {
    assert!(fixture_census().rests_on.is_empty());
}

// 13.10.26g AC4: the text report names each theorem with its classed roots.
#[test]
fn the_report_names_each_resting_theorem_and_its_roots() {
    let text = render(&single_file_census(AXIOM_CHAIN), "ax.tg");
    assert!(
        text.contains("\n3 theorem(s) rest on an axiom or sorry:\n\n"),
        "{text}"
    );
    assert!(text.contains("  bad3  rests on ax (axiom)\n"), "{text}");
    let both = SorryCensus {
        rests_on: vec![resting("t", &[("ax", "axiom"), ("holey", "sorry")])],
        ..single_file_census(SORRY_CHAIN)
    };
    assert!(
        render(&both, "a.tg").contains("  t  rests on ax (axiom), holey (sorry)\n"),
        "{both:?}"
    );
}

// 13.10.26g AC4: a census with no resting theorem prints no section.
#[test]
fn a_census_with_no_resting_theorem_prints_no_section() {
    let text = render(&fixture_census(), "a.tg");
    assert!(!text.contains("rest on an axiom"), "{text}");
}

// 13.10.26g AC4: `--json` carries `rests_on` with each root's name and class.
#[test]
fn the_json_carries_rests_on() {
    let json = serde_json::to_value(single_file_census(SORRY_CHAIN)).unwrap();
    assert_eq!(
        json["rests_on"],
        serde_json::json!([{ "name": "uses_holey", "roots": [{ "name": "holey", "class": "sorry" }] }])
    );
    let clean = serde_json::to_value(single_file_census(CLEAN)).unwrap();
    assert_eq!(clean["rests_on"], serde_json::json!([]));
}

// 13.10.26g AC5: across modules, cold then warm, `bad2` rests on `trusted::ax`.
#[test]
fn a_theorem_rests_on_an_axiom_in_another_module_cold_and_warm() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "main.tg",
            "mod trusted;\nuse trusted::{ax};\ntheorem bad2(): Void { ax() }\n",
        ),
        ("trusted.tg", "pub axiom ax(): Void\n"),
    ];
    let cold = census_of(dir.path(), &files);
    let warm = census_of(dir.path(), &files);
    for found in [cold, warm] {
        assert_eq!(found.rests_on, vec![resting("bad2", &[("ax", "axiom")])]);
    }
}
