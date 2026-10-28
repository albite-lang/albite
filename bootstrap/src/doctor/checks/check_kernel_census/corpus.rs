//! `kernel-census --corpus <root>…` (ADR 2.10.26o): the census over every
//! entry file under one or more roots, and the register diff over all of them.
//!
//! - **Entries** (D2): every `.tg` under a root, minus companions and minus
//!   every file some candidate's `mod`-declared tree reaches as a non-root
//!   node. Workspace siblings and mounted packages are not consulted.
//! - **Each entry** elaborates in-process, cold and quiet
//!   (`driver::elaborate_project_cold`), under the census's reporting window,
//!   one at a time; its outcome is census-ed, empty, or does not elaborate (D3).
//! - **Each term-mode theorem** of an elaborated entry is decided by the
//!   build's own gate (`elaborate::kernel::term_verdicts`) against the
//!   injected register and root, and `register_diff` compares (D4).
//! - **Under `--proof-bearing`** (ADR 3.10.26s) an entry whose tree adds no
//!   proof-bearing file to what earlier entries cover is not elaborated: it
//!   reads *skipped*, and the files only it reaches are *not examined*.
//!
//! Exit 0 whenever the census ran, 2 when it could not ask (D5).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::ast::{Item, TheoremBody};
use crate::driver::{self, ProjectOutput};
use crate::elaborate::kernel::{
    decreasing_hints, repository_root, term_verdicts, DefinitionSite, KernelMode, Register,
    TermGate,
};
use crate::elaborate::termination::{ReportingOnly, TerminationInput};
use crate::elaborate::CoreDef;
use crate::parser::Parser;

use super::entry_plan::{
    parse_for_proofs, parse_tree_files, plan_entries, select_proof_entries, walk_roots,
};
use super::register_diff::{register_diff, FileStatus, Observation, RegisterDiff, TheoremKey};
use super::{record_presentation, run_census, CensusTally};

/// What became of one entry file (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    Censused(CensusTally),
    /// No proof-relevant definition.
    Empty,
    /// Fails before the gate; the first line of the error.
    DoesNotElaborate(String),
    /// Not elaborated under `--proof-bearing`: its tree adds no proof item
    /// the parse recognised (3.10.26s D3, 14.10.26b D3).
    Skipped,
}

/// The corpus run, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusReport {
    /// The canonical root register keys were taken against (D4).
    pub keyed_against: PathBuf,
    /// Each entry, by its register key where it has one, and its outcome.
    pub files: Vec<(String, FileOutcome)>,
    pub diff: RegisterDiff,
    /// Whether `--proof-bearing` chose the entries (3.10.26s D1).
    pub proof_bearing: bool,
}

/// Entry point for `albite doctor check proof kernel-census --corpus`.
pub fn cmd_check_kernel_census_corpus(
    roots: &[PathBuf],
    proof_bearing: bool,
    verbose: bool,
) -> ExitCode {
    let root = repository_root();
    let root = fs::canonicalize(&root).unwrap_or(root);
    match census_corpus(roots, proof_bearing, Register::real(), &root) {
        Ok(report) => {
            print!("{}", render_corpus(&report, verbose));
            ExitCode::SUCCESS
        }
        Err(refusal) => {
            eprintln!("error: {refusal}");
            ExitCode::from(2)
        }
    }
}

/// The whole run over `roots`, keyed against `key_root` and diffed against
/// `register` — both injected, so a test's tempdir rows can match (D4).
/// With `proof_bearing`, only the entries `select_proof_entries` keeps are
/// elaborated (3.10.26s).
///
/// # Errors
///
/// A root that is missing or unreadable, or zero entry files (D5).
pub fn census_corpus(
    roots: &[PathBuf],
    proof_bearing: bool,
    register: &Register,
    key_root: &Path,
) -> Result<CorpusReport, String> {
    let candidates = walk_roots(roots)?;
    let plan = plan_entries(&candidates);
    if plan.entries.is_empty() {
        return Err(format!(
            "no entry file under {} — a census of nothing is not a clean one",
            roots
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let key = |path: &Path| crate::elaborate::kernel::register_key(path, key_root);
    let mut run = CorpusRun::default();
    for companion in &plan.companions {
        if let Some(file) = key(companion) {
            run.files.insert(file, FileStatus::Companion);
        }
    }
    let kept = proof_bearing
        .then(|| select_proof_entries(&plan, &parse_tree_files(&plan, parse_for_proofs)));
    let mut skipped = Vec::new();
    let mut files = Vec::with_capacity(plan.entries.len());
    for entry in &plan.entries {
        let entry_name = key(entry).unwrap_or_else(|| entry.display().to_string());
        if kept.as_ref().is_some_and(|kept| !kept.contains(entry)) {
            skipped.push(entry);
            files.push((entry_name, FileOutcome::Skipped));
            continue;
        }
        let reporting = ReportingOnly::begin();
        let elaborated = driver::elaborate_project_cold(entry, 20);
        drop(reporting);
        let outcome = match elaborated {
            Ok(project) => {
                // A built file that defines nothing still answers *absent*.
                for file in plan.trees.get(entry).into_iter().flatten() {
                    if let Some(file) = key(file) {
                        run.files.insert(file, FileStatus::Censused);
                    }
                }
                let names = project.concrete_type_names();
                let hints = decreasing_hints(&project.termination_meta);
                let presentation = record_presentation(&project);
                let gate = TermGate {
                    mode: KernelMode::Gate,
                    register,
                    root: Some(key_root),
                    concrete_type_names: Some(&names),
                    decreasing: Some(&hints),
                    presentation: &presentation,
                };
                run.observe(&project, &entry_name, &gate, key);
                let (tally, _) = run_census(&project, false);
                if tally.population() == 0 {
                    FileOutcome::Empty
                } else {
                    FileOutcome::Censused(tally)
                }
            }
            Err(error) => {
                for file in plan.trees.get(entry).into_iter().flatten() {
                    if let Some(file) = key(file) {
                        run.files
                            .entry(file)
                            .or_insert(FileStatus::DoesNotElaborate);
                    }
                }
                let message = error.to_string();
                FileOutcome::DoesNotElaborate(message.lines().next().unwrap_or("").to_string())
            }
        };
        files.push((entry_name, outcome));
    }
    // After every kept entry, so a file a kept entry also reaches keeps its
    // status: only the files no kept entry reaches read skipped (D4).
    for file in skipped
        .into_iter()
        .filter_map(|e| plan.trees.get(e))
        .flatten()
    {
        if let Some(file) = key(file) {
            run.files.entry(file).or_insert(FileStatus::Skipped);
        }
    }
    Ok(CorpusReport {
        keyed_against: key_root.to_path_buf(),
        files,
        diff: register_diff(register.rows(), &run.observed, &run.files),
        proof_bearing,
    })
}

/// What the run has seen so far: every theorem's observations and every
/// reached file's status, plus each parsed file's proof modes.
#[derive(Default)]
pub(super) struct CorpusRun {
    pub(super) observed: BTreeMap<TheoremKey, Vec<(String, Observation)>>,
    files: BTreeMap<String, FileStatus>,
    modes: BTreeMap<PathBuf, BTreeMap<String, bool>>,
}

impl CorpusRun {
    /// Decide every theorem `project` defines, through the gate (D4).
    fn observe(
        &mut self,
        project: &ProjectOutput,
        entry: &str,
        gate: &TermGate<'_>,
        key: impl Fn(&Path) -> Option<String>,
    ) {
        let sites: Vec<(PathBuf, &CoreDef)> = project
            .codegen_units
            .iter()
            .flat_map(|unit| {
                let file = fs::canonicalize(&unit.source_file)
                    .unwrap_or_else(|_| unit.source_file.clone());
                unit.defs.iter().map(move |def| (file.clone(), def))
            })
            .collect();
        let definition_sites: Vec<DefinitionSite<'_>> = sites
            .iter()
            .map(|(file, def)| DefinitionSite {
                module: Some(file.as_path()),
                def,
            })
            .collect();
        let mut term_theorems = Vec::new();
        for (file, def) in &sites {
            let Some(file_key) = key(file) else { continue };
            self.files.insert(file_key.clone(), FileStatus::Censused);
            let modes = self
                .modes
                .entry(file.clone())
                .or_insert_with(|| proof_modes(file));
            match modes.get(&def.name) {
                Some(true) => self.record(&file_key, &def.name, entry, Observation::Tactic),
                Some(false) => term_theorems.push((file_key, *def, file.as_path())),
                None => {}
            }
        }
        let admission = TerminationInput::from_meta(&project.termination_meta)
            .check(&project.defs)
            .admission;
        let asked: Vec<(&CoreDef, &Path)> =
            term_theorems.iter().map(|(_, d, f)| (*d, *f)).collect();
        let verdicts = term_verdicts(&definition_sites, &admission, gate, &asked);
        for ((file_key, def, _), verdict) in term_theorems.iter().zip(verdicts) {
            self.record(file_key, &def.name, entry, Observation::Decided(verdict));
        }
    }

    /// One observation of `theorem` from `entry`. A file the entry reaches by
    /// two import paths is one codegen unit per path, so the same decision
    /// arrives twice: an identical pair is dropped, a differing one kept for
    /// the diff to report as `EntriesDisagree` (ADR 9.10.26p D4).
    pub(super) fn record(&mut self, file: &str, theorem: &str, entry: &str, seen: Observation) {
        let observations = self
            .observed
            .entry((file.to_string(), theorem.to_string()))
            .or_default();
        let observation = (entry.to_string(), seen);
        if !observations.contains(&observation) {
            observations.push(observation);
        }
    }
}

/// Each theorem and lemma `file` declares, `true` for a `by` block. A file
/// that cannot be read or parsed declares none.
fn proof_modes(file: &Path) -> BTreeMap<String, bool> {
    let Ok(source) = fs::read_to_string(file) else {
        return BTreeMap::new();
    };
    let (ast, _) = Parser::new(&source).parse();
    ast.items
        .iter()
        .filter_map(|item| match item {
            Item::Theorem(t) | Item::Lemma(t) => Some((
                t.name.name.clone(),
                matches!(t.body, TheoremBody::Tactics(_)),
            )),
            _ => None,
        })
        .collect()
}

/// ` — sort Type₀: a, b` naming the statements that state data (ADR 4.10.26i
/// D6 (b)), or nothing when there are none, so a clean line is unchanged.
pub(super) fn data_statements_suffix(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!(" — sort Type₀: {}", names.join(", "))
    }
}

/// The report: per-file lines under `-v`, the totals, the key root and the
/// register diff (§2.1).
#[must_use]
pub fn render_corpus(report: &CorpusReport, verbose: bool) -> String {
    let mut out = String::new();
    let (mut censused, mut empty, mut failing, mut skipped) = (0, 0, 0, 0);
    let mut by_cause: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (file, outcome) in &report.files {
        let line = match outcome {
            FileOutcome::Censused(tally) => {
                censused += 1;
                for (cause, n) in &tally.primary {
                    *by_cause.entry(cause.label()).or_default() += n;
                }
                format!(
                    "census-ed ({} proof-relevant, {} admitted, {} refused){}",
                    tally.proof_relevant,
                    tally.admitted,
                    tally.refused,
                    data_statements_suffix(&tally.data_statements)
                )
            }
            FileOutcome::Empty => {
                empty += 1;
                "empty".to_string()
            }
            FileOutcome::DoesNotElaborate(first) => {
                failing += 1;
                format!("does not elaborate: {first}")
            }
            FileOutcome::Skipped => {
                skipped += 1;
                "skipped (no proof item recognised in its tree)".to_string()
            }
        };
        if verbose {
            let _ = writeln!(out, "  {file}: {line}");
        }
    }
    let refusals: usize = by_cause.values().sum();
    let causes: Vec<String> = by_cause.iter().map(|(c, n)| format!("{c} {n}")).collect();
    // Only under the flag, so a default run's line is today's (3.10.26s D1, D3).
    let skipped = if report.proof_bearing {
        format!(", {skipped} skipped")
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "  totals:    {} files: {censused} census-ed, {empty} empty{skipped}, {failing} not elaborating; {refusals} refusals{}",
        report.files.len(),
        if causes.is_empty() {
            String::new()
        } else {
            format!(" ({})", causes.join(", "))
        }
    );
    let _ = writeln!(
        out,
        "  keyed:     against {}",
        report.keyed_against.display()
    );
    out.push_str(&report.diff.render());
    out
}
