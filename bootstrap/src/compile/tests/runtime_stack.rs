//! ADR 19.10.26c AC 3 — a compiled program's body runs on a stack the
//! runtime sizes, not the shell's `RLIMIT_STACK`, and `ALBITE_STACK_SIZE`
//! overrides it.
//!
//! One fixture recurses a million calls deep, building a chain on the way
//! back up so the call is not in tail position and LLVM cannot turn it into a
//! loop. It is compiled in-process (the bootstrap's pinned code-generation
//! level) without `--debug-info`, which would skip the overflow handler. Each
//! arm runs the binary under `sh -c 'ulimit -s N; exec …'` so the shell's
//! limit is the one the arm names.

use super::mono_depot_externs::ensure_runtime_staticlib_beside_test_exe;
use crate::compile::{cmd_compile, CompileFlags};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};
use std::sync::OnceLock;
use tempfile::TempDir;

const DEEP_CHAIN_SOURCE: &str = r"
type Chain = Link(Nat, Chain) | End

#[partial]
fn build_chain(n: Nat) -> Chain {
    if n == 0 { End } else { Link(n, build_chain(n - 1)) }
}

fn chain_length(chain: Chain, acc: Nat) -> Nat {
    match chain {
        End => acc,
        Link(_, rest) => chain_length(rest, acc + 1),
    }
}

fn main() -> Nat {
    chain_length(build_chain(1000000), 0)
}
";

/// The fixture, compiled once for every arm. The `TempDir` is kept alive for
/// the whole test binary so the path stays valid.
fn deep_chain_binary() -> &'static Path {
    static BINARY: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    let (_, binary) = BINARY.get_or_init(|| {
        ensure_runtime_staticlib_beside_test_exe();
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("deep_chain.tg");
        std::fs::write(&source, DEEP_CHAIN_SOURCE).unwrap();
        let binary = dir.path().join("deep_chain");
        let flags = CompileFlags {
            max_errors: 20,
            codegen_jobs: 1,
            ..CompileFlags::default()
        };
        assert_eq!(
            cmd_compile(&source, Some(&binary), &flags),
            ExitCode::SUCCESS,
            "the deep-chain fixture must compile"
        );
        (dir, binary)
    });
    binary
}

/// Run the fixture under a shell whose stack limit `ulimit_prefix` sets, with
/// `ALBITE_STACK_SIZE` set to `stack_size` or removed.
fn run_deep_chain(ulimit_prefix: &str, stack_size: Option<&str>) -> Output {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("{ulimit_prefix}; exec \"$0\""))
        .arg(deep_chain_binary());
    match stack_size {
        Some(size) => command.env("ALBITE_STACK_SIZE", size),
        None => command.env_remove("ALBITE_STACK_SIZE"),
    };
    command.env_remove("ALBITE_NO_SIGNAL_HANDLER");
    command.output().expect("sh runs")
}

/// The shell's usual 8 MB limit.
const SHELL_8_MB: &str = "ulimit -s 8192";
/// As much stack as the shell may have: `unlimited` on Linux, the hard limit
/// on macOS, which refuses `unlimited`.
const SHELL_MAXIMUM: &str = "ulimit -s unlimited 2>/dev/null || ulimit -s \"$(ulimit -Hs)\"";

/// The `Stack size:` the overflow report printed, in whole MB.
fn reported_stack_mb(stderr: &str) -> Option<u64> {
    let line = stderr.lines().find(|l| l.contains("Stack size:"))?;
    let mb = line.split('(').nth(1)?.split_whitespace().next()?;
    mb.parse().ok()
}

/// No override, an 8 MB shell: the default runtime stack holds a million
/// frames the shell's would not.
#[test]
fn the_default_runtime_stack_outlives_the_shells_8_mb() {
    let output = run_deep_chain(SHELL_8_MB, None);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1000000");
}

/// The must-fail twin: `ALBITE_STACK_SIZE=8M` under the largest shell
/// limit overflows, which it can only do if the runtime honoured 8M.
#[test]
fn an_8m_override_overflows_even_under_the_largest_shell_limit() {
    let output = run_deep_chain(SHELL_MAXIMUM, Some("8M"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(134), "{output:?}");
    assert!(stderr.contains("Stack overflow detected"), "{stderr}");
}

/// `ALBITE_STACK_SIZE=12M` under an 8 MB shell: the report says about
/// 12 MB, so the body ran on the spawned thread, not the main one.
#[test]
fn the_overflow_report_names_the_spawned_threads_size() {
    let output = run_deep_chain(SHELL_8_MB, Some("12M"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(134), "{output:?}");
    // glibc reports a little under the request; the handler rounds down.
    let mb = reported_stack_mb(&stderr).unwrap_or_else(|| panic!("no Stack size: {stderr}"));
    assert!((11..=12).contains(&mb), "reported {mb} MB: {stderr}");
}

/// A malformed override exits 2, naming the variable, before the body runs.
#[test]
fn a_malformed_override_exits_2_naming_the_variable() {
    let output = run_deep_chain(SHELL_8_MB, Some("banana"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr.contains("ALBITE_STACK_SIZE"), "{stderr}");
    assert!(
        output.stdout.is_empty(),
        "the body must not run: {output:?}"
    );
}

#[test]
fn the_reported_size_is_read_from_the_overflow_banner() {
    let banner = "  Stack size:     12582912 bytes (12 MB)\n";
    assert_eq!(reported_stack_mb(banner), Some(12));
    assert_eq!(reported_stack_mb("no banner"), None);
}
