//! `tungsten` — the compiler's pre-rename command, kept for one release
//! (ADR 20.10.26d D1).
//!
//! It prints one deprecation line (none under `--error-format json`) and
//! hands its arguments to the `albite` binary in its **own directory** —
//! `current_exe()`'s parent, never a `PATH` lookup, so a `tungsten` always
//! runs the `albite` that was built or packaged beside it.

mod notice;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// What a shell reports for a command it could not find; returned when there
/// is no `albite` beside this binary.
const EXIT_NO_SIBLING: u8 = 127;
/// What a shell reports for a command it found but could not run.
const EXIT_CANNOT_RUN: u8 = 126;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(&args, std::env::current_exe(), cfg!(unix)))
}

/// The whole shim over injected inputs — the arguments, where this executable
/// lives, and whether to exec — so a test can drive it without being replaced.
fn run(args: &[String], exe: std::io::Result<PathBuf>, replace_process: bool) -> u8 {
    if let Some(line) = notice::notice(args) {
        eprintln!("{line}");
    }
    let sibling = match exe {
        Ok(exe) => sibling_compiler(&exe),
        Err(error) => {
            eprintln!("error: cannot locate the `tungsten` executable to find `albite` beside it: {error}");
            return EXIT_NO_SIBLING;
        }
    };
    forward(&sibling, args, replace_process)
}

/// The `albite` binary in the same directory as `exe`.
fn sibling_compiler(exe: &Path) -> PathBuf {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("albite{}", std::env::consts::EXE_SUFFIX))
}

/// Run `sibling` with `args` and return the exit code to report.
///
/// `replace_process` execs it, so the compiler inherits this process outright
/// and the code is never seen here; otherwise it is spawned and waited on, and
/// its code forwarded. A missing sibling is [`EXIT_NO_SIBLING`], naming the
/// path that was looked for.
fn forward(sibling: &Path, args: &[String], replace_process: bool) -> u8 {
    if !sibling.is_file() {
        eprintln!(
            "error: `tungsten` forwards to `albite`, but there is no `albite` at {}",
            sibling.display()
        );
        return EXIT_NO_SIBLING;
    }
    let mut command = Command::new(sibling);
    command.args(args);
    if replace_process {
        return exec(&mut command, sibling);
    }
    match command.status() {
        Ok(status) => exit_code_of(status.code()),
        Err(error) => {
            eprintln!("error: cannot run {}: {error}", sibling.display());
            EXIT_CANNOT_RUN
        }
    }
}

/// Replace this process with `command`; returns only when that fails.
#[cfg(unix)]
fn exec(command: &mut Command, sibling: &Path) -> u8 {
    use std::os::unix::process::CommandExt;
    let error = command.exec();
    eprintln!("error: cannot exec {}: {error}", sibling.display());
    EXIT_CANNOT_RUN
}

#[cfg(not(unix))]
fn exec(command: &mut Command, sibling: &Path) -> u8 {
    match command.status() {
        Ok(status) => exit_code_of(status.code()),
        Err(error) => {
            eprintln!("error: cannot run {}: {error}", sibling.display());
            EXIT_CANNOT_RUN
        }
    }
}

/// A child's exit code as this process's; a signal death (no code) reads as 1.
fn exit_code_of(code: Option<i32>) -> u8 {
    code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1)
}
