//! Table mode for `diff bootstrap-selfhost-check` (ADR 3.10.26a): several
//! paths, or a directory, become one row per `.tg` entry. The decisions are
//! in `table.rs`; this file only resolves the executables, runs both
//! compilers per entry and prints.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::table::{classify_run, expand_paths, Side};
use super::table_verdict::{render_table, table_exit_code, Row};
use super::{preflight, report_missing, run_check, MissingInput, RunOptions};

/// Why table mode refused before running anything (D4's exit 2).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    Missing(MissingInput),
    NoEntries(String),
}

/// Print the table and return D4's exit code.
pub(super) fn cmd_table_mode(paths: &[PathBuf], run: &RunOptions) -> ExitCode {
    match table_rows(paths, run) {
        Ok(rows) => {
            print!("{}", render_table(&rows, run.verbose));
            ExitCode::from(table_exit_code(&rows))
        }
        Err(Refusal::Missing(missing)) => {
            report_missing(&missing, &run.selfhost_binary);
            ExitCode::from(2)
        }
        Err(Refusal::NoEntries(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

/// Resolve `path` against the caller's working directory (D2), so a relative
/// `--selfhost-binary ./albite1` — or a bare `albite1`, which is a file
/// here and never a `PATH` lookup — still names the caller's file once the
/// child runs from inside a fixture directory.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Run both compilers on every entry and classify each row.
pub(super) fn table_rows(paths: &[PathBuf], run: &RunOptions) -> Result<Vec<Row>, Refusal> {
    preflight(paths, &run.selfhost_binary).map_err(Refusal::Missing)?;
    let entries = expand_paths(paths).map_err(Refusal::NoEntries)?;
    let bootstrap_exe = absolute(&run.bootstrap_exe);
    let selfhost_exe = absolute(&run.selfhost_binary);

    let mut rows = Vec::with_capacity(entries.len());
    for entry in &entries {
        let label = entry.label();
        let working_dir = entry.working_dir.as_deref();
        let arg = entry.arg.as_os_str();
        // `--no-cache`: a `.albite/` an earlier build left in the fixture
        // directory must not answer for this run (D2). The self-host has no
        // such flag.
        let bootstrap_args = [OsStr::new("check"), OsStr::new("--no-cache"), arg];
        let selfhost_args = [OsStr::new("check"), arg];
        if run.verbose {
            eprintln!("  running: {label}");
        }
        let bootstrap = run_check(&bootstrap_exe, &bootstrap_args, working_dir, run.timeout);
        let selfhost = run_check(&selfhost_exe, &selfhost_args, working_dir, run.timeout);
        rows.push(Row::new(
            label.clone(),
            classify_run(&bootstrap, &label, Side::Bootstrap),
            classify_run(&selfhost, &label, Side::Selfhost),
        ));
    }
    Ok(rows)
}
