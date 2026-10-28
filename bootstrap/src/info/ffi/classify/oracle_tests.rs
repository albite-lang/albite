//! The oracle: `--classify` against 1.10.26k's record (ADR 5.10.26f D3, AC 3).
//!
//! Every symbol in the record's §§ 4–5 must resolve to one current item or a
//! `cfg` pair. Its class must match the record's, and so must its basis where
//! the record's basis names a call or state the facts also hold — elsewhere
//! the class alone is compared. A disagreement fails unless the expectation
//! table lists it; a listed row that now agrees fails too.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::expectations::{Expectation, EXPECTED_DISAGREEMENTS, PINNED_DISAGREEMENTS};
use super::extract::{CrateIndex, SourceFile};
use super::reach::Reach;
use super::{classify_export, Class, Facts};
use crate::info::ffi::{classify_scanned, parse_roots, ScannedRoot};
use albite_bootstrap::doctor::checks::check_extern_symbols::scan_exports;

const RECORD: &str = "benchmarks/results/1.10.26k.core-export-inventory.md";

/// One row of the record's §§ 4–5.
struct RecordRow {
    symbol: String,
    class: Class,
    basis: String,
}

/// The rows of §§ 4–5: every table line under those headings.
fn record_rows(text: &str) -> Vec<RecordRow> {
    let mut rows = Vec::new();
    let mut in_tables = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            in_tables = line.starts_with("## 4.") || line.starts_with("## 5.");
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if !in_tables || cells.len() < 6 || !cells[1].starts_with('`') {
            continue;
        }
        let class = Class::from_label(cells[3]).unwrap_or_else(|| panic!("class in {line}"));
        rows.push(RecordRow {
            symbol: cells[1].trim_matches('`').to_string(),
            class,
            basis: cells[4].to_string(),
        });
    }
    rows
}

/// The names a basis spells in backticks, by their last path segment:
/// `` `fs::read` `` → `read`, `` `eval_with_env*` `` → `eval_with_env*`.
fn basis_names(basis: &str) -> Vec<String> {
    basis
        .split('`')
        .skip(1)
        .step_by(2)
        .flat_map(|quoted| quoted.split(['/', ',', ' ', '+']))
        .filter_map(|name| name.rsplit("::").next())
        .map(|name| name.trim_matches(|c: char| c == '.' || c == '(' || c == ')' || c == '!'))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn names_match(record_name: &str, fact_name: &str) -> bool {
    match record_name.strip_suffix('*') {
        Some(prefix) => fact_name.starts_with(prefix),
        None => record_name == fact_name,
    }
}

/// The last segment of each fact's token and of the helper it went through.
fn fact_names<'a>(facts: impl Iterator<Item = &'a super::Fact>) -> Vec<String> {
    facts
        .flat_map(|fact| {
            let token = fact.token.rsplit("::").next().unwrap_or("");
            let token = token.trim_matches(|c: char| c == '.' || c == '(' || c == ')' || c == '!');
            std::iter::once(token.to_string()).chain(fact.via.clone())
        })
        .collect()
}

fn overlaps(record: &[String], facts: &[String]) -> bool {
    record
        .iter()
        .any(|name| facts.iter().any(|fact| names_match(name, fact)))
}

/// `None` when the row agrees; else the rule's class (equal to the record's
/// when only the basis differs).
fn disagreement(row: &RecordRow, facts: &Facts) -> Option<Class> {
    let verdict = classify_export(facts);
    if verdict.class != row.class {
        return Some(verdict.class);
    }
    let record = basis_names(&row.basis);
    let all = facts
        .kernel
        .iter()
        .chain(&facts.runtime)
        .chain(&facts.unreadable);
    let comparable = overlaps(&record, &fact_names(all));
    let agrees = overlaps(&record, &fact_names(verdict.deciding.iter()));
    (comparable && !agrees).then_some(verdict.class)
}

/// Each disagreeing row: the record's class, the rule's (`None` when the
/// symbol resolves to no item, or to an ambiguous set), and why.
type Disagreements = BTreeMap<String, (Class, Option<Class>, String)>;

fn disagreements(rows: &[RecordRow], reach: &Reach<'_>) -> Disagreements {
    let mut found = BTreeMap::new();
    for row in rows {
        let outcome = match reach.export_facts(&row.symbol) {
            Ok(facts) => disagreement(row, &facts)
                .map(|class| (Some(class), format!("{facts:?} vs record `{}`", row.basis))),
            Err(error) => Some((None, error)),
        };
        if let Some((rule, why)) = outcome {
            found.insert(row.symbol.clone(), (row.class, rule, why));
        }
    }
    found
}

/// Every way the table and the disagreements fail to match: a listed row
/// that now agrees or has moved, and a disagreement nobody listed.
fn review(expected: &[Expectation], mut actual: Disagreements) -> Vec<String> {
    let mut failures = Vec::new();
    for listed in expected {
        match actual.remove(listed.symbol) {
            Some((record, rule, _)) if (record, rule) == (listed.record, listed.rule) => {}
            Some((record, rule, why)) => failures.push(format!(
                "{}: listed as {:?}→{:?}, now {record:?}→{rule:?} — {why}",
                listed.symbol, listed.record, listed.rule
            )),
            None => failures.push(format!(
                "{}: listed, but now agrees — remove the row (it said: {})",
                listed.symbol, listed.cause
            )),
        }
    }
    for (symbol, (record, rule, why)) in &actual {
        let rule = rule.map_or_else(
            || "no current item, or an ambiguous set".to_string(),
            |rule| format!("rule {rule}"),
        );
        failures.push(format!(
            "{symbol}: record {record}, {rule} — not listed — {why}"
        ));
    }
    failures
}

fn workspace_roots() -> Vec<PathBuf> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    vec![
        workspace.join("albite_core/src"),
        workspace.join("albite_runtime/src"),
    ]
}

/// The record's text, or `None` where its directory is absent. Self-skip as
/// ADR 31.7.26c's precedent does: `mutant-schemata` copies the workspace with
/// `benchmarks/` in `SKIPPED_AT_ROOT`, so a hard read failed the schema
/// BASELINE there and aborted the crate's whole sweep. Guarding on the
/// DIRECTORY, not the file, keeps the read sharp in-tree: a renamed or
/// deleted record still fails here.
fn record_text() -> Option<String> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    if !workspace.join("benchmarks/results").is_dir() {
        return None;
    }
    Some(std::fs::read_to_string(workspace.join(RECORD)).expect("the record"))
}

// 5.10.26f AC3: every recorded symbol resolves; each class (and comparable
// basis) matches the record or is listed; a listed row that agrees fails; the
// table's length is pinned.
#[test]
fn the_record_is_the_oracle_and_every_disagreement_is_reviewed() {
    let Some(text) = record_text() else { return };
    let rows = record_rows(&text);
    assert_eq!(rows.len(), 222, "§ 4 holds 210 rows and § 5 holds 12");
    let parsed = parse_roots(&workspace_roots()).expect("the crates parse");
    let crates: Vec<(&str, &[SourceFile])> = parsed
        .iter()
        .map(|(name, files)| (name.as_str(), files.as_slice()))
        .collect();
    let index = CrateIndex::build(&crates);
    let reach = Reach::new(&index);
    let failures = review(&EXPECTED_DISAGREEMENTS, disagreements(&rows, &reach));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(EXPECTED_DISAGREEMENTS.len(), PINNED_DISAGREEMENTS);
}

// 5.10.26f AC3: exports added since the record are reported, not compared —
// and every current export resolves to one item or a `cfg` pair.
#[test]
fn exports_added_since_the_record_are_reported_not_compared() {
    let Some(text) = record_text() else { return };
    let recorded: Vec<String> = record_rows(&text)
        .into_iter()
        .map(|row| row.symbol)
        .collect();
    let roots = workspace_roots();
    let scanned: Vec<ScannedRoot> = roots
        .iter()
        .map(|root| ScannedRoot {
            root: root.display().to_string(),
            exports: scan_exports(root).expect("scan"),
        })
        .collect();
    let parsed = parse_roots(&roots).expect("the crates parse");
    let (verdicts, errors) = classify_scanned(&scanned, &parsed);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    let unrecorded: Vec<&String> = verdicts
        .keys()
        .filter(|symbol| !recorded.contains(symbol))
        .collect();
    eprintln!(
        "unrecorded since 1.10.26k ({}): {unrecorded:?}",
        unrecorded.len()
    );
    assert!(!verdicts.is_empty());
}

// 5.10.26f AC3: the review over injected tables — a listed row that now
// agrees fails, a moved one fails, an unlisted one fails, a matching one
// passes, and a symbol with no item is reported as such.
#[test]
fn a_listed_row_that_now_agrees_fails_and_so_does_an_unlisted_one() {
    let listed = |symbol, record, rule| Expectation {
        symbol,
        record,
        rule,
        cause: "injected",
    };
    let table = [
        listed("tg_held", Class::Pure, Some(Class::Runtime)),
        listed("tg_fixed", Class::Pure, Some(Class::Runtime)),
        listed("tg_moved", Class::Pure, Some(Class::Runtime)),
    ];
    let mut actual = Disagreements::new();
    let entry = |rule| (Class::Pure, rule, "why".to_string());
    actual.insert("tg_held".to_string(), entry(Some(Class::Runtime)));
    actual.insert("tg_moved".to_string(), entry(Some(Class::Kernel)));
    actual.insert("tg_new".to_string(), entry(Some(Class::Unknown)));
    actual.insert("tg_gone".to_string(), entry(None));
    let failures = review(&table, actual);
    assert_eq!(failures.len(), 4, "{failures:#?}");
    assert!(failures[0].starts_with("tg_fixed: listed, but now agrees"));
    assert!(
        failures[1].starts_with("tg_moved: listed as Pure→Some(Runtime), now Pure→Some(Kernel)")
    );
    assert!(failures[2].starts_with("tg_gone: record pure, no current item"));
    assert!(failures[3].starts_with("tg_new: record pure, rule unknown — not listed"));
    assert!(review(
        &table[..1],
        BTreeMap::from([("tg_held".to_string(), entry(Some(Class::Runtime)))])
    )
    .is_empty());
}

// The oracle's own comparison, over injected rows: a basis is compared only
// where the record names a fact the extraction also holds.
#[test]
fn a_basis_is_compared_only_where_the_record_names_a_held_fact() {
    use super::Fact;
    let row = |class, basis: &str| RecordRow {
        symbol: "tg_probe".to_string(),
        class,
        basis: basis.to_string(),
    };
    let runtime = |token: &str| Facts {
        runtime: vec![Fact::direct(token)],
        ..Facts::default()
    };
    assert_eq!(
        disagreement(&row(Class::Runtime, "`fs::read`"), &runtime("fs::read")),
        None
    );
    assert_eq!(
        disagreement(
            &row(Class::Runtime, "spawns a process"),
            &runtime("fs::read")
        ),
        None
    );
    assert_eq!(
        disagreement(&row(Class::Pure, "`fs::read`"), &runtime("fs::read")),
        Some(Class::Runtime)
    );
    let both = Facts {
        kernel: vec![Fact::direct("with_arena")],
        runtime: vec![Fact::direct("CLI_ARGS")],
        ..Facts::default()
    };
    assert_eq!(
        disagreement(&row(Class::Kernel, "reads `CLI_ARGS`"), &both),
        Some(Class::Kernel)
    );
    assert_eq!(
        basis_names("`eval_with_env*` and `fs::read`/`a::b`"),
        ["eval_with_env*", "read", "b"]
    );
    assert!(names_match("eval_with_env*", "eval_with_env_fuel"));
}
