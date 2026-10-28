//! `albite doctor check type termination <file>` — the Phase-1 structural
//! recursion report (ADR 29.6.26e).
//!
//! Read-only counterpart of the E0062/E0063 gate, running the *same* engine
//! (`albite_core::terms::termination::analyze`) over the same definitions, so
//! the tool and the gate cannot disagree about whether a definition terminates.
//!
//! **Which one a user runs.** The gate explains one rejection at a time and
//! stops the build; this prints the whole census, which is what an annotation
//! pass needs. So it is reachable on a file the gate rejects *by construction*:
//! [`ReportingOnly`] forces `Enforcement::Report` for the elaboration it drives
//! (ADR 12.8.26a). Without that it would inherit the build's level, and after
//! ADR 11.8.26b made `All` the default it inherited a level that aborts
//! elaboration on the very files this tool exists to describe.
//!
//! The exit code is not softened by that: [`TerminationTally::exit`] counts
//! rejections in the report, so an uncertifiable corpus still exits non-zero.

use std::path::PathBuf;
use std::process::ExitCode;

use albite_core::terms::termination::{AdmissionState, TerminationReport};

use crate::driver;
use crate::elaborate::termination::{ReportingOnly, TerminationInput};

/// The verdict, separated from printing so it is assertable.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TerminationTally {
    /// Definitions in the trusted environment.
    pub definitions: usize,
    /// Recursive groups the checker had to certify.
    pub recursive_groups: usize,
    /// Definitions admitted as reducible total constants.
    pub total: usize,
    /// Definitions admitted opaquely — annotated `#[partial]` or tainted.
    pub partial: usize,
    /// Definitions not admitted.
    pub rejected: usize,
    /// Rejections whose reason is the proof boundary.
    pub proof_failures: usize,
}

impl TerminationTally {
    /// Summarize one report.
    #[must_use]
    pub fn from_report(report: &TerminationReport) -> Self {
        TerminationTally {
            definitions: report.admission.len(),
            recursive_groups: report.recursive_groups.len(),
            total: report
                .admission
                .values()
                .filter(|state| **state == AdmissionState::Total)
                .count(),
            partial: report
                .admission
                .values()
                .filter(|state| **state == AdmissionState::Partial)
                .count(),
            rejected: report
                .admission
                .values()
                .filter(|state| **state == AdmissionState::Rejected)
                .count(),
            proof_failures: report
                .failures
                .iter()
                .filter(|failure| failure.reason.is_proof_boundary())
                .count(),
        }
    }

    /// Non-zero exactly when a definition was not admitted.
    #[must_use]
    pub fn exit(&self) -> ExitCode {
        if self.rejected == 0 && self.proof_failures == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}

/// Entry point for `albite doctor check type termination <file>`.
pub fn cmd_check_termination(file: &PathBuf, verbose: bool, max_errors: usize) -> ExitCode {
    // Held across elaboration only; the verdict below is computed from the
    // report, not from the enforcement level (ADR 12.8.26a).
    let _reporting = ReportingOnly::begin();
    let project = match driver::elaborate_project(file, verbose, max_errors, None) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let input = TerminationInput::from_meta(&project.termination_meta);
    let report = input.check(&project.defs);
    let tally = TerminationTally::from_report(&report);
    let annotated: Vec<String> = project
        .termination_meta
        .iter()
        .filter(|(_, meta)| meta.attrs.partial)
        .map(|(name, _)| name.clone())
        .collect();

    print!("{}", render_report(&report, &tally, &annotated, verbose));
    tally.exit()
}

/// The command's whole output, as a value.
///
/// Built rather than printed so every branch — the clean/rejecting split, the
/// per-failure reason lines, the `--verbose` group listing — is assertable from
/// a unit test. `annotated` names the `#[partial]` definitions, which the
/// `--verbose` seed listing labels beside the report's foreign seeds.
#[must_use]
pub fn render_report(
    report: &TerminationReport,
    tally: &TerminationTally,
    annotated: &[String],
    verbose: bool,
) -> String {
    let mut out = String::new();
    if report.is_clean() {
        out.push_str(&format!(
            "✓ {} definition(s) admitted; {} recursive group(s) certified\n",
            tally.definitions, tally.recursive_groups
        ));
    } else {
        out.push_str(&format!(
            "✗ {} definition(s) not admitted of {} ({} recursive group(s)):\n",
            tally.rejected, tally.definitions, tally.recursive_groups
        ));
        for failure in &report.failures {
            out.push_str(&format!("  {}\n", failure.headline()));
            for note in failure.notes() {
                out.push_str(&format!("      {note}\n"));
            }
        }
    }

    out.push_str(&format!(
        "  admission: {} total, {} partial, {} rejected\n",
        tally.total, tally.partial, tally.rejected
    ));

    if !verbose {
        return out;
    }
    out.push_str("\nRecursive groups:\n");
    for group in &report.recursive_groups {
        out.push_str(&format!("  {}\n", group.join(", ")));
    }
    if !report.tainted.is_empty() {
        out.push_str(&format!("\nTainted: {}\n", report.tainted.join(", ")));
    }
    let seeds = taint_seeds(report, annotated);
    if !seeds.is_empty() {
        out.push_str("\nTaint seeds:\n");
        for seed in seeds {
            out.push_str(&format!("  {seed}\n"));
        }
    }
    out
}

/// Where taint starts, each labelled with why (ADR 3.10.26c D7): `partial`
/// for a `#[partial]` annotation, `foreign (<symbol>)` for a definition whose
/// own term calls foreign code. Both are `Partial` to the admission state, so
/// this is the one place a census reader can tell them apart.
fn taint_seeds(report: &TerminationReport, annotated: &[String]) -> Vec<String> {
    let mut seeds: Vec<String> = annotated
        .iter()
        .map(|name| format!("{name} — partial"))
        .collect();
    seeds.extend(
        report
            .foreign_seeds
            .iter()
            .filter(|(name, _)| !annotated.contains(name))
            .map(|(name, symbol)| format!("{name} — foreign ({symbol})")),
    );
    seeds.sort();
    seeds
}

#[cfg(test)]
mod tests;
