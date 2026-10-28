//! `albite diff bootstrap-selfhost-check` — compare the bootstrap compiler and
//! the self-hosted compiler (albite1) on the same source.
//!
//! Runs the bootstrap (this binary) and a self-host binary (albite1) on the same file in check
//! mode, then compares error output. Cost 3+3 (two elaboration passes).
//! See ADR 20.5.26a. Several paths, or a directory, select the table mode
//! (ADR 3.10.26a): one row per `.tg` entry with each compiler's verdict.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::time::{Duration, Instant};

mod table;
mod table_mode;
mod table_verdict;

use table::{RunCapture, RunStatus};

/// What the command needs besides its paths (ADR 3.10.26a D2). The bootstrap
/// executable is injected — `main` passes `current_exe()` — because under
/// `cargo test` the current executable is the libtest harness, not a compiler.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub bootstrap_exe: PathBuf,
    pub selfhost_binary: PathBuf,
    pub timeout: Duration,
    pub verbose: bool,
}

/// Entry point for `albite diff bootstrap-selfhost-check <paths>... --selfhost-binary <path>`.
/// One file argument keeps the original single-file report; anything else is
/// table mode.
pub fn cmd_diff_bootstrap_selfhost_check(paths: &[PathBuf], run: &RunOptions) -> ExitCode {
    match paths {
        [file] if !file.is_dir() => single_file(file, run),
        _ => table_mode::cmd_table_mode(paths, run),
    }
}

/// Print why a missing input stopped the run.
fn report_missing(missing: &MissingInput, selfhost_binary: &Path) {
    eprint!("{}", missing_message(missing, selfhost_binary));
}

/// Why a missing input stopped the run, with the build hint for a missing
/// self-host binary.
fn missing_message(missing: &MissingInput, selfhost_binary: &Path) -> String {
    match missing {
        MissingInput::SourceFile(path) => {
            format!("error: source file not found: {}\n", path.display())
        }
        MissingInput::SelfhostBinary => format!(
            "error: self-host binary not found: {}\n  \
             hint: build with `make devcontainer-self-compile-fast`\n",
            selfhost_binary.display()
        ),
    }
}

/// The original single-file report (ADR 20.5.26a), output and exit codes
/// unchanged; only the timeout is new.
fn single_file(file: &Path, run: &RunOptions) -> ExitCode {
    let selfhost_binary = run.selfhost_binary.as_path();
    let verbose = run.verbose;
    if let Err(missing) = preflight(std::slice::from_ref(&file.to_path_buf()), selfhost_binary) {
        report_missing(&missing, selfhost_binary);
        return ExitCode::FAILURE;
    }

    println!(
        "Comparing bootstrap vs self-host check on {}...\n",
        file.display()
    );

    // --- Run the bootstrap (this binary) ---
    let bootstrap_result = run_check_single(&run.bootstrap_exe, file, run);
    let (bootstrap_exit, bootstrap_errors) = match bootstrap_result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to run the bootstrap: {e}");
            return ExitCode::FAILURE;
        }
    };

    // --- Run the self-host binary (albite1) ---
    let selfhost_result = run_check_single(selfhost_binary, file, run);
    let (selfhost_exit, selfhost_errors) = match selfhost_result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to run the self-host binary: {e}");
            eprintln!("  hint: set `ALBITE_STACK_SIZE` (e.g. `1G`) before running albite1");
            return ExitCode::FAILURE;
        }
    };

    // --- Compare ---
    println!("Results:");
    println!(
        "  bootstrap: exit={}, errors={}",
        bootstrap_exit,
        bootstrap_errors.len()
    );
    println!(
        "  self-host: exit={}, errors={}",
        selfhost_exit,
        selfhost_errors.len()
    );

    report_divergence(&bootstrap_errors, &selfhost_errors, verbose)
}

/// An input the comparison cannot start without.
#[derive(Debug, PartialEq, Eq)]
enum MissingInput {
    SourceFile(PathBuf),
    SelfhostBinary,
}

/// Every input must exist before either compiler is spawned. Returning *which*
/// one is missing (rather than printing and bailing inline) is what makes the
/// guards distinguishable in a test — an exit code alone cannot tell them
/// apart, since every path fails. Paths are checked in order, before the binary.
fn preflight(paths: &[PathBuf], selfhost_binary: &Path) -> Result<(), MissingInput> {
    if let Some(missing) = paths.iter().find(|path| !path.exists()) {
        return Err(MissingInput::SourceFile(missing.clone()));
    }
    if !selfhost_binary.exists() {
        return Err(MissingInput::SelfhostBinary);
    }
    Ok(())
}

/// Entries of the self-host-only list printed before the "and N more" tail.
const SELFHOST_LIST_CAP: usize = 20;
/// Entries of the bootstrap-only list printed under `--verbose`.
const BOOTSTRAP_LIST_CAP: usize = 10;

/// Report how the two error sets differ and pick the exit code: agreement (no
/// errors at all, or the identical set) succeeds; anything else is a
/// divergence worth a non-zero exit.
fn report_divergence(
    bootstrap_errors: &[String],
    selfhost_errors: &[String],
    verbose: bool,
) -> ExitCode {
    let (text, code) = render_report(bootstrap_errors, selfhost_errors, verbose);
    print!("{text}");
    code
}

/// Build the comparison report as text plus its exit code.
///
/// Rendering to a `String` instead of printing inline is deliberate: the
/// verdict, the list caps, and the `--verbose` gate are all *output* decisions,
/// so a test that only inspects the exit code cannot see any of them.
fn render_report(
    bootstrap_errors: &[String],
    selfhost_errors: &[String],
    verbose: bool,
) -> (String, ExitCode) {
    if bootstrap_errors.is_empty() && selfhost_errors.is_empty() {
        return (
            "\n✓ Both compilers agree: 0 errors\n".to_string(),
            ExitCode::SUCCESS,
        );
    }

    if bootstrap_errors == selfhost_errors {
        return (
            format!(
                "\n✓ Both compilers produce identical errors ({})\n",
                bootstrap_errors.len()
            ),
            ExitCode::SUCCESS,
        );
    }

    // Errors unique to the self-host binary are the regression signal; those
    // unique to the bootstrap are usually a self-host improvement or a
    // deliberate behavioural difference, so they only render under --verbose.
    let selfhost_only = errors_missing_from(selfhost_errors, bootstrap_errors);
    let bootstrap_only = errors_missing_from(bootstrap_errors, selfhost_errors);

    // Writing into a String is infallible, so each `write!` result is dropped.
    let mut out = String::new();
    if !selfhost_only.is_empty() {
        let _ = writeln!(
            out,
            "\n⚠ {} error(s) only in self-host (potential codegen regressions):",
            selfhost_only.len()
        );
        out.push_str(&numbered(&selfhost_only, SELFHOST_LIST_CAP));
        if selfhost_only.len() > SELFHOST_LIST_CAP {
            let _ = writeln!(
                out,
                "  ... and {} more",
                selfhost_only.len() - SELFHOST_LIST_CAP
            );
        }
    }

    if !bootstrap_only.is_empty() && verbose {
        let _ = writeln!(
            out,
            "\n  {} error(s) only in bootstrap (self-host may handle differently):",
            bootstrap_only.len()
        );
        out.push_str(&numbered(&bootstrap_only, BOOTSTRAP_LIST_CAP));
    }

    let _ = writeln!(
        out,
        "\nSummary: bootstrap={} errors, self-host={} errors, self-host-only={}, bootstrap-only={}",
        bootstrap_errors.len(),
        selfhost_errors.len(),
        selfhost_only.len(),
        bootstrap_only.len()
    );

    (out, ExitCode::FAILURE)
}

/// The entries of `theirs` that `mine` does not also contain.
fn errors_missing_from<'a>(theirs: &'a [String], mine: &[String]) -> Vec<&'a String> {
    theirs.iter().filter(|e| !mine.contains(e)).collect()
}

/// A 1-based numbered list, capped at `limit` entries. Whether a truncated list
/// gets an "and N more" tail is the caller's call — only the self-host-only
/// list (the regression signal) carries one.
fn numbered(errors: &[&String], limit: usize) -> String {
    let mut out = String::new();
    for (i, err) in errors.iter().enumerate().take(limit) {
        let _ = writeln!(out, "  {}. {}", i + 1, err);
    }
    out
}

/// True if a captured output line looks like a compiler error. Both spellings
/// count: `error[E0001]: …` (coded) and a bare `error: …` (uncoded).
fn is_error_line(line: &str) -> bool {
    line.contains("error[") || line.contains("error:")
}

/// Keep just the error lines from a run's captured streams, stderr first.
fn collect_error_lines(stderr: &str, stdout: &str) -> Vec<String> {
    stderr
        .lines()
        .chain(stdout.lines())
        .filter(|line| is_error_line(line))
        .map(str::to_string)
        .collect()
}

/// Single-file mode's view of a run: `<binary> check <file>` from the
/// caller's directory, reduced to the exit code and error lines. An expiry
/// is reported as a launch failure is (ADR 3.10.26a D6).
fn run_check_single(
    binary: &Path,
    file: &Path,
    run: &RunOptions,
) -> Result<(i32, Vec<String>), String> {
    if run.verbose {
        eprintln!("  running: {} check {}", binary.display(), file.display());
    }
    let args = [OsStr::new("check"), file.as_os_str()];
    let capture = run_check(binary, &args, None, run.timeout)?;
    let exit_code = match capture.status {
        RunStatus::Exited(code) => code,
        RunStatus::Signalled(_) => -1,
        RunStatus::TimedOut(secs) => return Err(format!("timed out after {secs}s")),
    };
    Ok((
        exit_code,
        collect_error_lines(&capture.stderr, &capture.stdout),
    ))
}

/// Spawn `<binary> <args>` with `working_dir` as the CHILD's working
/// directory (never this process's — ADR 3.10.26a D2), drain both pipes on
/// threads, and kill and reap the child at `timeout` (D6). The poll loop is
/// `diff::exec::run_with_timeout`'s, written again because that one folds
/// every non-zero exit into one status, and table mode must tell exit 1 from
/// any other exit and from a signal.
fn run_check(
    binary: &Path,
    args: &[&OsStr],
    working_dir: Option<&Path>,
    timeout: Duration,
) -> Result<RunCapture, String> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = working_dir {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("{}: {e}", binary.display()))?;

    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut buf).ok();
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("wait failed: {e}"))? {
            break status_of(status);
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            break RunStatus::TimedOut(timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(RunCapture {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// An exit code when there is one, else the terminating signal.
fn status_of(status: ExitStatus) -> RunStatus {
    if let Some(code) = status.code() {
        return RunStatus::Exited(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(signal) = status.signal() {
            return RunStatus::Signalled(signal);
        }
    }
    RunStatus::Exited(-1)
}

// Tests: bootstrap_selfhost/tests.rs, and the stub-compiler end-to-end
// tests in bootstrap_selfhost/stub_tests.rs (ADR 3.10.26a AC 4).
#[cfg(test)]
#[path = "bootstrap_selfhost/tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "bootstrap_selfhost/stub_tests.rs"]
mod stub_tests;
