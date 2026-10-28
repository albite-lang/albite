//! `albite doctor check proof kernel-census <file>` — every proof-relevant
//! definition's elaborated Core through the kernel's proof judgment, each
//! refusal sorted by cause (ADR 30.9.26f).
//!
//! A census, not a gate (D1): it exits 0 whatever it counts, and 2 only when
//! it could not ask — the corpus did not elaborate, or nothing was in the
//! population. It reads: nothing that `check`, `run`, `compile` or `test`
//! accept moves. Since ADR 2.10.26c the build calls the kernel too, on every
//! tactic proof, through the same context builder (`elaborate::kernel`,
//! D6), so the census measures the context the build enforces.
//!
//! Terms go to `check_proof` exactly as the elaborator emitted them — no
//! `@`-stripping, no normalisation (D4) — because what the census measures is
//! what a B10 tactic would hand the kernel if it reused this output.
//!
//! This file is the effectful shell: elaboration under [`ReportingOnly`], the
//! termination report and each definition's context. The decisions are in
//! `row` and in `elaborate::kernel`'s `classify`, and each definition's
//! context and probes come from `elaborate::kernel` too, so the build's
//! kernel gate shares them (ADR 2.10.26d D10).

mod corpus;
mod entry_plan;
mod register_diff;
mod render;
mod row;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use albite_core::terms::analysis;
use albite_core::terms::termination::AdmissionState;
use albite_core::typecheck::present_definition;
use albite_core::types::Sort;
use albite_core::{Term, Type, TypeError};

use crate::driver::{self, ProjectOutput};
use crate::elaborate::termination::{ReportingOnly, TerminationInput};
use crate::elaborate::{carries_proof, CoreDef};

pub use crate::elaborate::kernel::{bind_globals, mentioned_globals, ElaboratedProbes};
use crate::elaborate::kernel::{
    bind_with_bodies, decreasing_hints, delta_admission, present_definitions, RecordPresentation,
};
pub use crate::elaborate::kernel::{classify, Cause, CensusTally, Probes};
pub use corpus::{cmd_check_kernel_census_corpus, CorpusReport, FileOutcome};
pub use render::render_census;
pub use row::{census_definition, CensusRow, ProbeEngine};

/// Which per-definition lines the report prints under its counts (ADR
/// 1.10.26m D1). The counts and the cause table print in every mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CensusRows {
    /// No per-row lines.
    None,
    /// One line per refused definition — `-v`.
    Refused,
    /// One line per definition in the population — `--all-rows`.
    All,
}

impl CensusRows {
    /// D1's table: `--all-rows` prints every row whether or not `-v` is
    /// given, and `-v` alone prints the refused ones.
    #[must_use]
    pub fn from_flags(verbose: bool, all_rows: bool) -> Self {
        if all_rows {
            CensusRows::All
        } else if verbose {
            CensusRows::Refused
        } else {
            CensusRows::None
        }
    }

    /// Whether `row` gets a line of its own in this mode.
    #[must_use]
    pub fn shows(self, row: &CensusRow) -> bool {
        match self {
            CensusRows::None => false,
            CensusRows::Refused => row.refusal.is_some(),
            CensusRows::All => true,
        }
    }
}

/// Entry point for `albite doctor check proof kernel-census <file>`.
pub fn cmd_check_kernel_census(file: &Path, include_total: bool, shown: CensusRows) -> ExitCode {
    // Refused before the driver sees it: a missing entry file sends project
    // discovery walking from `/`, which a census must not wait on (D1).
    if !file.is_file() {
        eprintln!("error: {} is not a file", file.display());
        return ExitCode::from(2);
    }
    // Held across elaboration only, so a corpus with a termination rejection
    // still elaborates; the rejected definition is then bound `Opaque` (D3).
    let reporting = ReportingOnly::begin();
    let project = match driver::elaborate_project(file, false, 20, None) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    drop(reporting);

    let (tally, rows) = run_census(&project, include_total);
    let file_name = file.display().to_string();
    match census_report(&file_name, include_total, &tally, &rows, shown) {
        Ok(report) => print!("{report}"),
        Err(refusal) => eprintln!("{refusal}"),
    }
    tally.exit()
}

/// What the command prints: the report on stdout, or — for an empty
/// population — the refusal on stderr (D1). A value, so the choice between
/// them is assertable; the exit code alone cannot tell them apart.
///
/// # Errors
///
/// The refusal line, when the population is empty.
pub fn census_report(
    file: &str,
    include_total: bool,
    tally: &CensusTally,
    rows: &[CensusRow],
    shown: CensusRows,
) -> Result<String, String> {
    if tally.population() > 0 {
        return Ok(render_census(file, tally, rows, shown));
    }
    let hint = if include_total {
        ""
    } else {
        " (try --include-total)"
    };
    Err(format!(
        "error: no definition to check in {file} — a census of nothing is not a clean one{hint}"
    ))
}

/// Each record of `project` presented by its encoding (ADR 3.10.26p D2): the
/// gate builds the same value from the same two maps.
#[must_use]
pub fn record_presentation(project: &ProjectOutput) -> RecordPresentation {
    RecordPresentation::new(&project.encoded_types, project.record_types.keys())
}

/// Run every definition in the population through the kernel, in name order.
pub fn run_census(project: &ProjectOutput, include_total: bool) -> (CensusTally, Vec<CensusRow>) {
    let admission = TerminationInput::from_meta(&project.termination_meta)
        .check(&project.defs)
        .admission;
    let hints = decreasing_hints(&project.termination_meta);
    let presentation = record_presentation(project);
    let presented = present_definitions(&project.defs, &presentation);
    let mut delta = delta_admission(
        presented.iter().map(AsRef::as_ref),
        &admission,
        Some(&hints),
    );
    let probes = ElaboratedProbes::new(&mut delta, project.concrete_type_names());
    let defs_by_name: HashMap<&str, &Type> = presented
        .iter()
        .map(|def| (def.name.as_str(), &def.ty))
        .collect();

    let mut population: Vec<(&CoreDef, bool)> = presented
        .iter()
        .map(AsRef::as_ref)
        .filter_map(|def| {
            let proof = is_proof_relevant(project, def);
            let total = admission.get(&def.name) == Some(&AdmissionState::Total);
            (proof || (include_total && total)).then_some((def, proof))
        })
        .collect();
    population.sort_by(|a, b| a.0.name.cmp(&b.0.name));

    let mut tally = CensusTally::default();
    let mut rows = Vec::with_capacity(population.len());
    for (def, proof) in population {
        if proof {
            tally.proof_relevant += 1;
            if states_data(&def.ty) {
                tally.data_statements.push(def.name.clone());
            }
        } else {
            tally.total_only += 1;
        }
        let row = if is_axiom(def) {
            CensusRow::axiom(&def.name)
        } else {
            let mut mentioned = mentioned_globals(&def.term.term, &def.ty);
            mentioned.remove(&def.name);
            let ctx = bind_with_bodies(&mentioned, &defs_by_name, &admission, &mut delta);
            let presented = present_definition(&def.name, &def.ty, &def.term.term);
            census_definition(&def.name, &ctx, &presented, &def.ty, &probes)
        };
        tally.record(row.primary, row.secondary);
        rows.push(row);
    }
    (tally, rows)
}

/// Whether `def` is in the set the build's gate checks: a theorem, lemma or
/// axiom, or a non-`#[partial]` `fn` whose type carries a proof (ADR
/// 13.10.26t D4 — read from the type, not from termination's `is_proof`,
/// whose rules for a body this must not change).
pub(crate) fn is_proof_relevant(project: &ProjectOutput, def: &CoreDef) -> bool {
    let meta = project.termination_meta.get(&def.name);
    if meta.is_some_and(|meta| meta.is_proof) {
        return true;
    }
    !meta.is_some_and(|meta| meta.attrs.partial)
        && carries_proof(&def.ty, |name| project.encoded_types.get(name))
}

/// Whether a proof-relevant definition's statement sorts `Type₀` (ADR
/// 4.10.26i D6 (b)). A statement with no sort — a poison — is transit, not
/// data: its error was reported where it arose.
pub(crate) fn states_data(statement: &Type) -> bool {
    statement.sort_of() == Some(Sort::Type0)
}

/// Whether `def` is an axiom: the shared core shape test (ADR 13.10.26g D2),
/// read against the definition's item span.
///
/// `termination_meta` marks axioms and theorems alike as proof-relevant, so
/// the census tells them apart by shape (P0.3). `term-size` and `sorry-sites`
/// share it, so every census agrees on what an axiom is (ADR 13.10.26c D2).
pub(crate) fn is_axiom(def: &CoreDef) -> bool {
    def.term
        .span
        .is_some_and(|item_span| analysis::is_axiom(&def.term.term, item_span))
}

impl ProbeEngine for ElaboratedProbes {
    fn probes_for(&self, refusal: &TypeError) -> Probes {
        ElaboratedProbes::probes_for(self, refusal)
    }
}
