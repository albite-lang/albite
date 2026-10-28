//! 28.9.26f AC4 — the output contract is stable: the JSON's keys and order,
//! and the exit-code map (never 1).

use std::path::Path;

use super::*;
use crate::doctor::checks::ir_checks::corpus::fixtures::corpus_at;
use crate::doctor::checks::ir_checks::corpus::AuditVerdict;

const COUNTED: &str = "define void @f({ i32, [0 x i8] } %0) {
entry:
  %mu_alloc = call ptr @__albite_alloc(i64 20, i32 0)
  ret void
}
";

/// Every key of a JSON object, in document order, flattened with `/` paths.
fn key_paths(json: &serde_json::Value, prefix: &str, paths: &mut Vec<String>) {
    if let serde_json::Value::Object(map) = json {
        for (key, child) in map {
            let path = format!("{prefix}{key}");
            paths.push(path.clone());
            key_paths(child, &format!("{path}/"), paths);
        }
    }
}

/// Pull the object keys out of the rendered text in the order they appear —
/// `serde_json::Value` re-sorts keys without `preserve_order`, so order is
/// asserted on the raw string.
fn keys_in_text_order(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix('"')?;
            let (key, after) = rest.split_once('"')?;
            after.starts_with(':').then(|| key.to_string())
        })
        .collect()
}

/// 28.9.26f AC4 — the §2.1 keys, in the §2.1 order, `schema_version` 1.
#[test]
fn the_json_has_exactly_the_schema_keys_in_order() {
    let dir = corpus_at(&[("m.ll", COUNTED)]);
    let (verdict, report) = census_directory(dir.path(), true, None);
    assert_eq!(verdict, AuditVerdict::Clean);
    let rendered = render_json(&report.expect("a report"));
    let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    let mut paths = Vec::new();
    key_paths(&parsed, "", &mut paths);
    assert!(paths.contains(&"categories/pointer_fallbacks".to_string()));
    assert_eq!(
        keys_in_text_order(&rendered),
        [
            "schema_version",
            "corpus",
            "files",
            "defines",
            "reach",
            "candidates",
            "tracked",
            "blob_shapes",
            "payload_bytes",
            "blob_bytes",
            "by_value_params",
            "by_value_returns",
            "sret_returns",
            "indirect_params",
            "constructions",
            "categories",
            "blob_buffer_sites",
            "small_blob_buffer_sites",
            "small16_blob_buffer_sites",
            "small_blob_by_value_sites",
            "blob_constructions",
            "pointer_payload_constructions",
            "mu_alloc_sites",
            "mu_stack_sites",
            "nullary_mu_alloc_sites",
            "construction_allocas",
            "pointer_fallbacks",
            "verdict",
            "nullary_cells",
            "small_sum_registers",
            "pointer_niche",
            "fallbacks",
            "b19",
        ]
    );
    assert!(parsed["categories"]["pointer_fallbacks"].is_null());
    assert_eq!(parsed["verdict"]["fallbacks"], "unknown");
    assert_eq!(parsed["categories"]["mu_alloc_sites"], 1);
    assert_eq!(parsed["blob_shapes"][0]["blob_bytes"], 4);
    assert_eq!(parsed["corpus"]["defines"], 1);
    assert_eq!(parsed["reach"]["tracked"], 1);
}

#[test]
fn blob_shapes_are_sorted_by_payload_bytes() {
    let module =
        "define void @f({ i32, [16 x i8] } %0, { i32, [1 x i8] } %1, { i32, [8 x i8] } %2) {
entry:
  ret void
}
";
    let dir = corpus_at(&[("m.ll", module)]);
    let (_, report) = census_directory(dir.path(), false, None);
    let parsed: serde_json::Value =
        serde_json::from_str(&render_json(&report.expect("a report"))).expect("JSON");
    let sizes: Vec<u64> = parsed["blob_shapes"]
        .as_array()
        .expect("array")
        .iter()
        .map(|shape| shape["payload_bytes"].as_u64().expect("number"))
        .collect();
    assert_eq!(sizes, [1, 8, 16]);
}

#[test]
fn a_non_directory_and_an_empty_directory_are_bad_input() {
    let (verdict, report) = census_directory(Path::new("/no/such/dir"), false, None);
    assert_eq!(verdict, AuditVerdict::NotADirectory);
    assert!(report.is_none());
    let empty = tempfile::tempdir().expect("tempdir");
    let (verdict, report) = census_directory(empty.path(), false, None);
    assert_eq!(verdict, AuditVerdict::EmptyCorpus);
    assert!(report.is_none());
    assert_eq!(
        format!("{:?}", verdict.exit()),
        format!("{:?}", ExitCode::from(2))
    );
}

#[test]
fn a_candidate_corpus_with_nothing_counted_is_vacuous_only_under_strict() {
    // Mentions a blob, but in no position any category reads.
    let drifted = "define void @f(ptr %0) {
entry:
  %x = load { i32, [4 x i8] }, ptr %0, align 4
  ret void
}
";
    let dir = corpus_at(&[("m.ll", drifted)]);
    let (plain, report) = census_directory(dir.path(), false, None);
    assert_eq!(plain, AuditVerdict::Clean);
    let report = report.expect("a report");
    assert_eq!(report.reach.candidates, 1);
    assert_eq!(report.reach.tracked, 0);
    let (strict, _) = census_directory(dir.path(), true, None);
    assert_eq!(strict, AuditVerdict::VacuousUnderStrict);
    assert_eq!(
        format!("{:?}", strict.exit()),
        format!("{:?}", ExitCode::from(2))
    );
}

#[test]
fn a_counted_corpus_exits_zero_never_one() {
    let dir = corpus_at(&[("m.ll", COUNTED), ("n.ll", "; not a candidate\n")]);
    let (verdict, report) = census_directory(dir.path(), true, None);
    assert_eq!(verdict, AuditVerdict::Clean);
    assert_eq!(
        format!("{:?}", verdict.exit()),
        format!("{:?}", ExitCode::SUCCESS)
    );
    let report = report.expect("a report");
    assert_eq!(report.files, 2);
    assert_eq!(report.reach.candidates, 1);
    assert!(
        report.skip_note.is_empty(),
        "nothing skipped, nothing noted"
    );
}

// ADR 28.10.26e D2 — the census skips the core's optimised unit too, and its
// report carries the note the command prints after the summary.
#[test]
fn the_census_skips_the_native_core_unit_and_carries_the_note() {
    let dir = corpus_at(&[("m.ll", COUNTED), ("__native_core/native_core.ll", COUNTED)]);
    let (_, report) = census_directory(dir.path(), false, None);
    let report = report.expect("a report");
    assert_eq!(report.files, 1, "the core's file is not censused");
    assert_eq!(report.reach.candidates, 1);
    assert_eq!(report.skip_note.len(), 1, "{:?}", report.skip_note);
    assert!(
        report.skip_note[0].contains("__native_core/native_core.ll"),
        "{:?}",
        report.skip_note
    );
}

#[test]
fn the_emit_log_supplies_pointer_fallbacks_and_an_unreadable_one_is_bad_input() {
    let dir = corpus_at(&[
        ("ll/m.ll", COUNTED),
        ("emit.log", "warning: lower_type recursion depth exceeded\n"),
    ]);
    let log = dir.path().join("emit.log");
    let (_, report) = census_directory(&dir.path().join("ll"), false, Some(&log));
    let report = report.expect("a report");
    assert_eq!(report.counts.pointer_fallbacks, Some(1));
    assert_eq!(report.verdict.fallbacks.as_str(), "route-to-correctness");
    let (verdict, report) =
        census_directory(&dir.path().join("ll"), false, Some(Path::new("/no/log")));
    assert_eq!(
        format!("{:?}", verdict.exit()),
        format!("{:?}", ExitCode::from(2))
    );
    assert!(report.is_none());
}

#[test]
fn the_text_form_carries_categories_table_verdict_and_summary() {
    let dir = corpus_at(&[("m.ll", COUNTED)]);
    let (_, report) = census_directory(dir.path(), false, None);
    let lines = render_text(&report.expect("a report"));
    assert!(lines
        .iter()
        .any(|l| l.trim() == "mu_alloc_sites                1"));
    assert!(lines
        .iter()
        .any(|l| l.trim() == "pointer_fallbacks             null (no --emit-log)"));
    assert!(lines
        .iter()
        .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["0", "4", "1", "0", "0", "0", "0"]));
    assert!(lines
        .iter()
        .any(|l| l.trim() == "b19                  close"));
    assert_eq!(
        lines.last().map(String::as_str),
        Some("summary: files=1 defines=1 candidates=1 tracked=1")
    );
}

/// The command wrapper returns the census's exit code, not a default one: a
/// bad path is exit 2 through the CLI entry point too.
#[test]
fn the_command_exit_code_is_the_verdicts() {
    let bad = cmd_census_small_adts(Path::new("/no/such/dir"), true, false, None);
    assert_eq!(format!("{bad:?}"), format!("{:?}", ExitCode::from(2)));
    let dir = corpus_at(&[("m.ll", COUNTED)]);
    let ok = cmd_census_small_adts(dir.path(), false, true, None);
    assert_eq!(format!("{ok:?}"), format!("{:?}", ExitCode::SUCCESS));
}

#[test]
fn is_candidate_needs_a_blob_or_an_allocator_call() {
    assert!(is_candidate("x { i32, [0 x i8] }"));
    assert!(is_candidate("call ptr @__albite_alloc(i64 4, i32 1)"));
    assert!(!is_candidate("define i64 @f() {"));
}
