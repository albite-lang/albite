//! `term-size --baseline` (ADR 15.10.26g): every D5 refusal, the order and
//! the flag rule (AC 2), and the exit mapping (AC 3), over inline JSON and
//! hand-built rows.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::delta::{delta_rows, parse_baseline, Baseline, Thresholds};
use super::tests::{request, row, size};
use super::*;

fn nodes(nodes: usize) -> Option<ProofTermSize> {
    Some(size(0, 0, 0, nodes))
}

fn census_json(population: &str, rows: &[SizeRow]) -> String {
    let parsed: Value =
        serde_json::from_str(&render_json("f.tg", Population::All, rows, None)).expect("json");
    let mut object = parsed;
    object["population"] = json!(population);
    object.to_string()
}

fn baseline(rows: &[SizeRow]) -> Baseline {
    Baseline {
        population: "all".into(),
        rows: rows.to_vec(),
    }
}

const LOW: Thresholds = Thresholds {
    max_growth: 1.5,
    min_delta: 10,
};

/// AC2: a census round-trips as a baseline, axiom included.
#[test]
fn a_census_report_parses_as_a_baseline() {
    let rows = [row("m", "a", false, nodes(5)), row("m", "ax", false, None)];
    let parsed = parse_baseline(&census_json("all", &rows)).expect("parses");
    assert_eq!(parsed, baseline(&rows));
}

/// AC2 (D5): a delta report's own `"rows"` are the next window's baseline.
#[test]
fn a_delta_report_is_accepted_as_a_baseline() {
    let before = [row("m", "a", false, nodes(5))];
    let after = [
        row("m", "a", false, nodes(9)),
        row("m", "b", false, nodes(1)),
    ];
    let delta = delta_rows(&baseline(&before), &after, LOW);
    let header = render::DeltaHeader {
        file: "f.tg",
        baseline: "b.json",
        population: Population::All,
        thresholds: LOW,
        top: 20,
    };
    let report = render::render_delta_json(&header, &delta, &after);
    assert_eq!(parse_baseline(&report).expect("parses"), baseline(&after));
}

/// AC2 (D5): each refusal, naming the row where there is one.
#[test]
fn parse_baseline_refuses_every_bad_input() {
    let refusal = |text: &str| parse_baseline(text).expect_err(text);
    assert!(refusal("not json").starts_with("not one JSON object"));
    assert!(refusal("[1, 2]").starts_with("not one JSON object"));
    assert!(refusal(r#"{"error": "boom"}"#).contains("error report"));
    assert!(refusal(r#"{"rows": []}"#).contains("no \"population\""));
    assert!(refusal(r#"{"population": "all"}"#).contains("no \"rows\""));
    let missing_name = r#"{"population": "all", "rows": [{"module": "m"}]}"#;
    assert_eq!(refusal(missing_name), "row 0: missing \"name\"");
    let missing_arms = r#"{"population": "all", "rows": [{"module": "m", "name": "f",
        "axiom": false, "subst": 0, "refl": 0, "global_occurrences": 0, "nodes": 3}]}"#;
    assert_eq!(refusal(missing_arms), "row 0: m::f: missing \"case_arms\"");
    let missing_axiom = r#"{"population": "all", "rows": [{"module": "m", "name": "f"}]}"#;
    assert_eq!(refusal(missing_axiom), "row 0: m::f: missing \"axiom\"");
    let twice = [
        row("m", "a", false, nodes(1)),
        row("m", "a", false, nodes(2)),
    ];
    assert_eq!(
        refusal(&census_json("all", &twice)),
        "row 1: m::a appears twice"
    );
}

/// AC2 (D5): a population mismatch or absence is refused through the shell,
/// naming the baseline path, at exit 2.
#[test]
fn a_population_mismatch_is_a_refusal_naming_the_path() {
    let rows = vec![row("m", "a", false, nodes(5))];
    let asked = TermSizeRequest {
        baseline: Some(PathBuf::from("base.json")),
        ..request(Population::All, false)
    };
    let text = census_json("proof", &rows);
    let (out, err, code) = term_size_output("f.tg", &asked, Ok(rows.clone()), Some(Ok(text)));
    assert_eq!((out.as_str(), code), ("", 2));
    assert!(
        err.starts_with("error: baseline base.json: population \"proof\""),
        "{err}"
    );
    let (_, err, code) = term_size_output(
        "f.tg",
        &asked,
        Ok(rows.clone()),
        Some(Err("No such file".into())),
    );
    assert_eq!(code, 2);
    assert_eq!(err, "error: baseline base.json: No such file\n");
    let legacy = r#"{"file": "f.tg", "rows": []}"#.to_string();
    let (_, err, code) = term_size_output("f.tg", &asked, Ok(rows), Some(Ok(legacy)));
    assert_eq!(code, 2);
    assert!(err.contains("base.json: it has no \"population\""), "{err}");
}

/// AC2 (D5, D6): matched rows by delta, then ratio, then name; `new` by size,
/// `removed` by name; `largest` this run's rows by size.
#[test]
fn delta_rows_ranks_and_partitions() {
    let before = [
        row("m", "big", false, nodes(1000)),
        row("m", "gone", false, nodes(4)),
        row("m", "small", false, nodes(10)),
        row("m", "tie_b", false, nodes(20)),
        row("m", "tie_a", false, nodes(20)),
        row("m", "ax", false, None),
    ];
    let after = [
        row("m", "big", false, nodes(1100)),
        row("m", "small", false, nodes(110)),
        row("m", "tie_b", false, nodes(30)),
        row("m", "tie_a", false, nodes(30)),
        row("m", "ax", false, None),
        row("m", "fresh_small", false, nodes(3)),
        row("m", "fresh_large", false, nodes(50)),
    ];
    let delta = delta_rows(&baseline(&before), &after, LOW);
    let order: Vec<&str> = delta.matched.iter().map(|r| r.name.as_str()).collect();
    // big and small both +100; small's ratio (11) beats big's (1.1)
    assert_eq!(order, ["small", "big", "tie_a", "tie_b", "ax"]);
    assert_eq!(delta.matched[0].delta(), Some(100));
    assert_eq!(delta.matched[4].delta(), None);
    let new: Vec<&str> = delta.new.iter().map(|(r, _)| r.name.as_str()).collect();
    assert_eq!(new, ["fresh_large", "fresh_small"]);
    let removed: Vec<&str> = delta.removed.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(removed, ["gone"]);
    assert_eq!(delta.largest[0].name, "big");
    assert_eq!(delta.largest[1].name, "small");
    assert_eq!((delta.nodes_before, delta.nodes_after), (1054, 1323));
}

/// AC2 (D6): a matched row is flagged only when BOTH thresholds hold; a new
/// row at `min_delta`, the default one when none was given.
#[test]
fn the_flag_rule_needs_both_thresholds() {
    let before = [
        row("m", "both", false, nodes(10)),
        row("m", "ratio_only", false, nodes(2)),
        row("m", "delta_only", false, nodes(1000)),
        row("m", "exact", false, nodes(20)),
        row("m", "shrank", false, nodes(100)),
    ];
    let after = [
        row("m", "both", false, nodes(30)),
        row("m", "ratio_only", false, nodes(9)),
        row("m", "delta_only", false, nodes(1100)),
        row("m", "exact", false, nodes(30)),
        row("m", "shrank", false, nodes(1)),
        row("m", "new_at_floor", false, nodes(10)),
        row("m", "new_below", false, nodes(9)),
    ];
    let delta = delta_rows(&baseline(&before), &after, LOW);
    let flagged: Vec<&str> = delta
        .matched
        .iter()
        .filter(|r| r.flagged)
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(flagged, ["both", "exact"]);
    let new_flags: Vec<(&str, bool)> = delta
        .new
        .iter()
        .map(|(r, f)| (r.name.as_str(), *f))
        .collect();
    assert_eq!(new_flags, [("new_at_floor", true), ("new_below", false)]);
    assert_eq!(delta.flagged(), 3);

    let defaulted = delta_rows(
        &baseline(&[]),
        &[
            row("m", "born_large", false, nodes(2000)),
            row("m", "n", false, nodes(1999)),
        ],
        Thresholds::DEFAULT,
    );
    let flags: Vec<bool> = defaulted.new.iter().map(|(_, f)| *f).collect();
    assert_eq!(flags, [true, false]);
}

/// AC3 (D6): 0 / 2 without a baseline, never 1; 0 / 1 / 2 with one.
#[test]
fn the_exit_mapping_follows_d6() {
    assert_eq!(exit_code(false, false, 0), 0);
    assert_eq!(exit_code(false, false, 5), 0);
    assert_eq!(exit_code(false, true, 0), 2);
    assert_eq!(exit_code(true, false, 0), 0);
    assert_eq!(exit_code(true, false, 1), 1);
    assert_eq!(exit_code(true, true, 1), 2);
}

/// AC3 (D6): through the shell — a flagged row exits 1, a quiet one 0, and
/// `--json` on a refusal is still one `{"error": …}` object.
#[test]
fn term_size_output_maps_the_delta_to_its_exit() {
    let before = vec![row("m", "a", false, nodes(10))];
    let text = census_json("all", &before);
    let asked = TermSizeRequest {
        baseline: Some(PathBuf::from("b.json")),
        thresholds: LOW,
        ..request(Population::All, false)
    };
    let grown = vec![row("m", "a", false, nodes(40))];
    let (out, _, code) = term_size_output("f.tg", &asked, Ok(grown), Some(Ok(text.clone())));
    assert_eq!(code, 1);
    assert!(
        out.starts_with("core size: f.tg  vs  b.json   (population: all)\n"),
        "{out}"
    );
    assert!(
        out.contains("FLAGGED") && out.contains("largest now:\n"),
        "{out}"
    );
    assert!(
        out.contains("nodes 10 → 40; 1 flagged (max-growth 1.5, min-delta 10)"),
        "{out}"
    );
    let (_, _, code) = term_size_output("f.tg", &asked, Ok(before.clone()), Some(Ok(text)));
    assert_eq!(code, 0);

    let as_json = TermSizeRequest {
        json: true,
        ..asked
    };
    let (out, _, code) = term_size_output("f.tg", &as_json, Ok(before), Some(Ok("garbage".into())));
    assert_eq!(code, 2);
    let parsed: Value = serde_json::from_str(&out).expect("one object");
    assert!(parsed["error"]
        .as_str()
        .is_some_and(|e| e.contains("b.json")));
}

/// D6: the delta's JSON carries every section, thresholds and totals.
#[test]
fn the_delta_json_names_every_section() {
    let before = [
        row("m", "a", false, nodes(10)),
        row("m", "gone", false, nodes(1)),
    ];
    let after = [
        row("m", "a", false, nodes(40)),
        row("m", "b", false, nodes(12)),
    ];
    let delta = delta_rows(&baseline(&before), &after, LOW);
    let header = render::DeltaHeader {
        file: "f.tg",
        baseline: "b.json",
        population: Population::All,
        thresholds: LOW,
        top: 1,
    };
    let parsed: Value =
        serde_json::from_str(&render::render_delta_json(&header, &delta, &after)).expect("json");
    assert_eq!(
        parsed["thresholds"],
        json!({"max_growth": 1.5, "min_delta": 10})
    );
    assert_eq!(
        parsed["matched"][0],
        json!({"module": "m", "name": "a", "before": 10, "after": 40, "delta": 30,
               "ratio": 4.0, "case_arms_before": 0, "case_arms_after": 0, "flagged": true})
    );
    assert_eq!(parsed["new"][0]["flagged"], json!(true));
    assert_eq!(parsed["removed"][0]["name"], "gone");
    assert_eq!(parsed["largest"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        parsed["totals"],
        json!({"nodes_before": 11, "nodes_after": 52, "matched": 1, "new": 1,
               "removed": 1, "flagged": 2})
    );
}

/// D6: a flagged row past `--top` is still printed; an unflagged one is not.
#[test]
fn the_text_form_shows_flagged_rows_past_top() {
    let before = [
        row("m", "a_big", false, nodes(100)),
        row("m", "b_quiet", false, nodes(100)),
        row("m", "c_flag", false, nodes(10)),
    ];
    let after = [
        row("m", "a_big", false, nodes(300)),
        row("m", "b_quiet", false, nodes(130)),
        row("m", "c_flag", false, nodes(25)),
    ];
    let delta = delta_rows(&baseline(&before), &after, LOW);
    let header = render::DeltaHeader {
        file: "f.tg",
        baseline: "b.json",
        population: Population::All,
        thresholds: LOW,
        top: 1,
    };
    let text = render::render_delta_text(&header, &delta);
    let (matched, largest) = text.split_once("largest now:").expect("section");
    assert!(
        matched.contains("a_big") && matched.contains("c_flag"),
        "{text}"
    );
    assert!(!matched.contains("b_quiet"), "{text}");
    assert!(
        largest.contains("a_big") && !largest.contains("c_flag"),
        "{text}"
    );
}

/// D6: a matched row prints both node counts and both case-arm widths, and a
/// `new` row past `--top` is printed only when it is flagged.
#[test]
fn the_text_form_prints_counts_and_filters_new_rows() {
    let with_arms = |n, case_arms| {
        Some(ProofTermSize {
            case_arms,
            ..size(0, 0, 0, n)
        })
    };
    let before = [row("m", "grown", false, with_arms(10, 2))];
    let after = [
        row("m", "grown", false, with_arms(40, 6)),
        row("m", "new_big", false, nodes(30)),
        row("m", "new_mid", false, nodes(5)),
        row("m", "new_low", false, nodes(4)),
    ];
    let delta = delta_rows(&baseline(&before), &after, LOW);
    let header = render::DeltaHeader {
        file: "f.tg",
        baseline: "b.json",
        population: Population::All,
        thresholds: LOW,
        top: 2,
    };
    let text = render::render_delta_text(&header, &delta);
    let grown = format!(
        "  {:<32} {:<32} {:>8} {:>8} {:>8} {:>7}  2 → 6  FLAGGED\n",
        "grown", "m", 10, 40, "+30", "×4.00"
    );
    assert!(text.contains(&grown), "{text}");
    let (_, new_rows) = text.split_once("removed: 0\n").expect("new line");
    let (new_rows, _) = new_rows.split_once("largest now:").expect("section");
    assert!(
        new_rows.contains("new_big") && new_rows.contains("FLAGGED"),
        "{text}"
    );
    assert!(new_rows.contains("new_mid"), "{text}");
    assert!(!new_rows.contains("new_low"), "{text}");
}
