//! `albite doctor check ir small-adts` — a census of where small sums still
//! cost in emitted LLVM IR (ADR 28.9.26f).
//!
//! B19 promised to "unbox small ADTs", but a non-recursive sum is already a
//! by-value `{ i32, [N x i8] }` blob and never allocated on its own. What small
//! sums do cost is visible in the IR: an empty constructor of a recursive type
//! is a full heap cell, a ≤ 8-byte sum crosses a `musttail` entry through an
//! `sret`/indirect buffer, and an `Option` of a pointer spends a tag it could
//! niche. This counts those shapes; [`decide`] turns the counts into B19's
//! scope.
//!
//! A **census, not a gate**: it shares the IR-audit corpus contract (walker,
//! reach pair, `--strict` vacuity) but never exits 1, and it is deliberately
//! not part of `make check-ir-audits`.

use std::path::Path;
use std::process::ExitCode;

use serde::Serialize;

use super::corpus::{reject_non_directory, scan_ll_corpus, AuditVerdict, ReachCounts};

pub mod decide;
pub mod scan;

use decide::{decide, Verdict};
use scan::{blob_bytes, count_fallbacks, scan_module, SmallAdtCounts};

// Tests are grouped under `tests/` (three surfaces: scanner, rule, output; the
// scanner's nullary-fold edge cases in a file of their own) and
// `#[path]`-mounted so they stay direct children with access to private items.
#[cfg(test)]
#[path = "tests/decide_tests.rs"]
mod decide_tests;
#[cfg(test)]
#[path = "tests/fold_tests.rs"]
mod fold_tests;
#[cfg(test)]
#[path = "tests/render_tests.rs"]
mod render_tests;
#[cfg(test)]
#[path = "tests/scan_tests.rs"]
mod scan_tests;

/// A censused corpus: the counts, how far the scanner reached, and the rule's
/// verdict over them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusReport {
    pub files: usize,
    pub reach: ReachCounts,
    pub counts: SmallAdtCounts,
    pub verdict: Verdict,
    /// The corpus's `note: skipped` line, if any. The census skips the native
    /// core's optimised unit like every audit: its post-O2 ADT shapes are not
    /// the emitter's, so counting them would skew the census (ADR 28.10.26e).
    pub skip_note: Vec<String>,
}

/// Census a directory of `.ll` files. `emit_log`, when given, is the stderr of
/// the emit that produced the corpus and supplies `pointer_fallbacks`.
pub fn cmd_census_small_adts(
    dir: &Path,
    json: bool,
    strict: bool,
    emit_log: Option<&Path>,
) -> ExitCode {
    let (verdict, report) = census_directory(dir, strict, emit_log);
    if let Some(report) = &report {
        if json {
            println!("{}", render_json(report));
        } else {
            for line in render_text(report) {
                println!("{line}");
            }
        }
        // After the summary line; to stderr in JSON mode, like the drift lines.
        for line in &report.skip_note {
            if json {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
    }
    // Drift and bad-input lines go to stderr in JSON mode, so stdout stays one
    // parseable object for the make target's redirect.
    let vacuous = report.as_ref().is_some_and(|r| r.reach.is_vacuous());
    for line in verdict.drift_report(
        vacuous,
        "blob or allocation candidates seen, nothing counted",
    ) {
        if json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    verdict.exit()
}

/// Scan, count and decide, returning the exit verdict and — when the corpus
/// was read at all — the report. The testable half of the command.
///
/// Never [`AuditVerdict::Violations`]: a census has no findings.
pub(crate) fn census_directory(
    dir: &Path,
    strict: bool,
    emit_log: Option<&Path>,
) -> (AuditVerdict, Option<CensusReport>) {
    if let Some(rejected) = reject_non_directory(dir) {
        return (rejected, None);
    }
    let pointer_fallbacks = match emit_log.map(std::fs::read_to_string) {
        None => None,
        Some(Ok(log)) => Some(count_fallbacks(&log)),
        Some(Err(err)) => {
            // Bad input, exit 2 — the corpus contract has no finer variant,
            // and one would change nothing the shell sees.
            eprintln!("error: cannot read --emit-log: {err}");
            return (AuditVerdict::NotADirectory, None);
        }
    };
    let mut counts = SmallAdtCounts::default();
    let mut reach = ReachCounts::default();
    let scan = scan_ll_corpus(dir, |_, text| {
        let module = scan_module(text);
        if is_candidate(text) {
            reach.note_candidate();
        }
        if module.counted_anything() {
            reach.note_tracked();
        }
        counts.add(&module);
    });
    scan.charge_unreadable(&mut reach);
    counts.pointer_fallbacks = pointer_fallbacks;
    let verdict = AuditVerdict::classify(&scan, reach.is_vacuous(), 0, strict);
    if verdict == AuditVerdict::EmptyCorpus {
        return (verdict, None);
    }
    let report = CensusReport {
        files: scan.files,
        reach,
        verdict: decide(&counts),
        counts,
        skip_note: scan.skip_report(),
    };
    (verdict, Some(report))
}

/// A module the scanner is expected to count something in.
fn is_candidate(text: &str) -> bool {
    text.contains("{ i32, [") || text.contains("@__albite_alloc(")
}

#[derive(Serialize)]
struct JsonReport {
    schema_version: u32,
    corpus: JsonCorpus,
    reach: JsonReach,
    blob_shapes: Vec<JsonShape>,
    categories: JsonCategories,
    verdict: JsonVerdict,
}

#[derive(Serialize)]
struct JsonCorpus {
    files: usize,
    defines: u64,
}

#[derive(Serialize)]
struct JsonReach {
    candidates: usize,
    tracked: usize,
}

#[derive(Serialize)]
struct JsonShape {
    payload_bytes: u64,
    blob_bytes: u64,
    by_value_params: u64,
    by_value_returns: u64,
    sret_returns: u64,
    indirect_params: u64,
    constructions: u64,
}

#[derive(Serialize)]
struct JsonCategories {
    blob_buffer_sites: u64,
    small_blob_buffer_sites: u64,
    small16_blob_buffer_sites: u64,
    small_blob_by_value_sites: u64,
    blob_constructions: u64,
    pointer_payload_constructions: u64,
    mu_alloc_sites: u64,
    mu_stack_sites: u64,
    nullary_mu_alloc_sites: u64,
    construction_allocas: u64,
    pointer_fallbacks: Option<u64>,
}

#[derive(Serialize)]
struct JsonVerdict {
    nullary_cells: &'static str,
    small_sum_registers: &'static str,
    pointer_niche: &'static str,
    fallbacks: &'static str,
    b19: &'static str,
}

/// The stable `schema_version: 1` form (§2.1). Struct field order is the key
/// order, and `shapes` is a `BTreeMap`, so `blob_shapes` is sorted by size.
pub(crate) fn render_json(report: &CensusReport) -> String {
    let counts = &report.counts;
    let verdict = &report.verdict;
    let json = JsonReport {
        schema_version: 1,
        corpus: JsonCorpus {
            files: report.files,
            defines: counts.defines,
        },
        reach: JsonReach {
            candidates: report.reach.candidates,
            tracked: report.reach.tracked,
        },
        blob_shapes: counts
            .shapes
            .iter()
            .map(|(payload_bytes, shape)| JsonShape {
                payload_bytes: *payload_bytes,
                blob_bytes: blob_bytes(*payload_bytes),
                by_value_params: shape.by_value_params,
                by_value_returns: shape.by_value_returns,
                sret_returns: shape.sret_returns,
                indirect_params: shape.indirect_params,
                constructions: shape.constructions,
            })
            .collect(),
        categories: JsonCategories {
            blob_buffer_sites: counts.blob_buffer_sites(),
            small_blob_buffer_sites: counts.small_blob_buffer_sites(),
            small16_blob_buffer_sites: counts.small16_blob_buffer_sites(),
            small_blob_by_value_sites: counts.small_blob_by_value_sites(),
            blob_constructions: counts.blob_constructions(),
            pointer_payload_constructions: counts.pointer_payload_constructions,
            mu_alloc_sites: counts.mu_alloc_sites,
            mu_stack_sites: counts.mu_stack_sites,
            nullary_mu_alloc_sites: counts.nullary_mu_alloc_sites,
            construction_allocas: counts.construction_allocas,
            pointer_fallbacks: counts.pointer_fallbacks,
        },
        verdict: JsonVerdict {
            nullary_cells: verdict.nullary_cells.as_str(),
            small_sum_registers: verdict.small_sum_registers.as_str(),
            pointer_niche: verdict.pointer_niche.as_str(),
            fallbacks: verdict.fallbacks.as_str(),
            b19: verdict.b19.as_str(),
        },
    };
    serde_json::to_string_pretty(&json).unwrap_or_default()
}

/// The human form: one line per category, the per-`N` table, the verdict
/// block and the shared `summary:` line.
pub(crate) fn render_text(report: &CensusReport) -> Vec<String> {
    let counts = &report.counts;
    let fallbacks = counts
        .pointer_fallbacks
        .map_or_else(|| "null (no --emit-log)".to_string(), |n| n.to_string());
    let mut lines = vec![
        "small-ADT census (ADR 28.9.26f)".to_string(),
        format!("  blob_buffer_sites             {}", counts.blob_buffer_sites()),
        format!("  small_blob_buffer_sites       {}", counts.small_blob_buffer_sites()),
        format!("  small16_blob_buffer_sites     {}", counts.small16_blob_buffer_sites()),
        format!("  small_blob_by_value_sites     {}", counts.small_blob_by_value_sites()),
        format!("  blob_constructions            {}", counts.blob_constructions()),
        format!("  pointer_payload_constructions {}", counts.pointer_payload_constructions),
        format!("  mu_alloc_sites                {}", counts.mu_alloc_sites),
        format!("  mu_stack_sites                {}", counts.mu_stack_sites),
        format!("  nullary_mu_alloc_sites        {}", counts.nullary_mu_alloc_sites),
        format!("  construction_allocas          {}", counts.construction_allocas),
        format!("  pointer_fallbacks             {fallbacks}"),
        String::new(),
        "  payload  blob  by_value_params  by_value_returns  sret_returns  indirect_params  constructions"
            .to_string(),
    ];
    for (payload_bytes, shape) in &counts.shapes {
        lines.push(format!(
            "  {:>7}  {:>4}  {:>15}  {:>16}  {:>12}  {:>15}  {:>13}",
            payload_bytes,
            blob_bytes(*payload_bytes),
            shape.by_value_params,
            shape.by_value_returns,
            shape.sret_returns,
            shape.indirect_params,
            shape.constructions
        ));
    }
    let verdict = &report.verdict;
    lines.extend([
        String::new(),
        "verdict:".to_string(),
        format!("  nullary_cells        {}", verdict.nullary_cells.as_str()),
        format!(
            "  small_sum_registers  {}",
            verdict.small_sum_registers.as_str()
        ),
        format!("  pointer_niche        {}", verdict.pointer_niche.as_str()),
        format!("  fallbacks            {}", verdict.fallbacks.as_str()),
        format!("  b19                  {}", verdict.b19.as_str()),
        format!(
            "summary: files={} defines={} candidates={} tracked={}",
            report.files, counts.defines, report.reach.candidates, report.reach.tracked
        ),
    ]);
    lines
}
