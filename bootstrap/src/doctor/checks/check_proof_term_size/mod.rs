//! `albite doctor check proof term-size <file>` — definitions beside the
//! size of their Core term, from one cold elaboration (ADR 13.10.26c,
//! widened by 15.10.26g).
//!
//! A census, not a gate (13.10.26c D5): exit 0 when it completes, 2 when it
//! could not ask — the file does not elaborate, or the population is empty.
//! The one exit-1 case is `--baseline`'s (15.10.26g D6): a row grew past both
//! thresholds. The population is [`Population`] (D2), the count is
//! [`count_proof_nodes`] (D3), the comparison is [`delta`]; this file is the
//! shell around them — elaborate, select, compare, map the exit.

mod count;
pub mod delta;
#[cfg(test)]
mod delta_tests;
#[cfg(test)]
mod elaboration_tests;
mod render;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use albite_core::terms::termination::AdmissionState;
use serde_json::json;

use crate::doctor::checks::check_kernel_census::is_axiom;
use crate::driver::{self, ProjectOutput};
use crate::elaborate::termination::{ReportingOnly, TerminationInput};
use crate::elaborate::CoreDef;

pub use count::{count_proof_nodes, ProofTermSize};
pub use delta::Thresholds;
pub use render::{render_delta_json, render_delta_text, render_json, render_text, DeltaHeader};

/// The `--top` used by a delta report when none is given (D6); `diff
/// term-size` shares it (ADR 16.10.26w D1).
pub const DELTA_DEFAULT_TOP: usize = 20;

/// Which definitions are counted (15.10.26g D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Population {
    /// Theorems, lemmas and axioms — 13.10.26c's default.
    Proof,
    /// Those, plus every definition admitted `Total` (`--include-total`).
    ProofTotal,
    /// Every definition, whatever its admission (`--all-definitions`).
    All,
}

impl Population {
    /// The population the two flags select; clap keeps them exclusive.
    #[must_use]
    pub fn from_flags(include_total: bool, all_definitions: bool) -> Population {
        if all_definitions {
            Population::All
        } else if include_total {
            Population::ProofTotal
        } else {
            Population::Proof
        }
    }

    /// The JSON `"population"` value (D2).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Population::Proof => "proof",
            Population::ProofTotal => "proof+total",
            Population::All => "all",
        }
    }

    /// The population a saved report's `"population"` names, if any —
    /// `diff term-size` renders against it (ADR 16.10.26w D3).
    #[must_use]
    pub fn from_label(label: &str) -> Option<Population> {
        [Population::Proof, Population::ProofTotal, Population::All]
            .into_iter()
            .find(|population| population.label() == label)
    }

    fn admits(self, candidate: &Candidate<'_>) -> bool {
        match self {
            Population::Proof => candidate.proof,
            Population::ProofTotal => candidate.proof || candidate.total,
            Population::All => true,
        }
    }
}

/// Everything a `term-size` run was asked, beside the file.
#[derive(Debug, Clone, PartialEq)]
pub struct TermSizeRequest {
    pub population: Population,
    pub json: bool,
    pub top: Option<usize>,
    pub baseline: Option<PathBuf>,
    pub thresholds: Thresholds,
    /// Also write the `--json` form of the report here.
    pub json_out: Option<PathBuf>,
}

/// One definition of the population, before it is counted.
pub struct Candidate<'a> {
    pub module: String,
    pub def: &'a CoreDef,
    /// A theorem, lemma or axiom (`termination_meta`'s `is_proof`).
    pub proof: bool,
    /// Admitted `Total` by the termination report.
    pub total: bool,
}

/// One row of the census (D6), identified by `(module, name)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeRow {
    pub module: String,
    pub name: String,
    pub proof: bool,
    /// `None` for an axiom, which has no proof body to count (D2).
    pub size: Option<ProofTermSize>,
}

/// Entry point for `albite doctor check proof term-size <file>`.
pub fn cmd_check_proof_term_size(file: &Path, request: &TermSizeRequest) -> ExitCode {
    let file_name = file.display().to_string();
    let rows = census_file(file, request.population);
    // Only the shell reads the baseline; everything after is pure (D5).
    let baseline_text = request
        .baseline
        .as_ref()
        .map(|path| std::fs::read_to_string(path).map_err(|e| e.to_string()));
    // `--json-out` (D8): the same report as a JSON object to a file, beside
    // the form stdout gets, so the batch lane pays for one census, not two.
    // Written only when the census answered, so a refusal leaves no file a
    // later run would read as a baseline.
    if let Some(path) = &request.json_out {
        let as_json = TermSizeRequest {
            json: true,
            ..request.clone()
        };
        let (object, _, code) =
            term_size_output(&file_name, &as_json, rows.clone(), baseline_text.clone());
        if code != 2 {
            if let Err(e) = std::fs::write(path, object) {
                eprintln!("error: cannot write {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
    }
    let (stdout, stderr, code) = term_size_output(&file_name, request, rows, baseline_text);
    print!("{stdout}");
    eprint!("{stderr}");
    ExitCode::from(code)
}

/// Elaborate `file` once, cold, and count its population.
///
/// Cold (D4): nothing is read from or written under `.albite/`, so a
/// baseline taken one day and a run taken the next differ by source alone,
/// and no diagnostic is rendered — the `Err` carries the first message.
///
/// # Errors
///
/// The reason the census could not ask: no such file, or elaboration failed.
pub fn census_file(file: &Path, population: Population) -> Result<Vec<SizeRow>, String> {
    // A missing entry file sends project discovery walking from `/`.
    if !file.is_file() {
        return Err(format!("{} is not a file", file.display()));
    }
    // Held across elaboration only, so a termination rejection still
    // elaborates — as `kernel-census` does.
    let reporting = ReportingOnly::begin();
    let project = driver::elaborate_project_cold(file, 20).map_err(|e| e.to_string())?;
    drop(reporting);
    Ok(select_rows(&candidates(&project), population))
}

/// Each definition of `project` once, with the module that defines it and
/// its population flags. A module reached by two import paths is two codegen
/// units, so the first unit naming a definition gives its module.
#[must_use]
pub fn candidates(project: &ProjectOutput) -> Vec<Candidate<'_>> {
    let admission = TerminationInput::from_meta(&project.termination_meta)
        .check(&project.defs)
        .admission;
    let mut module_of: HashMap<&str, String> = HashMap::new();
    for unit in &project.codegen_units {
        for def in &unit.defs {
            module_of
                .entry(def.name.as_str())
                .or_insert_with(|| module_name(&unit.module_path, &unit.source_file));
        }
    }
    let mut by_name: BTreeMap<&str, &CoreDef> = BTreeMap::new();
    for def in &project.defs {
        by_name.entry(def.name.as_str()).or_insert(def);
    }
    by_name
        .into_values()
        .map(|def| Candidate {
            module: module_of
                .get(def.name.as_str())
                .cloned()
                .unwrap_or_default(),
            def,
            proof: project
                .termination_meta
                .get(&def.name)
                .is_some_and(|meta| meta.is_proof),
            total: admission.get(&def.name) == Some(&AdmissionState::Total),
        })
        .collect()
}

/// A unit's module path, `::`-joined; the root module's empty path falls
/// back to its file stem.
fn module_name(module_path: &[String], source_file: &Path) -> String {
    if module_path.is_empty() {
        source_file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        module_path.join("::")
    }
}

/// The population, counted and ordered by `(module, name)`. An axiom gets a
/// row with no counts and is never walked.
#[must_use]
pub fn select_rows(candidates: &[Candidate<'_>], population: Population) -> Vec<SizeRow> {
    let mut rows: Vec<SizeRow> = candidates
        .iter()
        .filter(|c| population.admits(c))
        .map(|c| SizeRow {
            module: c.module.clone(),
            name: c.def.name.clone(),
            proof: c.proof,
            size: (!is_axiom(c.def)).then(|| count_proof_nodes(&c.def.term.term)),
        })
        .collect();
    rows.sort_by(|a, b| (&a.module, &a.name).cmp(&(&b.module, &b.name)));
    rows
}

/// The non-axiom rows' counts, summed (D6).
fn totals(rows: &[SizeRow]) -> ProofTermSize {
    rows.iter()
        .filter_map(|row| row.size)
        .fold(ProofTermSize::default(), ProofTermSize::plus)
}

fn axiom_count(rows: &[SizeRow]) -> usize {
    rows.iter().filter(|row| row.size.is_none()).count()
}

/// The exit code (15.10.26g D6): without a baseline 0 or 2, never 1; with
/// one, 1 exactly when a row is flagged. A refusal is always 2.
#[must_use]
pub fn exit_code(baseline_given: bool, refused: bool, flagged: usize) -> u8 {
    if refused {
        2
    } else {
        u8::from(baseline_given && flagged > 0)
    }
}

/// What the command prints and exits with — `(stdout, stderr, code)` — so
/// the choice between report and refusal is assertable (D5). An empty
/// population is a refusal: a census of nothing is not a clean one.
#[must_use]
pub fn term_size_output(
    file: &str,
    request: &TermSizeRequest,
    rows: Result<Vec<SizeRow>, String>,
    baseline_text: Option<Result<String, String>>,
) -> (String, String, u8) {
    let answered = match rows {
        Err(reason) => Err(reason),
        Ok(rows) if rows.is_empty() => Err(empty_population(file, request.population)),
        Ok(rows) => census_output(file, request, &rows, baseline_text),
    };
    answered.unwrap_or_else(|refusal| refusal_output(request.json, &refusal))
}

/// A non-empty census, rendered — or, under `--baseline`, compared.
fn census_output(
    file: &str,
    request: &TermSizeRequest,
    rows: &[SizeRow],
    baseline_text: Option<Result<String, String>>,
) -> Result<(String, String, u8), String> {
    if let Some(text) = baseline_text {
        return delta_output(file, request, rows, text);
    }
    let report = if request.json {
        render_json(file, request.population, rows, request.top)
    } else {
        render_text(file, rows, request.top)
    };
    Ok((report, String::new(), exit_code(false, false, 0)))
}

fn empty_population(file: &str, population: Population) -> String {
    let hint = match population {
        Population::Proof => " (try --include-total)",
        Population::ProofTotal => " (try --all-definitions)",
        Population::All => "",
    };
    format!("no definition to count in {file} — a census of nothing is not a clean one{hint}")
}

/// Exit 2 on stderr; `--json` still prints one object, so a consumer never
/// parses nothing.
fn refusal_output(json: bool, refusal: &str) -> (String, String, u8) {
    let stdout = if json {
        format!("{}\n", json!({ "error": refusal }))
    } else {
        String::new()
    };
    (
        stdout,
        format!("error: {refusal}\n"),
        exit_code(false, true, 0),
    )
}

/// The `--baseline` arm: parse, refuse a mismatched population, compare and
/// render (D5, D6). `Err` is a refusal, naming the baseline path.
fn delta_output(
    file: &str,
    request: &TermSizeRequest,
    rows: &[SizeRow],
    baseline_text: Result<String, String>,
) -> Result<(String, String, u8), String> {
    let path = request
        .baseline
        .as_ref()
        .map_or_else(String::new, |p| p.display().to_string());
    let refuse = |reason: String| format!("baseline {path}: {reason}");
    let baseline = delta::parse_baseline(&baseline_text.map_err(&refuse)?).map_err(&refuse)?;
    if baseline.population != request.population.label() {
        return Err(refuse(format!(
            "population \"{}\" differs from this run's \"{}\"",
            baseline.population,
            request.population.label()
        )));
    }
    let delta = delta::delta_rows(&baseline, rows, request.thresholds);
    let header = render::DeltaHeader {
        file,
        baseline: &path,
        population: request.population,
        thresholds: request.thresholds,
        top: request.top.unwrap_or(DELTA_DEFAULT_TOP),
    };
    let report = if request.json {
        render::render_delta_json(&header, &delta, rows)
    } else {
        render::render_delta_text(&header, &delta)
    };
    Ok((
        report,
        String::new(),
        exit_code(true, false, delta.flagged()),
    ))
}
