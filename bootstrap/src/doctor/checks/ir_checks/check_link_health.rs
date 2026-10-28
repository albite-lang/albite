//! `albite doctor check link health` — verify compiled binary properties.
//!
//! Checks that a compiled Tungsten binary has the expected properties:
//! - Its program body gets a deep stack. On macOS that is the `LC_MAIN`
//!   stack size ld64 writes (at least 64 MB). On Linux glibc sizes the main
//!   thread from `RLIMIT_STACK` and ignores the ELF's stack field, so the
//!   check is instead that the binary defines `__albite_run_on_stack`, the
//!   runtime entry that runs the body on a thread it sizes (ADR 19.10.26c D5).
//!   User links strip unreferenced sections (ADR 6.10.26i), so the symbol
//!   survives only if `main` calls it.
//! - Binary is executable and has a valid entrypoint
//!
//! Cost 1: shells out to platform tools (otool/nm), no elaboration.

use std::path::Path;
use std::process::{Command, ExitCode};

/// Expected stack size in bytes (64 MB).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // Reason: only macOS reads a linked stack size; the constant stays pinned by its test everywhere
const EXPECTED_STACK_SIZE: u64 = 64 * 1024 * 1024;

/// The runtime entry a Linux binary must define (ADR 19.10.26c D2).
const RUN_ON_STACK_SYMBOL: &str = "__albite_run_on_stack";

/// The argument the execute step passes: one both the bootstrap `tungsten`
/// and the self-hosted `albite1` answer with exit 0.
const EXECUTE_PROBE_ARG: &str = "--version";

/// Run the link-health check on a compiled binary.
pub fn cmd_check_link_health(binary: &Path, verbose: bool) -> ExitCode {
    let mut ok = true;

    // 1. Check binary exists and is executable
    if !binary.exists() {
        eprintln!("error: binary not found: {}", binary.display());
        return ExitCode::FAILURE;
    }

    if verbose {
        eprintln!("Checking link health: {}", binary.display());
    }

    // 2. Check the body's stack
    ok &= check_body_stack(binary, verbose);

    // 3. Check binary runs. `--version`, not `commands`: `albite1` — this
    // check's own example — has no `commands` subcommand and reads the word
    // as a file name, exiting 3 (ADR 19.10.26c P0.1).
    match Command::new(binary).arg(EXECUTE_PROBE_ARG).output() {
        Ok(output) if output.status.success() => {
            if verbose {
                eprintln!("  binary executes successfully ✓");
            }
        }
        Ok(output) => {
            eprintln!("FAIL: binary exited with status {}", output.status);
            ok = false;
        }
        Err(e) => {
            eprintln!("FAIL: could not execute binary: {}", e);
            ok = false;
        }
    }

    if ok {
        eprintln!("link-health: all checks passed ✓");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// macOS: the main thread's linked stack size must be at least 64 MB.
#[cfg(target_os = "macos")]
fn check_body_stack(binary: &Path, verbose: bool) -> bool {
    let mut ok = true;
    match check_stack_size(binary, verbose) {
        Ok(size) => {
            if size < EXPECTED_STACK_SIZE {
                eprintln!(
                    "FAIL: stack size is {} bytes ({:.1} MB), expected {} bytes ({} MB)",
                    size,
                    size as f64 / (1024.0 * 1024.0),
                    EXPECTED_STACK_SIZE,
                    EXPECTED_STACK_SIZE / (1024 * 1024)
                );
                eprintln!(
                    "hint: this usually means the linker ignored -stack_size (ld64.lld does this)"
                );
                ok = false;
            } else if verbose {
                eprintln!(
                    "  stack size: {} bytes ({} MB) ✓",
                    size,
                    size / (1024 * 1024)
                );
            }
        }
        Err(e) => {
            eprintln!("warning: could not check stack size: {}", e);
            // Non-fatal — the tool may not be available
        }
    }
    ok
}

/// What `nm` says about [`RUN_ON_STACK_SYMBOL`] in a binary.
#[derive(Debug, PartialEq, Eq)]
enum RuntimeEntryVerdict {
    /// Defined: `main` runs the body on the runtime-sized stack.
    Defined,
    /// The binary has symbols but not this one: linked before ADR 19.10.26c,
    /// or with a `main` that bypasses the entry.
    Missing,
    /// No symbol table (a stripped release): nothing to read either way.
    NoSymbols,
}

/// Classify `nm`'s output for a binary. A defined symbol is a line whose
/// type letter is `T`/`t` and whose name is exactly the entry; an undefined
/// reference (`U`) does not count. `nm` reports a stripped binary as
/// "no symbols" on stderr.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // Reason: only the Linux arm runs nm; the classifier is pure and tested on every host
fn classify_runtime_entry(nm_stdout: &str, nm_stderr: &str) -> RuntimeEntryVerdict {
    let defined = nm_stdout.lines().any(|line| {
        let mut fields = line.split_whitespace().rev();
        let (Some(name), Some(kind)) = (fields.next(), fields.next()) else {
            return false;
        };
        name == RUN_ON_STACK_SYMBOL && matches!(kind, "T" | "t")
    });
    if defined {
        RuntimeEntryVerdict::Defined
    } else if nm_stdout.trim().is_empty() || nm_stderr.contains("no symbols") {
        RuntimeEntryVerdict::NoSymbols
    } else {
        RuntimeEntryVerdict::Missing
    }
}

/// Linux: the binary must define the runtime stack entry; a stripped binary
/// only warns (ADR 19.10.26c D5).
#[cfg(target_os = "linux")]
fn check_body_stack(binary: &Path, verbose: bool) -> bool {
    let output = match Command::new("nm").arg(binary).output() {
        Ok(output) => output,
        Err(e) => {
            eprintln!(
                "warning: could not check the runtime stack entry: nm: {}",
                e
            );
            return true;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match classify_runtime_entry(&stdout, &stderr) {
        RuntimeEntryVerdict::Defined => {
            if verbose {
                eprintln!("  runtime stack entry: {RUN_ON_STACK_SYMBOL} defined ✓");
            }
            true
        }
        RuntimeEntryVerdict::Missing => {
            eprintln!("FAIL: {RUN_ON_STACK_SYMBOL} is not defined in the binary");
            eprintln!(
                "hint: a binary linked before ADR 19.10.26c runs its body on the shell's \
                 RLIMIT_STACK (usually 8 MB); rebuild it, rebuilding libalbite_core.a first"
            );
            false
        }
        RuntimeEntryVerdict::NoSymbols => {
            eprintln!(
                "warning: no symbol table, so {RUN_ON_STACK_SYMBOL} cannot be checked (stripped binary?)"
            );
            true
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn check_body_stack(_binary: &Path, _verbose: bool) -> bool {
    eprintln!("warning: stack check not implemented for this platform");
    true
}

/// Read the stack size from a compiled binary using platform tools.
#[cfg(target_os = "macos")]
fn check_stack_size(binary: &Path, verbose: bool) -> Result<u64, String> {
    // On macOS, use `otool -l` to find LC_MAIN's stacksize field
    let output = Command::new("otool")
        .args(["-l"])
        .arg(binary)
        .output()
        .map_err(|e| format!("otool not found: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    if verbose {
        // Show the relevant section
        for line in stdout.lines() {
            if line.contains("stacksize") {
                eprintln!("  otool: {}", line.trim());
            }
        }
    }

    // Parse LC_MAIN section for stacksize
    let mut in_lc_main = false;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.contains("LC_MAIN") {
            in_lc_main = true;
            continue;
        }
        if in_lc_main && trimmed.starts_with("cmd ") {
            // Hit the next load command — LC_MAIN didn't have stacksize
            break;
        }
        if in_lc_main && trimmed.starts_with("stacksize") {
            // Parse "stacksize 67108864" or "stacksize 0x4000000"
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let val = parts[1];
                let size = if val.starts_with("0x") || val.starts_with("0X") {
                    u64::from_str_radix(&val[2..], 16).map_err(|e| format!("parse hex: {}", e))?
                } else {
                    val.parse::<u64>()
                        .map_err(|e| format!("parse decimal: {}", e))?
                };
                return Ok(size);
            }
        }
    }

    // No LC_MAIN stacksize found — binary uses default
    Ok(8 * 1024 * 1024) // Default 8MB
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 19.10.26c P0.1: `albite1 commands` exits 3, so the probe must
    /// be an argument the self-host answers.
    #[test]
    fn the_execute_probe_is_one_tungsten1_answers() {
        assert_eq!(EXECUTE_PROBE_ARG, "--version");
    }

    #[test]
    fn test_expected_stack_size_is_64mb() {
        assert_eq!(EXPECTED_STACK_SIZE, 64 * 1024 * 1024);
    }

    /// ADR 19.10.26c AC 5: a binary linking the entry passes.
    #[test]
    fn a_defined_runtime_entry_passes() {
        let nm = "0000000000401000 T main\n00000000004a2b30 T __albite_run_on_stack\n";
        assert_eq!(classify_runtime_entry(nm, ""), RuntimeEntryVerdict::Defined);
        let local = "00000000004a2b30 t __albite_run_on_stack\n";
        assert_eq!(
            classify_runtime_entry(local, ""),
            RuntimeEntryVerdict::Defined
        );
    }

    /// ADR 19.10.26c AC 5: a binary built before the change — symbols, but
    /// not the entry, or only an undefined reference to it — fails.
    #[test]
    fn a_binary_without_the_entry_is_missing() {
        let before = "0000000000401000 T main\n0000000000401100 T __albite_inner_main\n";
        assert_eq!(
            classify_runtime_entry(before, ""),
            RuntimeEntryVerdict::Missing
        );
        let undefined = "                 U __albite_run_on_stack\n0000000000401000 T main\n";
        assert_eq!(
            classify_runtime_entry(undefined, ""),
            RuntimeEntryVerdict::Missing
        );
        let lookalike = "0000000000401000 T __albite_run_on_stack_v2\n";
        assert_eq!(
            classify_runtime_entry(lookalike, ""),
            RuntimeEntryVerdict::Missing
        );
    }

    /// A path that is not there fails before anything is spawned.
    #[test]
    fn a_missing_binary_fails() {
        let missing = Path::new("/nonexistent/tungsten1-19-10-26c");
        assert_eq!(cmd_check_link_health(missing, true), ExitCode::FAILURE);
    }

    /// The execute probe's failure fails the check on every host:
    /// `false --version` exits 1.
    #[test]
    fn a_binary_that_does_not_execute_cleanly_fails() {
        assert_eq!(
            cmd_check_link_health(Path::new("/usr/bin/false"), true),
            ExitCode::FAILURE
        );
    }

    /// ADR 19.10.26c D5, on a system binary that is not a Tungsten program.
    /// Linux: `/usr/bin/true` is stripped, so the entry check only warns, and
    /// `true --version` exits 0, so the check passes. macOS: `true` links no
    /// 64 MB `LC_MAIN` stack, so the stack arm fails it.
    #[test]
    fn a_system_binary_runs_every_arm_of_the_check() {
        let verdict = cmd_check_link_health(Path::new("/usr/bin/true"), true);
        #[cfg(target_os = "linux")]
        assert_eq!(verdict, ExitCode::SUCCESS);
        #[cfg(target_os = "macos")]
        assert_eq!(verdict, ExitCode::FAILURE);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = verdict;
    }

    /// ADR 19.10.26c D5: a stripped binary warns rather than fails.
    #[test]
    fn a_stripped_binary_has_no_symbols() {
        assert_eq!(
            classify_runtime_entry("", "nm: albite1: no symbols\n"),
            RuntimeEntryVerdict::NoSymbols
        );
        assert_eq!(
            classify_runtime_entry("", ""),
            RuntimeEntryVerdict::NoSymbols
        );
    }
}
