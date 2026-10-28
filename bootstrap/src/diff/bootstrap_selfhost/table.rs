//! The pure half of `diff bootstrap-selfhost-check`'s table mode (ADR 3.10.26a
//! D5): expanding the input paths, reading one run's output and classifying
//! that run. A row, the rendered table and the exit code are in the sibling
//! `table_verdict.rs`. Nothing here spawns a compiler — `run_check` in the
//! parent module is the only spawner — so every decision is testable over
//! strings and tempdir trees.

use std::collections::HashSet;
use std::path::PathBuf;

use albite_bootstrap::utils::{list_directory, walk_tg_tree};

/// One `.tg` file to run both compilers on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    /// The child's working directory: the directory argument this entry was
    /// found under, or `None` for a file argument, which runs from the
    /// caller's directory (D2).
    pub(super) working_dir: Option<PathBuf>,
    /// The path passed to each compiler: relative to `working_dir` for a
    /// directory's entry, the argument's own spelling for a file.
    pub(super) arg: PathBuf,
}

impl Entry {
    /// The row label, which is also the `<entry>` a verdict line names.
    pub(super) fn label(&self) -> String {
        self.arg.display().to_string()
    }
}

/// Expand the input paths into entries (D1): a file is kept as given, a
/// directory contributes every `.tg` file under it — skipping subdirectories
/// whose name starts with `.` — sorted by relative path. Rows follow input
/// order, and an entry reached twice (by canonical path) is kept once, first
/// occurrence winning. Zero entries in total is a refusal, never a zero-row
/// success.
pub(super) fn expand_paths(paths: &[PathBuf]) -> Result<Vec<Entry>, String> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for path in paths {
        let found = if path.is_dir() {
            // Unreadable directories contribute nothing here: the table is a
            // differential, and a missing entry is missing from both sides.
            walk_tg_tree(path, &mut list_directory)
                .files
                .into_iter()
                .map(|arg| Entry {
                    working_dir: Some(path.clone()),
                    arg,
                })
                .collect()
        } else {
            vec![Entry {
                working_dir: None,
                arg: path.clone(),
            }]
        };
        for entry in found {
            let full = match &entry.working_dir {
                Some(dir) => dir.join(&entry.arg),
                None => entry.arg.clone(),
            };
            let key = std::fs::canonicalize(&full).unwrap_or(full);
            if seen.insert(key) {
                entries.push(entry);
            }
        }
    }
    if entries.is_empty() {
        let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        return Err(format!("no `.tg` entries under {}", names.join(", ")));
    }
    Ok(entries)
}

/// How one compiler run ended, as `run_check` observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunStatus {
    Exited(i32),
    Signalled(i32),
    /// Killed by the tool at its `--timeout`, in seconds (D6).
    TimedOut(u64),
}

/// One finished run: its status and both captured streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunCapture {
    pub(super) status: RunStatus,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

/// Which compiler a run belongs to — only the self-host gets the stack hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Bootstrap,
    Selfhost,
}

/// A clean or warning accept: the definition count, and a `⚠` line's suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Accepted {
    pub(super) definitions: usize,
    pub(super) warning: Option<String>,
}

/// One compiler's run, in exactly one of three outcomes (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Accepted(Accepted),
    Refused(String),
    Fault(String),
}

impl Outcome {
    /// The table cell, untruncated.
    pub(super) fn cell(&self) -> String {
        match self {
            Self::Accepted(Accepted {
                definitions,
                warning: None,
            }) => format!("ok, {definitions}"),
            Self::Accepted(Accepted {
                definitions,
                warning: Some(suffix),
            }) => format!("ok, {definitions} ⚠ {suffix}"),
            Self::Refused(line) => line.clone(),
            Self::Fault(reason) => format!("fault: {reason}"),
        }
    }
}

/// Read the single verdict line for `entry` from stdout:
/// `^(✓|⚠) <entry>: (\d+) definition\(s\)(, .*)?$`. Zero matching lines, or
/// more than one, is `Err` naming which — an ambiguous count proves nothing.
pub(super) fn parse_count_line(stdout: &str, entry: &str) -> Result<Accepted, String> {
    let mut matches: Vec<Accepted> = stdout
        .lines()
        .filter_map(|line| parse_one_count_line(line, entry))
        .collect();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err("exit 0 with no count line".to_string()),
        n => Err(format!("exit 0 with {n} count lines")),
    }
}

fn parse_one_count_line(line: &str, entry: &str) -> Option<Accepted> {
    let (warned, rest) = if let Some(rest) = line.strip_prefix("✓ ") {
        (false, rest)
    } else {
        (true, line.strip_prefix("⚠ ")?)
    };
    let rest = rest.strip_prefix(entry)?.strip_prefix(": ")?;
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let definitions = rest[..digits_end].parse().ok()?;
    let tail = rest[digits_end..].strip_prefix(" definition(s)")?;
    let suffix = if tail.is_empty() {
        None
    } else {
        Some(tail.strip_prefix(", ")?.to_string())
    };
    Some(Accepted {
        definitions,
        warning: if warned { suffix } else { None },
    })
}

/// Remove ANSI CSI escape sequences (`ESC [ … final-byte`). The bootstrap
/// colours its rendered header even into a pipe.
pub(super) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A diagnostic header (D3): a line that STARTS with `error[` or `error:`
/// (the self-host), or with `[E<digits>] Error:` (the bootstrap's rendered
/// form). The bootstrap's `error: aborting due to …` trailer is not one, and
/// an indented continuation never starts with either.
fn is_diagnostic_header(line: &str) -> bool {
    if line.starts_with("error: aborting") {
        return false;
    }
    if line.starts_with("error[") || line.starts_with("error:") {
        return true;
    }
    let Some(rest) = line.strip_prefix("[E") else {
        return false;
    };
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    digits_end > 0 && rest[digits_end..].starts_with("] Error:")
}

/// The first diagnostic header, stderr before stdout, ANSI stripped.
pub(super) fn first_error_line(stderr: &str, stdout: &str) -> Option<String> {
    strip_ansi(stderr)
        .lines()
        .chain(strip_ansi(stdout).lines())
        .find(|line| is_diagnostic_header(line))
        .map(str::to_string)
}

/// Classify one run (D3). A launch failure arrives as `Err`.
pub(super) fn classify_run(run: &Result<RunCapture, String>, entry: &str, side: Side) -> Outcome {
    let capture = match run {
        Ok(capture) => capture,
        Err(launch) => return Outcome::Fault(format!("launch failed: {launch}")),
    };
    match capture.status {
        RunStatus::TimedOut(secs) => Outcome::Fault(format!("timeout after {secs}s")),
        RunStatus::Signalled(signal) => {
            let mut reason = format!("killed by signal {signal}");
            if side == Side::Selfhost && signal == SIGSEGV {
                reason.push_str("; set `ALBITE_STACK_SIZE` (e.g. `1G`) before running albite1");
            }
            Outcome::Fault(reason)
        }
        RunStatus::Exited(0) => {
            parse_count_line(&capture.stdout, entry).map_or_else(Outcome::Fault, Outcome::Accepted)
        }
        RunStatus::Exited(1) => first_error_line(&capture.stderr, &capture.stdout).map_or_else(
            || Outcome::Fault("exit 1 with no error header".to_string()),
            Outcome::Refused,
        ),
        RunStatus::Exited(code) => Outcome::Fault(format!("exit {code}")),
    }
}

/// `SIGSEGV` on every Unix the tool runs on.
const SIGSEGV: i32 = 11;

// Tests: table_tests.rs
#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
