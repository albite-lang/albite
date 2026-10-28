//! `diff term-size` (ADR 16.10.26w) over inline census JSON: the comparison
//! through the command's own entry point (AC 1), every refusal naming its
//! side (AC 2) and the exit contract (AC 3). Expectations are written out by
//! hand, never derived from `delta_rows`.

use albite_bootstrap::doctor::checks::check_proof_term_size::delta::parse_baseline;
use albite_bootstrap::doctor::checks::check_proof_term_size::{Population, Thresholds};
use clap::Parser;
use serde_json::{json, Value};

use super::*;
use crate::cli::Cli;

/// One census row: `nodes` `None` is an axiom.
fn census_row(name: &str, nodes: Option<usize>) -> Value {
    json!({
        "module": "m",
        "name": name,
        "axiom": nodes.is_none(),
        "subst": nodes.map(|_| 0),
        "refl": nodes.map(|_| 0),
        "global_occurrences": nodes.map(|_| 0),
        "nodes": nodes,
        "case_arms": nodes.map(|_| 1),
    })
}

fn census(population: &str, rows: &[(&str, Option<usize>)]) -> String {
    let rows: Vec<Value> = rows
        .iter()
        .map(|(name, nodes)| census_row(name, *nodes))
        .collect();
    json!({ "file": "f.tg", "population": population, "rows": rows }).to_string()
}

fn before_census() -> String {
    census(
        "all",
        &[
            ("a", Some(100)),
            ("ax", None),
            ("b", Some(1000)),
            ("c", Some(50)),
            ("gone", Some(10)),
        ],
    )
}

fn after_census() -> String {
    census(
        "all",
        &[
            ("a", Some(400)),
            ("ax", None),
            ("b", Some(1200)),
            ("c", Some(50)),
            ("fresh", Some(600)),
            ("small", Some(5)),
        ],
    )
}

/// Non-default thresholds: `a` (+300, ×4) passes both, `b` (+200, ×1.2)
/// neither, `fresh` (600 nodes) is a flagged new row.
fn strict(top: Option<usize>, json: bool) -> TermSizeDiffRequest {
    TermSizeDiffRequest {
        thresholds: Thresholds {
            max_growth: 2.0,
            min_delta: 250,
        },
        top,
        json,
    }
}

fn diff(before: &str, after: &str, request: &TermSizeDiffRequest) -> (String, String, u8) {
    diff_term_size_output(
        ("before.json", Ok(before.to_string())),
        ("after.json", Ok(after.to_string())),
        request,
    )
}

fn names(values: &Value) -> Vec<String> {
    values
        .as_array()
        .expect("array")
        .iter()
        .map(|row| row["name"].as_str().expect("name").to_string())
        .collect()
}

/// 16.10.26w AC1: matched order, new/removed and flags under non-default
/// thresholds, from the JSON form.
#[test]
fn two_saved_censuses_compare_in_growth_order() {
    let (stdout, stderr, code) = diff(&before_census(), &after_census(), &strict(None, true));
    assert_eq!((stderr.as_str(), code), ("", 1));
    let report: Value = serde_json::from_str(&stdout).expect("one object");
    assert_eq!(names(&report["matched"]), ["a", "b", "c", "ax"]);
    let flagged: Vec<bool> = report["matched"]
        .as_array()
        .expect("matched")
        .iter()
        .map(|row| row["flagged"].as_bool().expect("flag"))
        .collect();
    assert_eq!(flagged, [true, false, false, false]);
    assert_eq!(names(&report["new"]), ["fresh", "small"]);
    assert_eq!(report["new"][0]["flagged"], json!(true));
    assert_eq!(report["new"][1]["flagged"], json!(false));
    assert_eq!(names(&report["removed"]), ["gone"]);
    assert_eq!(report["file"], json!("after.json"));
    assert_eq!(report["baseline"], json!("before.json"));
    assert_eq!(report["totals"]["flagged"], json!(2));
}

/// 16.10.26w AC1: `--top 1` limits the printed matched and `largest now:`
/// rows; the totals line still covers every row.
#[test]
fn top_limits_the_printed_rows_and_not_the_totals() {
    let (stdout, _, code) = diff(&before_census(), &after_census(), &strict(Some(1), false));
    assert_eq!(code, 1);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines[0],
        "core size: after.json  vs  before.json   (population: all)"
    );
    let new_at = lines
        .iter()
        .position(|l| l.starts_with("new: "))
        .expect("new");
    let matched = &lines[2..new_at];
    assert_eq!(matched.len(), 1, "{stdout}");
    assert!(matched[0].trim_start().starts_with("a "), "{stdout}");
    assert!(matched[0].ends_with("FLAGGED"), "{stdout}");
    let largest_at = lines
        .iter()
        .position(|l| *l == "largest now:")
        .expect("largest");
    assert!(
        lines[largest_at + 1].trim_start().starts_with("b "),
        "{stdout}"
    );
    assert_eq!(
        lines[largest_at + 2],
        "nodes 1160 → 2255; 2 flagged (max-growth 2, min-delta 250)"
    );
    assert_eq!(lines.len(), largest_at + 3, "{stdout}");
}

/// 16.10.26w AC1 (D3): the `--json --top 1` output is itself a baseline,
/// carrying every after-side row.
#[test]
fn json_output_round_trips_as_a_baseline() {
    let (stdout, _, _) = diff(&before_census(), &after_census(), &strict(Some(1), true));
    let next = parse_baseline(&stdout).expect("a baseline");
    assert_eq!(next.population, "all");
    let rows: Vec<&str> = next.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(rows, ["a", "ax", "b", "c", "fresh", "small"]);
    assert_eq!(next.rows[0].size.map(|s| s.nodes), Some(400));
}

/// 16.10.26w AC1 (D2): a delta report is accepted on either side.
#[test]
fn a_delta_report_is_accepted_on_either_side() {
    let (delta_report, _, _) = diff(&before_census(), &after_census(), &strict(None, true));
    let (as_before, _, code) = diff(&delta_report, &after_census(), &strict(None, true));
    assert_eq!(code, 0, "after vs itself flags nothing");
    let report: Value = serde_json::from_str(&as_before).expect("object");
    assert_eq!(report["totals"]["matched"], json!(6));
    let (as_after, _, code) = diff(&before_census(), &delta_report, &strict(None, true));
    assert_eq!(code, 1);
    let report: Value = serde_json::from_str(&as_after).expect("object");
    assert_eq!(names(&report["removed"]), ["gone"]);
}

/// Each malformed report D2 refuses, with the reason fragment it names.
fn malformed_reports() -> Vec<(&'static str, String)> {
    let missing_nodes = json!({
        "population": "all",
        "rows": [{ "module": "m", "name": "a", "axiom": false, "subst": 0,
                   "refl": 0, "global_occurrences": 0, "case_arms": 0 }],
    });
    vec![
        ("not one JSON object", "not json".to_string()),
        ("error report", json!({ "error": "boom" }).to_string()),
        ("missing \"nodes\"", missing_nodes.to_string()),
        (
            "appears twice",
            census("all", &[("a", Some(1)), ("a", Some(2))]),
        ),
        ("no \"population\"", json!({ "rows": [] }).to_string()),
    ]
}

/// 16.10.26w AC2: every refusal exits 2 naming the side and path it read.
#[test]
fn each_malformed_report_is_refused_naming_its_side() {
    let reports = malformed_reports();
    // The five D2 names; an empty list would pass the loop below vacuously.
    assert_eq!(reports.len(), 5);
    for (reason, bad) in reports {
        let (_, stderr, code) = diff(&bad, &after_census(), &strict(None, false));
        assert_eq!(code, 2, "{reason}");
        assert!(
            stderr.starts_with("error: before before.json: "),
            "{stderr}"
        );
        assert!(stderr.contains(reason), "{reason}: {stderr}");
        let (_, stderr, code) = diff(&before_census(), &bad, &strict(None, false));
        assert_eq!(code, 2, "{reason}");
        assert!(stderr.starts_with("error: after after.json: "), "{stderr}");
        assert!(stderr.contains(reason), "{reason}: {stderr}");
    }
}

/// 16.10.26w AC3: the entry point returns the code the shell exits with —
/// 0 unflagged, 1 flagged, 2 refused.
#[test]
fn the_entry_point_returns_the_shell_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let before = dir.path().join("before.json");
    let after = dir.path().join("after.json");
    std::fs::write(&before, before_census()).expect("write");
    std::fs::write(&after, after_census()).expect("write");
    let request = strict(None, true);
    assert_eq!(cmd_diff_term_size(&before, &before, &request), 0);
    assert_eq!(cmd_diff_term_size(&before, &after, &request), 1);
    let missing = dir.path().join("missing.json");
    assert_eq!(cmd_diff_term_size(&missing, &after, &request), 2);
}

/// 16.10.26w AC2: a missing path and a directory are refused on each side,
/// in text and `--json` form.
#[test]
fn an_unreadable_path_is_refused_naming_its_side() {
    let dir = tempfile::tempdir().expect("tempdir");
    let good = dir.path().join("good.json");
    std::fs::write(&good, after_census()).expect("write");
    for unreadable in [dir.path().join("missing.json"), dir.path().to_path_buf()] {
        let shown = unreadable.display().to_string();
        for json in [false, true] {
            let request = strict(None, json);
            let (stdout, stderr, code) = run_diff_term_size(&unreadable, &good, &request);
            assert_eq!(code, 2);
            assert!(
                stderr.starts_with(&format!("error: before {shown}: ")),
                "{stderr}"
            );
            assert_eq!(stdout.is_empty(), !json);
            let (stdout, stderr, code) = run_diff_term_size(&good, &unreadable, &request);
            assert_eq!(code, 2);
            assert!(
                stderr.starts_with(&format!("error: after {shown}: ")),
                "{stderr}"
            );
            if json {
                let object: Value = serde_json::from_str(&stdout).expect("one object");
                assert!(object["error"].as_str().expect("error").contains(&shown));
            }
        }
    }
}

/// 16.10.26w AC2: two populations are refused, naming both paths.
#[test]
fn a_population_mismatch_names_both_sides() {
    let proof_only = census("proof", &[("a", Some(100))]);
    let (_, stderr, code) = diff(&proof_only, &after_census(), &strict(None, false));
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "error: population differs: before before.json is \"proof\", after after.json is \"all\"\n"
    );
}

/// A population label no `term-size` run writes is refused, not rendered
/// under a guessed one.
#[test]
fn an_unknown_population_is_refused() {
    let odd = census("odd", &[("a", Some(1))]);
    let (_, stderr, code) = diff(&odd, &odd, &strict(None, false));
    assert_eq!(code, 2);
    assert!(stderr.contains("population \"odd\""), "{stderr}");
}

/// Every label `term-size` writes renders under its own population.
#[test]
fn every_written_population_label_is_read_back() {
    for population in [Population::Proof, Population::ProofTotal, Population::All] {
        assert_eq!(Population::from_label(population.label()), Some(population));
        let saved = census(population.label(), &[("a", Some(1))]);
        let (stdout, _, code) = diff(&saved, &saved, &strict(None, false));
        assert_eq!(code, 0);
        let header = format!("(population: {})", population.label());
        assert!(stdout.lines().next().expect("header").ends_with(&header));
    }
}

/// 16.10.26w AC3: 0 when nothing is flagged, 1 when a row is, 2 refused.
#[test]
fn the_exit_code_follows_the_flags() {
    let before = census("all", &[("a", Some(100))]);
    let grown = census("all", &[("a", Some(400))]);
    assert_eq!(diff(&before, &before, &strict(None, false)).2, 0);
    assert_eq!(diff(&before, &grown, &strict(None, false)).2, 1);
    assert_eq!(diff("", &grown, &strict(None, false)).2, 2);
    // The defaults (×1.5, 2000 nodes) leave +300 unflagged.
    let defaults = TermSizeDiffRequest::from_flags(None, None, None, false);
    assert_eq!(diff(&before, &grown, &defaults).2, 0);
}

/// 16.10.26w AC3 (D4): under `--json` a shell refusal prints exactly one
/// `{"error": …}` object, and nothing under text.
#[test]
fn a_json_refusal_is_one_error_object() {
    let (stdout, stderr, code) = diff("", &after_census(), &strict(None, true));
    assert_eq!(code, 2);
    assert_eq!(stdout.lines().count(), 1);
    let object: Value = serde_json::from_str(&stdout).expect("one object");
    let reason = object["error"].as_str().expect("error");
    assert_eq!(format!("error: {reason}\n"), stderr);
    assert_eq!(object.as_object().map(serde_json::Map::len), Some(1));
    let (stdout, _, _) = diff("", &after_census(), &strict(None, false));
    assert_eq!(stdout, "");
}

/// 16.10.26w AC3 (D4): a usage error is clap's, raised before the shell
/// runs, so it is never the `{"error": …}` object.
#[test]
fn a_usage_error_is_a_clap_parse_error() {
    let parse = |extra: &[&str]| {
        let mut argv = vec![
            "tungsten",
            "diff",
            "term-size",
            "b.json",
            "a.json",
            "--json",
        ];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv)
    };
    assert!(parse(&[]).is_ok());
    let unknown = parse(&["--bogus"]).err().expect("unknown flag refused");
    assert_eq!(unknown.kind(), clap::error::ErrorKind::UnknownArgument);
    let numeric = parse(&["--min-delta", "many"])
        .err()
        .expect("non-numeric refused");
    assert_eq!(numeric.kind(), clap::error::ErrorKind::ValueValidation);
}

/// D1: each flag reaches the request; an absent one takes `term-size`'s
/// default.
#[test]
fn the_flags_build_the_request() {
    let given = TermSizeDiffRequest::from_flags(Some(3.0), Some(7), Some(2), true);
    assert_eq!(given, strict_with(3.0, 7, Some(2), true));
    let defaults = TermSizeDiffRequest::from_flags(None, None, None, false);
    assert_eq!(defaults.thresholds, Thresholds::DEFAULT);
    assert_eq!((defaults.top, defaults.json), (None, false));
}

fn strict_with(
    max_growth: f64,
    min_delta: usize,
    top: Option<usize>,
    json: bool,
) -> TermSizeDiffRequest {
    TermSizeDiffRequest {
        thresholds: Thresholds {
            max_growth,
            min_delta,
        },
        top,
        json,
    }
}
