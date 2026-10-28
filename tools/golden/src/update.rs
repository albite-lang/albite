//! `--update` mode for the golden runner. Split out of `main.rs` (ADR 16.7.26b
//! file-size paydown). Regenerates the `.expected` snapshot for a test from the
//! current compiler output (or the compiled-then-run output for `compile`).
//!
//! Every path writes through [`write_snapshot`], which leaves an unchanged
//! snapshot untouched and unreported (ADR 5.10.26b D2).

use std::path::Path;

use crate::capture::{
    capture_compile_run, snapshot_bytes, snapshot_text, update_report, write_snapshot, CompileRun,
    UpdateOutcome,
};
use crate::compiler::{read_args_file, run_compiler};
use crate::{Category, Cli};

pub(crate) fn update_test(
    cli: &Cli,
    category: &Category,
    tg: &Path,
    expected: &Path,
) -> UpdateOutcome {
    if matches!(category, Category::JsonParity) {
        // It compares two live runs; there is nothing to snapshot.
        println!("[json-parity] has no snapshots — nothing to update");
        return UpdateOutcome::Unchanged;
    }
    let bytes = if matches!(category, Category::Compile) {
        match compile_snapshot(cli, tg) {
            Some(bytes) => bytes,
            None => return UpdateOutcome::Failed,
        }
    } else {
        let extra_args = read_args_file(tg);
        snapshot_text(&run_compiler(
            &cli.compiler,
            category.tungsten_cmd(),
            tg,
            &extra_args,
        ))
    };

    let outcome = write_snapshot(expected, &bytes);
    if let Some(line) = update_report(expected, outcome) {
        println!("{line}");
    }
    outcome
}

/// The snapshot a `compile` fixture's run produced, or `None` — reported —
/// when there was no run to snapshot (D3): the existing file is then kept.
fn compile_snapshot(cli: &Cli, tg: &Path) -> Option<String> {
    let reason = match capture_compile_run(&cli.compiler, tg) {
        CompileRun::Ran { stdout, stderr } => return Some(snapshot_bytes(&stdout, &stderr)),
        CompileRun::CompileFailed => "Failed to compile".to_string(),
        CompileRun::NoBinary => "No binary produced for".to_string(),
        CompileRun::RunFailed(e) => format!("Failed to run ({e})"),
    };
    println!("\x1b[31m{reason}\x1b[0m {}", tg.display());
    None
}
