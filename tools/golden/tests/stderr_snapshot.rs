//! End-to-end: a `compile` golden whose binary reports only through stderr
//! passes the check AND survives `--update` byte for byte (ADR 5.10.26b AC3).
//!
//! The compiler is a stub shell script, so no LLVM is needed: `commands`
//! satisfies the preflight, and `compile <dir>/main.tg` writes `<dir>/main` as
//! a script that prints to stderr and exits nonzero, as `int_trap` does.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TRAP_SNAPSHOT: &str = "error: integer overflow in +\n";

/// A golden tree with one multi-file `compile` test, `compile/trap/main.tg`,
/// and a stub compiler whose binaries get file mode `binary_mode`.
fn golden_tree(root: &Path, binary_mode: &str) -> (PathBuf, PathBuf) {
    let test_dir = root.join("golden/compile/trap");
    fs::create_dir_all(&test_dir).unwrap();
    fs::write(test_dir.join("main.tg"), "fn main() {}\n").unwrap();

    let compiler = root.join("tungsten");
    let body = format!(
        "#!/bin/sh\n\
         case \"$1\" in\n\
         commands) echo 'compile   Compile a file' ;;\n\
         compile) out=\"${{2%.tg}}\"; printf '%s\\n' '#!/bin/sh' \
         'echo \"error: integer overflow in +\" >&2' 'exit 101' > \"$out\"; chmod {binary_mode} \"$out\" ;;\n\
         esac\n"
    );
    fs::write(&compiler, body).unwrap();
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
    (compiler, test_dir.join("main.expected"))
}

fn golden(compiler: &Path, root: &Path, update: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_golden"));
    cmd.arg("compile")
        .arg("--no-clean")
        .arg("--compiler")
        .arg(compiler)
        .arg("--dir")
        .arg(root.join("golden"));
    if update {
        cmd.arg("--update");
    }
    cmd.output().unwrap()
}

fn stdout(run: &Output) -> String {
    String::from_utf8_lossy(&run.stdout).into_owned()
}

// ADR 5.10.26b AC3
#[test]
fn a_stderr_only_snapshot_passes_and_survives_an_update() {
    let root = tempfile::tempdir().unwrap();
    let (compiler, expected) = golden_tree(root.path(), "755");
    fs::write(&expected, TRAP_SNAPSHOT).unwrap();

    let check = golden(&compiler, root.path(), false);
    assert!(check.status.success(), "check: {}", stdout(&check));
    assert!(
        stdout(&check).contains("1 passed, 0 failed"),
        "{}",
        stdout(&check)
    );

    let update = golden(&compiler, root.path(), true);
    assert!(update.status.success(), "update: {}", stdout(&update));
    assert_eq!(fs::read_to_string(&expected).unwrap(), TRAP_SNAPSHOT);
    assert!(!stdout(&update).contains("Updated"), "{}", stdout(&update));
    assert!(
        stdout(&update).contains("0 updated, 1 unchanged"),
        "{}",
        stdout(&update)
    );
}

#[test]
fn the_check_is_byte_exact_against_the_update() {
    // A snapshot whose bytes the update would change must not pass the check.
    let root = tempfile::tempdir().unwrap();
    let (compiler, expected) = golden_tree(root.path(), "755");
    fs::write(&expected, "error: integer overflow in +\n\n").unwrap();

    let check = golden(&compiler, root.path(), false);
    assert!(!check.status.success(), "{}", stdout(&check));

    let update = golden(&compiler, root.path(), true);
    assert!(stdout(&update).contains("Updated"), "{}", stdout(&update));
    assert_eq!(fs::read_to_string(&expected).unwrap(), TRAP_SNAPSHOT);
}

#[test]
fn a_spawn_failure_leaves_the_snapshot_unwritten() {
    let root = tempfile::tempdir().unwrap();
    // The "binary" is produced without its execute bit, so it cannot spawn.
    let (compiler, expected) = golden_tree(root.path(), "644");

    let update = golden(&compiler, root.path(), true);

    assert!(!update.status.success(), "{}", stdout(&update));
    assert!(
        !expected.exists(),
        "no snapshot is written from a failed run"
    );
    assert!(
        stdout(&update).contains("Failed to run"),
        "{}",
        stdout(&update)
    );
    assert!(stdout(&update).contains("1 failed"), "{}", stdout(&update));
}
