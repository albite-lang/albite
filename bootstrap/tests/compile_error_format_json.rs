//! `albite compile --error-format json` through the binary (ADR 30.9.26j
//! AC5).
//!
//! What only a spawned process can show: which stream each byte reaches and
//! the exit code a shell sees. `compile` writes its records to **stderr**,
//! because its stdout is its own product (D5). Fixtures are written into a
//! tempdir, so the assertions hold in a mutation sweep's copied workspace too.
#![cfg(feature = "codegen")]

use std::path::{Path, PathBuf};
use std::process::Command;

const CLEAN: &str = "fn main() -> Nat {\n    0\n}\n";
const TYPE_ERROR: &str = "fn main() -> Nat { true }\n";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Every stderr line, parsed — panics on a line that is not one JSON object.
    fn records(&self) -> Vec<serde_json::Value> {
        self.stderr
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("stderr line is not JSON ({e}): {line}"))
            })
            .collect()
    }

    /// The one record on stderr, asserting there is exactly one line.
    fn only_record(&self) -> serde_json::Value {
        let records = self.records();
        assert_eq!(records.len(), 1, "stderr:\n{}", self.stderr);
        records.into_iter().next().unwrap()
    }
}

/// The binary under test links `libalbite_core.a` from its own directory,
/// so a link test without it would report E0145, not what it tests. Under
/// `cargo test` the spawned binary is `target/<profile>/tungsten`, beside the
/// unhashed archive that only `cargo build -p albite_core` writes.
fn require_runtime_archive() {
    let archive = Path::new(env!("CARGO_BIN_EXE_albite"))
        .parent()
        .expect("the binary has a directory")
        .join("libalbite_core.a");
    assert!(
        archive.exists(),
        "runtime archive missing beside the tungsten binary under test: run make test, \
         which builds albite_core first ({})",
        archive.display()
    );
}

fn fixture(source: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let entry = dir.path().join("ok.tg");
    std::fs::write(&entry, source).expect("write fixture");
    (dir, entry)
}

fn compile_in(cwd: Option<&Path>, entry: &Path, extra: &[&str]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_albite"));
    cmd.arg("compile").arg(entry).args(extra);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().expect("spawn tungsten");
    Run {
        code: output.status.code().expect("exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn compile(entry: &Path, extra: &[&str]) -> Run {
    compile_in(None, entry, extra)
}

fn json<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    extra
        .iter()
        .copied()
        .chain(["--error-format", "json"])
        .collect()
}

#[test]
fn a_flag_conflict_is_one_e0140_record_on_stderr() {
    let (_dir, entry) = fixture(CLEAN);
    let run = compile(&entry, &json(&["--no-codegen", "--emit-llvm"]));
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let record = run.only_record();
    assert_eq!(record["schema"], 1);
    assert_eq!(record["kind"], "diagnostic");
    assert_eq!(record["code"], "E0140");
    assert!(record["primary_span"].is_null());
}

#[test]
fn a_type_error_is_one_e0010_record_and_no_restatement() {
    let (_dir, entry) = fixture(TYPE_ERROR);
    let run = compile(&entry, &json(&["--no-codegen"]));
    assert_eq!(run.code, 1);
    let record = run.only_record();
    assert_eq!(record["code"], "E0010");
    assert!(!run.stderr.contains("compilation failed"), "{}", run.stderr);
}

#[test]
fn a_missing_entry_is_one_code_null_failure_record() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let run = compile(&dir.path().join("nope.tg"), &json(&[]));
    assert_eq!(run.code, 1);
    let record = run.only_record();
    assert!(record["code"].is_null());
    assert!(record["message"]
        .as_str()
        .unwrap()
        .starts_with("could not read"));
}

#[test]
fn an_unwritable_ir_exits_3_with_one_e0146_record() {
    let (dir, entry) = fixture(CLEAN);
    let ir_path = dir.path().join("missing").join("x.ll");
    let ir_path = ir_path.to_str().unwrap();
    let run = compile(&entry, &json(&["--emit-llvm", "-o", ir_path]));
    assert_eq!(run.code, 3);
    assert_eq!(run.only_record()["code"], "E0146");
    let human = compile(&entry, &["--emit-llvm", "-o", ir_path]);
    assert_eq!(human.code, 3);
}

#[test]
fn a_failed_link_is_one_e0144_record_carrying_the_linker_output() {
    require_runtime_archive();
    let (dir, entry) = fixture(CLEAN);
    let exe_path = dir.path().join("missing").join("prog");
    let run = compile(&entry, &json(&["-o", exe_path.to_str().unwrap()]));
    assert_eq!(run.code, 1);
    let record = run.only_record();
    assert_eq!(record["code"], "E0144");
    assert!(record["message"]
        .as_str()
        .unwrap()
        .starts_with("linker failed with status"));
    let notes = record["notes"].as_array().unwrap();
    assert!(
        !notes.is_empty(),
        "the linker's output is captured: {record}"
    );
}

#[test]
fn a_clean_compile_keeps_stdout_and_writes_no_record() {
    require_runtime_archive();
    let (dir, entry) = fixture(CLEAN);
    let exe_path = dir.path().join("prog");
    let exe_path = exe_path.to_str().unwrap();
    let human = compile(&entry, &["-o", exe_path]);
    let run = compile(&entry, &json(&["-o", exe_path]));
    assert_eq!((human.code, run.code), (0, 0), "{}", human.stderr);
    assert!(run.stdout.starts_with("✓ Compiled to"), "{}", run.stdout);
    assert_eq!(run.stdout, human.stdout);
    assert_eq!(run.stderr, "");
}

// 30.9.26j AC5 / AC8: a bare filename prints no `cannot read directory`.
#[test]
fn a_bare_filename_reads_the_current_directory() {
    let (dir, _entry) = fixture(CLEAN);
    for extra in [vec!["--no-codegen"], json(&["--no-codegen"])] {
        let run = compile_in(Some(dir.path()), Path::new("ok.tg"), &extra);
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert!(
            !run.stderr.contains("cannot read directory"),
            "{}",
            run.stderr
        );
    }
}

// 30.9.26j D9: an ignored flag is a code-null warning record, and the
// `[no-codegen]` verdict line is suppressed.
#[test]
fn an_ignored_flag_is_one_warning_record() {
    let (_dir, entry) = fixture(CLEAN);
    let run = compile(&entry, &json(&["--no-codegen", "--debug-info"]));
    assert_eq!(run.code, 0);
    let record = run.only_record();
    assert_eq!(record["level"], "warning");
    assert!(record["code"].is_null());
    assert!(!run.stderr.contains("[no-codegen]"));
    let human = compile(&entry, &["--no-codegen", "--debug-info"]);
    assert!(human
        .stderr
        .contains("warning: ignored with --no-codegen: --debug-info"));
    assert!(human.stderr.contains("[no-codegen]"));
}
