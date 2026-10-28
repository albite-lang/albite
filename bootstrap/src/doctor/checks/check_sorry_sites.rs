//! `albite doctor check sorry-sites <file>` — which definitions carry a
//! proof hole, and who put it there (ADR 18.9.26g).
//!
//! `check` prints `contains sorry` from one bit. This is the census behind the
//! bit: one row per definition whose Core carries a `Sorry`, each hole
//! classified by `Term::sorry_sites` — authored (at `file:line:col`),
//! synthesised (named by the lowering construct that planted it) or
//! unclassified. A section after the rows names each theorem that carries no
//! hole of its own but rests on an axiom or sorry, with its roots (ADR
//! 13.10.26g). Cost 3, no codegen. Findings are not failures: exit 0.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use albite_core::terms::analysis::{SorryCounts, SorrySite};
use serde::Serialize;

use crate::driver::{self, ProjectOutput};
use crate::span::LineIndex;

pub mod corpus;

/// One definition's holes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorryRow {
    pub name: String,
    /// Each authored hole as `file:line:col`.
    pub authored: Vec<String>,
    /// Each synthesised hole, named by its construct.
    pub synthesised: Vec<&'static str>,
    pub unclassified: usize,
}

/// One root a theorem rests on, and whether it is an axiom or a sorry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestingRoot {
    pub name: String,
    pub class: &'static str,
}

/// One theorem that carries no hole of its own but reaches one (ADR 13.10.26g D3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestingTheorem {
    pub name: String,
    pub roots: Vec<RestingRoot>,
}

/// The whole census: rows in definition order, totals per class, how many
/// definitions were examined — so an empty census over zero definitions does
/// not read like a clean one — and the theorems resting on a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorryCensus {
    pub examined: usize,
    pub rows: Vec<SorryRow>,
    pub authored: usize,
    pub synthesised: usize,
    pub unclassified: usize,
    pub rests_on: Vec<RestingTheorem>,
}

/// Resolves an authored hole's byte span to `file:line:col`.
///
/// `ProjectOutput.defs` is flat and a `TermSpan` carries no file, so the file
/// comes from the codegen unit owning the definition of that name.
struct SiteLocator<'a> {
    source_of: HashMap<&'a str, &'a Path>,
    fallback: Option<&'a Path>,
    project: &'a ProjectOutput,
    line_indexes: HashMap<&'a Path, LineIndex>,
}

impl<'a> SiteLocator<'a> {
    fn new(project: &'a ProjectOutput) -> Self {
        let mut source_of = HashMap::new();
        for unit in &project.codegen_units {
            for def in &unit.defs {
                source_of
                    .entry(def.name.as_str())
                    .or_insert(unit.source_file.as_path());
            }
        }
        Self {
            source_of,
            fallback: project.source_map.main_file(),
            project,
            line_indexes: HashMap::new(),
        }
    }

    fn locate(&mut self, def_name: &str, offset: u32) -> String {
        let Some(file) = self.source_of.get(def_name).copied().or(self.fallback) else {
            return format!("<unknown file>:@{offset}");
        };
        let Some(source) = self.project.source_map.get(file) else {
            return format!("{}:@{offset}", file.display());
        };
        let index = self
            .line_indexes
            .entry(file)
            .or_insert_with(|| LineIndex::new(source));
        let at = index.location(offset);
        format!("{}:{}:{}", file.display(), at.line, at.column)
    }
}

/// Classify every hole in every definition of an elaborated project.
#[must_use]
pub fn census(project: &ProjectOutput) -> SorryCensus {
    let mut locator = SiteLocator::new(project);
    let mut totals = SorryCounts::default();
    let mut rows = Vec::new();
    for def in project.defs.iter().filter(|d| d.term.contains_sorry()) {
        let sites = def.term.sorry_sites();
        totals.add_sites(&sites);
        let mut row = SorryRow {
            name: def.name.clone(),
            authored: Vec::new(),
            synthesised: Vec::new(),
            unclassified: 0,
        };
        for site in sites {
            match site {
                SorrySite::Authored(span) => {
                    row.authored.push(locator.locate(&def.name, span.start));
                }
                SorrySite::Synthesised(construct) => row.synthesised.push(construct.label()),
                SorrySite::Unclassified => row.unclassified += 1,
            }
        }
        rows.push(row);
    }
    SorryCensus {
        examined: project.defs.len(),
        rows,
        authored: totals.authored,
        synthesised: totals.synthesised,
        unclassified: totals.unclassified,
        rests_on: resting_rows(project),
    }
}

/// The theorems resting on an axiom or sorry, in name order (ADR 13.10.26g D3).
fn resting_rows(project: &ProjectOutput) -> Vec<RestingTheorem> {
    driver::resting_theorems(&project.defs, &project.termination_meta)
        .into_iter()
        .map(|(name, roots)| RestingTheorem {
            name,
            roots: roots
                .into_iter()
                .map(|(name, class)| RestingRoot {
                    name,
                    class: class.label(),
                })
                .collect(),
        })
        .collect()
}

/// The section after the rows: each resting theorem and what it rests on.
fn render_resting(resting: &[RestingTheorem]) -> String {
    if resting.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n{} theorem(s) rest on an axiom or sorry:\n\n",
        resting.len()
    );
    for theorem in resting {
        let roots: Vec<String> = theorem
            .roots
            .iter()
            .map(|root| format!("{} ({})", root.name, root.class))
            .collect();
        out.push_str(&format!(
            "  {}  rests on {}\n",
            theorem.name,
            roots.join(", ")
        ));
    }
    out
}

/// Human-readable report.
#[must_use]
pub fn render(census: &SorryCensus, file: &str) -> String {
    if census.rows.is_empty() {
        return format!(
            "✓ no definition carries a sorry in {file} ({} definition(s) examined)\n",
            census.examined
        );
    }
    let mut out = format!(
        "⚠ {} of {} definition(s) carry a sorry in {file}:\n\n",
        census.rows.len(),
        census.examined
    );
    for row in &census.rows {
        out.push_str(&format!("  {}\n", row.name));
        for at in &row.authored {
            out.push_str(&format!("    authored      {at}\n"));
        }
        for construct in &row.synthesised {
            out.push_str(&format!("    synthesised   {construct}\n"));
        }
        if row.unclassified > 0 {
            out.push_str(&format!("    unclassified  {}\n", row.unclassified));
        }
    }
    out.push_str(&format!(
        "\ntotals: {} authored, {} synthesised, {} unclassified\n",
        census.authored, census.synthesised, census.unclassified
    ));
    out.push_str(&render_resting(&census.rests_on));
    out
}

/// Entry point for `albite doctor check sorry-sites <path>`: a directory is
/// a corpus (ADR 15.10.26a D1), a missing path is exit 2, a file is the
/// per-file census below.
pub fn cmd_check_sorry_sites(file: &PathBuf, json: bool, verbose: bool) -> ExitCode {
    if file.is_dir() {
        return cmd_check_sorry_sites_corpus(file, json);
    }
    if !file.exists() {
        eprintln!("error: {} does not exist", file.display());
        return ExitCode::from(2);
    }
    let project = match driver::elaborate_project(file, verbose, 20, None) {
        Ok(project) => project,
        Err(why) => {
            eprintln!("error: {why}");
            return ExitCode::FAILURE;
        }
    };
    let found = census(&project);
    if json {
        match serde_json::to_string_pretty(&found) {
            Ok(text) => println!("{text}"),
            Err(why) => {
                eprintln!("error: {why}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{}", render(&found, &file.display().to_string()));
    }
    ExitCode::SUCCESS
}

/// The directory form (ADR 15.10.26a): walk, census each entry cold, render.
fn cmd_check_sorry_sites_corpus(root: &Path, json: bool) -> ExitCode {
    let report = match corpus::census_directory(root) {
        Ok(report) => report,
        Err(why) => {
            eprintln!("error: {why}");
            return ExitCode::from(2);
        }
    };
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(why) => {
                eprintln!("error: {why}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{}", corpus::render_corpus(&report));
    }
    ExitCode::from(report.exit_code())
}

// Tests: check_sorry_sites_tests.rs — rows, rests_on, text and JSON
#[cfg(test)]
#[path = "check_sorry_sites_tests.rs"]
mod tests;
