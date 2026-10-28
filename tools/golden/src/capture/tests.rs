//! Unit tests for the shared capture path (ADR 5.10.26b).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{
    capture_compile_run, snapshot_bytes, snapshot_text, update_report, write_snapshot, CompileRun,
    UpdateOutcome, UpdateTally,
};
use crate::update::update_test;
use crate::{Category, Cli};

const INT_TRAP_EXPECTED: &[u8] =
    include_bytes!("../../../../tests/golden/compile/int_trap.expected");
const HELLO_EXPECTED: &[u8] = include_bytes!("../../../../tests/golden/compile/hello.expected");

/// Write an executable shell script at `dir/name`.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A stub `tungsten` whose `compile <file.tg>` writes `binary_body` as the
/// fixture's binary with `mode`, and whose `check` prints `check output`.
fn stub_compiler(dir: &Path, binary_body: &str, mode: &str) -> PathBuf {
    let body = format!(
        "case \"$1\" in\n\
         compile) out=\"${{2%.tg}}\"; printf '%s\\n' '#!/bin/sh' '{binary_body}' > \"$out\"; chmod {mode} \"$out\" ;;\n\
         check) echo 'check output' ;;\n\
         esac"
    );
    script(dir, "tungsten", &body)
}

fn fixture(dir: &Path) -> PathBuf {
    let tg = dir.join("main.tg");
    fs::write(&tg, "fn main() {}\n").unwrap();
    tg
}

fn cli(compiler: PathBuf, dir: &Path) -> Cli {
    Cli {
        category: None,
        update: true,
        compiler,
        dir: dir.to_path_buf(),
        no_clean: true,
    }
}

// ADR 5.10.26b AC1 — one serialization, every shape.
#[test]
fn a_stderr_only_run_serializes_to_the_int_trap_snapshot() {
    let bytes = snapshot_bytes(b"", b"error: integer overflow in +\n");
    assert_eq!(bytes.as_bytes(), INT_TRAP_EXPECTED);
}

#[test]
fn a_stdout_only_run_serializes_to_the_hello_snapshot() {
    let bytes = snapshot_bytes(b"hello from compiled binary\n", b"");
    assert_eq!(bytes.as_bytes(), HELLO_EXPECTED);
}

#[test]
fn stdout_comes_before_stderr_with_one_final_newline() {
    let bytes = snapshot_bytes(b"partial\n", b"error: trap\n\n  \n");
    assert_eq!(bytes, "partial\nerror: trap\n");
}

#[test]
fn an_empty_run_serializes_to_a_single_newline() {
    assert_eq!(snapshot_bytes(b"", b""), "\n");
    assert_eq!(snapshot_text(" \t\n"), "\n");
}

#[test]
fn invalid_utf8_is_decoded_lossily() {
    assert_eq!(snapshot_bytes(b"a\xffb", b""), "a\u{fffd}b\n");
}

// ADR 5.10.26b AC2 — an unchanged snapshot is not rewritten.
#[test]
fn an_unchanged_snapshot_is_not_touched_or_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.expected");
    fs::write(&path, INT_TRAP_EXPECTED).unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();

    let outcome = write_snapshot(&path, "error: integer overflow in +\n");

    assert_eq!(outcome, UpdateOutcome::Unchanged);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), old);
    assert_eq!(update_report(&path, outcome), None);
}

#[test]
fn a_changed_snapshot_is_written_and_names_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.expected");
    fs::write(&path, "\n").unwrap();

    let outcome = write_snapshot(&path, "new\n");

    assert_eq!(outcome, UpdateOutcome::Updated);
    assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
    let line = update_report(&path, outcome).expect("an update is reported");
    assert!(line.contains("Updated"), "{line}");
    assert!(line.contains(&path.display().to_string()), "{line}");
}

#[test]
fn a_missing_snapshot_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.expected");
    assert_eq!(write_snapshot(&path, "x\n"), UpdateOutcome::Updated);
    assert_eq!(fs::read_to_string(&path).unwrap(), "x\n");
}

#[test]
fn an_unwritable_snapshot_fails_and_is_not_reported() {
    let dir = tempfile::tempdir().unwrap();
    // A directory cannot be written as a file, whatever user runs the test.
    let path = dir.path().join("a_dir.expected");
    fs::create_dir(&path).unwrap();
    let outcome = write_snapshot(&path, "x\n");
    assert_eq!(outcome, UpdateOutcome::Failed);
    assert_eq!(update_report(&path, outcome), None);
}

#[test]
fn a_non_compile_update_goes_through_write_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let compiler = stub_compiler(dir.path(), "true", "755");
    let tg = fixture(dir.path());
    let expected = dir.path().join("main.expected");
    let cli = cli(compiler, dir.path());

    let first = update_test(&cli, &Category::Check, &tg, &expected);
    let second = update_test(&cli, &Category::Check, &tg, &expected);

    assert_eq!(first, UpdateOutcome::Updated);
    assert_eq!(second, UpdateOutcome::Unchanged);
    assert_eq!(fs::read_to_string(&expected).unwrap(), "check output\n");
}

#[test]
fn json_parity_has_nothing_to_update() {
    let dir = tempfile::tempdir().unwrap();
    let cli = cli(dir.path().join("unused"), dir.path());
    let outcome = update_test(&cli, &Category::JsonParity, Path::new(""), Path::new(""));
    assert_eq!(outcome, UpdateOutcome::Unchanged);
}

// ADR 5.10.26b D3 — which capture outcomes carry content.
#[test]
fn a_binary_that_exits_nonzero_still_ran() {
    let dir = tempfile::tempdir().unwrap();
    let compiler = stub_compiler(dir.path(), "echo out; echo err >&2; exit 3", "755");
    let tg = fixture(dir.path());

    let run = capture_compile_run(&compiler, &tg);

    assert_eq!(
        run,
        CompileRun::Ran {
            stdout: b"out\n".to_vec(),
            stderr: b"err\n".to_vec(),
        }
    );
    assert!(!tg.with_extension("").exists(), "the binary is removed");
}

#[test]
fn a_failed_compile_is_not_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let compiler = script(dir.path(), "tungsten", "exit 1");
    let tg = fixture(dir.path());
    assert_eq!(
        capture_compile_run(&compiler, &tg),
        CompileRun::CompileFailed
    );
}

#[test]
fn an_unspawnable_compiler_is_a_failed_compile() {
    let dir = tempfile::tempdir().unwrap();
    let tg = fixture(dir.path());
    let missing = dir.path().join("no-such-compiler");
    assert_eq!(
        capture_compile_run(&missing, &tg),
        CompileRun::CompileFailed
    );
}

#[test]
fn a_compile_that_leaves_no_binary_is_not_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let compiler = script(dir.path(), "tungsten", "exit 0");
    let tg = fixture(dir.path());
    assert_eq!(capture_compile_run(&compiler, &tg), CompileRun::NoBinary);
}

#[test]
fn a_binary_that_cannot_be_spawned_is_a_run_failure() {
    let dir = tempfile::tempdir().unwrap();
    let compiler = stub_compiler(dir.path(), "echo never", "644");
    let tg = fixture(dir.path());
    assert!(matches!(
        capture_compile_run(&compiler, &tg),
        CompileRun::RunFailed(_)
    ));
}

#[test]
fn a_failed_compile_update_keeps_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let compiler = stub_compiler(dir.path(), "echo never", "644");
    let tg = fixture(dir.path());
    let expected = dir.path().join("main.expected");
    fs::write(&expected, INT_TRAP_EXPECTED).unwrap();

    let outcome = update_test(
        &cli(compiler, dir.path()),
        &Category::Compile,
        &tg,
        &expected,
    );

    assert_eq!(outcome, UpdateOutcome::Failed);
    assert_eq!(fs::read(&expected).unwrap(), INT_TRAP_EXPECTED);
}

#[test]
fn the_update_tally_counts_each_outcome() {
    let mut tally = UpdateTally::default();
    for outcome in [
        UpdateOutcome::Updated,
        UpdateOutcome::Unchanged,
        UpdateOutcome::Unchanged,
        UpdateOutcome::Failed,
    ] {
        tally.record(outcome);
    }
    assert_eq!(
        tally,
        UpdateTally {
            updated: 1,
            unchanged: 2,
            failed: 1,
        }
    );
    assert_eq!(tally.summary_line(), "  1 updated, 2 unchanged, 1 failed\n");
}

#[test]
fn the_update_summary_omits_zero_failures() {
    let tally = UpdateTally {
        updated: 0,
        unchanged: 6,
        failed: 0,
    };
    assert_eq!(tally.summary_line(), "  0 updated, 6 unchanged\n");
}
