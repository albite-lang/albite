//! ADR 20.10.26e AC 3 — a compiled program honours both spellings of a
//! runtime setting: `ALBITE_ARENA=bump` and the legacy `TUNGSTEN_ARENA=bump`
//! give the same exit code and stdout, and only the legacy run prints the
//! one-line notice.

use super::mono_depot_externs::ensure_runtime_staticlib_beside_test_exe;
use crate::compile::{cmd_compile, CompileFlags};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};
use std::sync::OnceLock;
use tempfile::TempDir;

/// Allocates (a list), so the arena mode is actually exercised.
const ALLOCATING_SOURCE: &str = r"
type Chain = Link(Nat, Chain) | End

#[partial]
fn build_chain(n: Nat) -> Chain {
    if n == 0 { End } else { Link(n, build_chain(n - 1)) }
}

fn chain_sum(chain: Chain, acc: Nat) -> Nat {
    match chain {
        End => acc,
        Link(n, rest) => chain_sum(rest, acc + n),
    }
}

fn main() -> Nat {
    chain_sum(build_chain(100), 0)
}
";

fn allocating_binary() -> &'static Path {
    static BINARY: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    let (_, binary) = BINARY.get_or_init(|| {
        ensure_runtime_staticlib_beside_test_exe();
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("allocating.tg");
        std::fs::write(&source, ALLOCATING_SOURCE).unwrap();
        let binary = dir.path().join("allocating");
        let flags = CompileFlags {
            max_errors: 20,
            codegen_jobs: 1,
            ..CompileFlags::default()
        };
        assert_eq!(
            cmd_compile(&source, Some(&binary), &flags),
            ExitCode::SUCCESS,
            "the allocating fixture must compile"
        );
        (dir, binary)
    });
    binary
}

/// Run the fixture with exactly one spelling of the arena setting set.
fn run_with_arena_under(variable: &str) -> Output {
    let current = albite_runtime::settings::name("ARENA");
    let legacy = albite_runtime::settings::legacy_name("ARENA");
    Command::new(allocating_binary())
        .env_remove(&current)
        .env_remove(&legacy)
        .env(variable, "bump")
        .output()
        .expect("the fixture runs")
}

// 20.10.26e AC3
#[test]
fn a_compiled_program_honours_both_spellings_and_only_the_legacy_one_warns() {
    let current = run_with_arena_under(&albite_runtime::settings::name("ARENA"));
    let legacy = run_with_arena_under(&albite_runtime::settings::legacy_name("ARENA"));
    assert_eq!(
        current.status.code(),
        legacy.status.code(),
        "{current:?} / {legacy:?}"
    );
    assert_eq!(current.stdout, legacy.stdout);
    assert_eq!(String::from_utf8_lossy(&current.stdout).trim(), "5050");
    let notice = albite_runtime::settings::legacy_notice("ARENA");
    let current_stderr = String::from_utf8_lossy(&current.stderr);
    let legacy_stderr = String::from_utf8_lossy(&legacy.stderr);
    assert!(!current_stderr.contains(&notice), "{current_stderr}");
    assert_eq!(legacy_stderr.matches(&notice).count(), 1, "{legacy_stderr}");
}
