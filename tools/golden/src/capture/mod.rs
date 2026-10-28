//! One capture path for both golden modes (ADR 5.10.26b).
//!
//! The check and `--update` used to read a `compile` golden's binary output
//! differently: the check took stdout then stderr, the update stdout alone. A
//! fixture that reports through stderr (`int_trap`, which traps on overflow)
//! therefore passed `make golden` and was blanked by `make golden-update`.
//! Both modes now call [`capture_compile_run`] and serialize through
//! [`snapshot_bytes`], so what the check compares is what the update writes,
//! and every update goes through [`write_snapshot`], which writes only on
//! change.

use std::fs;
use std::path::Path;
use std::process::Command;

/// What compiling a `compile` fixture and running its binary produced (D3).
///
/// Only [`CompileRun::Ran`] carries content. A nonzero exit is still `Ran`:
/// `int_trap` traps on purpose, so its exit status is part of the result, not
/// a failure to obtain one.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompileRun {
    /// The binary was spawned and exited, with any status.
    Ran { stdout: Vec<u8>, stderr: Vec<u8> },
    /// `albite compile` failed or could not be spawned.
    CompileFailed,
    /// The compile succeeded but left no binary beside the fixture.
    NoBinary,
    /// The binary exists but could not be spawned.
    RunFailed(String),
}

/// Compile `tg`, run the binary it produces, then remove that binary.
pub(crate) fn capture_compile_run(compiler: &Path, tg: &Path) -> CompileRun {
    let compiled = Command::new(compiler)
        .args(["compile", &tg.to_string_lossy()])
        .output()
        .is_ok_and(|o| o.status.success());
    if !compiled {
        return CompileRun::CompileFailed;
    }

    // The binary is the fixture's path with the extension removed.
    let binary = tg.with_extension("");
    if !binary.exists() {
        return CompileRun::NoBinary;
    }

    let output = Command::new(&binary).output();
    let _ = fs::remove_file(&binary);
    match output {
        Ok(o) => CompileRun::Ran {
            stdout: o.stdout,
            stderr: o.stderr,
        },
        Err(e) => CompileRun::RunFailed(e.to_string()),
    }
}

/// The exact bytes of a `compile` snapshot: stdout then stderr, decoded
/// lossily, trailing whitespace trimmed, and exactly one final `\n` (D1).
pub(crate) fn snapshot_bytes(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(stderr));
    snapshot_text(&text)
}

/// The snapshot form of already-captured text: trimmed, one final `\n`.
pub(crate) fn snapshot_text(text: &str) -> String {
    format!("{}\n", text.trim_end())
}

/// What one `--update` did to one snapshot (D2, D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateOutcome {
    /// The bytes differed and the file was (re)written.
    Updated,
    /// The file already held these bytes; it was not touched.
    Unchanged,
    /// No result to write, or the write itself failed; the file is as it was.
    Failed,
}

/// Write `bytes` to `path` only when they differ from what is on disk.
pub(crate) fn write_snapshot(path: &Path, bytes: &str) -> UpdateOutcome {
    if fs::read(path).is_ok_and(|current| current == bytes.as_bytes()) {
        return UpdateOutcome::Unchanged;
    }
    match fs::write(path, bytes) {
        Ok(()) => UpdateOutcome::Updated,
        Err(e) => {
            println!("\x1b[31mFailed to write\x1b[0m {} ({e})", path.display());
            UpdateOutcome::Failed
        }
    }
}

/// The line `--update` prints for an outcome; an unchanged snapshot prints
/// nothing, so the output names exactly the files that changed.
pub(crate) fn update_report(path: &Path, outcome: UpdateOutcome) -> Option<String> {
    match outcome {
        UpdateOutcome::Updated => Some(format!("\x1b[32mUpdated\x1b[0m {}", path.display())),
        UpdateOutcome::Unchanged | UpdateOutcome::Failed => None,
    }
}

/// One category's `--update` counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpdateTally {
    pub updated: u32,
    pub unchanged: u32,
    pub failed: u32,
}

impl UpdateTally {
    pub(crate) fn record(&mut self, outcome: UpdateOutcome) {
        match outcome {
            UpdateOutcome::Updated => self.updated += 1,
            UpdateOutcome::Unchanged => self.unchanged += 1,
            UpdateOutcome::Failed => self.failed += 1,
        }
    }

    /// The one-line update summary, as a value so it can be asserted.
    pub(crate) fn summary_line(&self) -> String {
        let mut line = format!("  {} updated, {} unchanged", self.updated, self.unchanged);
        if self.failed > 0 {
            line.push_str(&format!(", {} failed", self.failed));
        }
        line.push('\n');
        line
    }
}

#[cfg(test)]
mod tests;
