//! End-to-end tests for table mode against STUB compiler executables (ADR
//! 3.10.26a AC 4, D5). Both sides are small shell scripts written into a
//! tempdir — the bootstrap's through the injected `RunOptions::bootstrap_exe`
//! — that record their working directory and argv, then print canned output.
//! No real compiler runs, so these prove the spawning half: the child's
//! working directory, the absolute binaries, `--no-cache`, the exit codes
//! and the timeout.

use std::os::unix::fs::PermissionsExt as _;
use std::sync::Mutex;

use super::table::Outcome;
use super::table_mode::{table_rows, Refusal};
use super::table_verdict::{table_exit_code, Row, Verdict};
use super::*;

/// Writing an executable while another thread forks can fail its spawn with
/// `ETXTBSY`; serializing these tests keeps that window closed among them.
static STUB_LOCK: Mutex<()> = Mutex::new(());

/// A stub compiler at `dir/name`: it appends `<cwd>|<argv>` to `dir/name.log`,
/// sets `$entry` to its last argument, then runs `body`.
fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let log = dir.join(format!("{name}.log"));
    let script = format!(
        "#!/bin/sh\nprintf '%s|%s\\n' \"$(pwd -P)\" \"$*\" >> '{}'\n\
         for a in \"$@\"; do entry=\"$a\"; done\n{body}\n",
        log.display()
    );
    std::fs::write(&path, script).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// The `<cwd>|<argv>` lines a stub recorded.
fn log_lines(dir: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(format!("{name}.log")))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Accept every entry with one definition.
const ACCEPT_ALL: &str = "echo \"✓ $entry: 1 definition(s), all OK\"";

/// A fixture: a `bin/` for the stubs and a `ws/` workspace with two entries,
/// one nested.
struct Fixture {
    _tmp: tempfile::TempDir,
    bin: PathBuf,
    workspace: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(tmp.path()).expect("canonical");
    let bin = root.join("bin");
    let workspace = root.join("ws");
    std::fs::create_dir_all(&bin).expect("mkdir bin");
    std::fs::create_dir_all(workspace.join("sub")).expect("mkdir ws");
    std::fs::write(workspace.join("a.tg"), "").expect("write a.tg");
    std::fs::write(workspace.join("sub/b.tg"), "").expect("write b.tg");
    Fixture {
        _tmp: tmp,
        bin,
        workspace,
    }
}

impl Fixture {
    fn options(&self, bootstrap_body: &str, selfhost_body: &str) -> RunOptions {
        RunOptions {
            bootstrap_exe: stub(&self.bin, "bootstrap", bootstrap_body),
            selfhost_binary: stub(&self.bin, "selfhost", selfhost_body),
            timeout: Duration::from_secs(10),
            verbose: false,
        }
    }

    fn workspace_arg(&self) -> Vec<PathBuf> {
        vec![self.workspace.clone()]
    }
}

fn verdicts(rows: &[Row]) -> Vec<(String, Verdict)> {
    rows.iter()
        .map(|row| (row.entry.clone(), row.verdict.clone()))
        .collect()
}

// 3.10.26a AC4
#[test]
fn directory_mode_runs_each_entry_inside_the_directory() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let run = fx.options(ACCEPT_ALL, ACCEPT_ALL);
    let rows = table_rows(&fx.workspace_arg(), &run).expect("rows");
    assert_eq!(
        verdicts(&rows),
        [
            ("a.tg".to_string(), Verdict::Agree),
            ("sub/b.tg".to_string(), Verdict::Agree),
        ]
    );
    let ws = fx.workspace.display();
    assert_eq!(
        log_lines(&fx.bin, "selfhost"),
        [format!("{ws}|check a.tg"), format!("{ws}|check sub/b.tg")]
    );
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(&fx.workspace_arg(), &run),
        ExitCode::SUCCESS
    );
}

// 3.10.26a AC4
#[test]
fn the_bootstrap_stub_gets_no_cache() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let run = fx.options(ACCEPT_ALL, ACCEPT_ALL);
    table_rows(&fx.workspace_arg(), &run).expect("rows");
    let ws = fx.workspace.display();
    assert_eq!(
        log_lines(&fx.bin, "bootstrap"),
        [
            format!("{ws}|check --no-cache a.tg"),
            format!("{ws}|check --no-cache sub/b.tg"),
        ]
    );
}

// 3.10.26a AC4
#[test]
fn relative_selfhost_binary_resolves_against_the_caller() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let mut run = fx.options(ACCEPT_ALL, ACCEPT_ALL);
    // Spell the self-host stub relative to THIS process's working directory:
    // climb to `/`, then descend. Run from inside the workspace unresolved,
    // it would name nothing and every row would fault.
    let caller = std::env::current_dir().expect("cwd");
    let climb = "../".repeat(caller.components().count() - 1);
    let absolute = run.selfhost_binary.clone();
    let below_root = absolute.strip_prefix("/").expect("absolute");
    run.selfhost_binary = PathBuf::from(climb).join(below_root);
    assert!(run.selfhost_binary.is_relative());

    let rows = table_rows(&fx.workspace_arg(), &run).expect("rows");
    assert!(
        rows.iter().all(|row| row.verdict == Verdict::Agree),
        "{rows:?}"
    );
    assert_eq!(log_lines(&fx.bin, "selfhost").len(), 2);
}

// 3.10.26a AC4
#[test]
fn divergence_exits_one() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let refuse_b = "case \"$entry\" in sub/b.tg) echo 'error[E0302]: unknown value: MkBox' >&2; \
                    exit 1;; *) echo \"✓ $entry: 1 definition(s), all OK\";; esac";
    let run = fx.options(ACCEPT_ALL, refuse_b);
    let rows = table_rows(&fx.workspace_arg(), &run).expect("rows");
    assert_eq!(
        verdicts(&rows),
        [
            ("a.tg".to_string(), Verdict::Agree),
            (
                "sub/b.tg".to_string(),
                Verdict::Diverge("bootstrap accepts")
            ),
        ]
    );
    assert_eq!(
        rows[1].selfhost,
        Outcome::Refused("error[E0302]: unknown value: MkBox".to_string())
    );
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(&fx.workspace_arg(), &run),
        ExitCode::from(1)
    );
}

// 3.10.26a AC4
#[test]
fn a_fault_exits_three() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    // Entry `a.tg` diverges and `sub/b.tg` hits the self-host's I/O-error
    // exit 3: the fault outranks the divergence.
    let selfhost = "case \"$entry\" in a.tg) echo 'error: no' >&2; exit 1;; *) exit 3;; esac";
    let run = fx.options(ACCEPT_ALL, selfhost);
    let rows = table_rows(&fx.workspace_arg(), &run).expect("rows");
    assert_eq!(rows[1].selfhost, Outcome::Fault("exit 3".to_string()));
    assert_eq!(rows[1].verdict, Verdict::Fault);
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(&fx.workspace_arg(), &run),
        ExitCode::from(3)
    );
}

// 3.10.26a AC4
#[test]
fn a_hung_compiler_is_a_fault_after_the_timeout() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    std::fs::remove_file(fx.workspace.join("sub/b.tg")).expect("one entry");
    // `exec` so the kill reaches the sleeper itself, not a parent shell
    // whose orphaned child would keep the pipes open.
    let mut run = fx.options("exec sleep 30", ACCEPT_ALL);
    run.timeout = Duration::from_secs(1);
    let started = Instant::now();
    let rows = table_rows(&fx.workspace_arg(), &run).expect("rows");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the kill fired"
    );
    assert_eq!(
        rows[0].bootstrap,
        Outcome::Fault("timeout after 1s".to_string())
    );
    // The run moved on to the other compiler.
    assert_eq!(log_lines(&fx.bin, "selfhost").len(), 1);
    assert_eq!(table_exit_code(&rows), 3);
}

// 3.10.26a AC4
#[test]
fn a_directory_with_no_selfhost_binary_exits_two_with_the_hint() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let mut run = fx.options(ACCEPT_ALL, ACCEPT_ALL);
    run.selfhost_binary = fx.bin.join("absent-tungsten1");
    let refusal = table_rows(&fx.workspace_arg(), &run).expect_err("refused");
    assert_eq!(refusal, Refusal::Missing(MissingInput::SelfhostBinary));
    let message = missing_message(&MissingInput::SelfhostBinary, &run.selfhost_binary);
    assert!(
        message.contains("hint: build with `make devcontainer-self-compile-fast`"),
        "{message}"
    );
    assert!(message.contains("absent-tungsten1"), "{message}");
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(&fx.workspace_arg(), &run),
        ExitCode::from(2)
    );
    assert!(
        log_lines(&fx.bin, "bootstrap").is_empty(),
        "nothing spawned"
    );
}

// 3.10.26a AC4
#[test]
fn a_missing_path_exits_two() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let run = fx.options(ACCEPT_ALL, ACCEPT_ALL);
    let absent = fx.workspace.join("absent.tg");
    let paths = [fx.workspace.clone(), absent.clone()];
    assert_eq!(
        table_rows(&paths, &run).expect_err("refused"),
        Refusal::Missing(MissingInput::SourceFile(absent))
    );
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(&paths, &run),
        ExitCode::from(2)
    );
    // A directory with no `.tg` entries is the same exit.
    let empty = fx.bin.join("empty");
    std::fs::create_dir_all(&empty).expect("mkdir");
    assert!(matches!(
        table_rows(std::slice::from_ref(&empty), &run),
        Err(Refusal::NoEntries(_))
    ));
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(&[empty], &run),
        ExitCode::from(2)
    );
    assert!(
        log_lines(&fx.bin, "bootstrap").is_empty(),
        "nothing spawned"
    );
}

// 3.10.26a AC4
#[test]
fn a_single_file_keeps_the_old_mode() {
    let _guard = STUB_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fx = fixture();
    let same_error = "echo 'error[E1]: boom' >&2; exit 1";
    let run = fx.options(same_error, same_error);
    let file = fx.workspace.join("a.tg");
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(std::slice::from_ref(&file), &run),
        ExitCode::SUCCESS
    );
    // Today's argv, from the caller's working directory: no `--no-cache`.
    let caller = std::fs::canonicalize(std::env::current_dir().expect("cwd")).expect("canonical");
    let expected = format!("{}|check {}", caller.display(), file.display());
    assert_eq!(log_lines(&fx.bin, "bootstrap"), [expected.clone()]);
    assert_eq!(log_lines(&fx.bin, "selfhost"), [expected]);

    // A differing error set is still single-file mode's exit 1, and a
    // hung compiler is reported as a launch failure, exit 1.
    let run = fx.options(same_error, "echo 'error[E2]: bang' >&2; exit 1");
    assert_eq!(
        cmd_diff_bootstrap_selfhost_check(std::slice::from_ref(&file), &run),
        ExitCode::FAILURE
    );
    let mut hung = fx.options("exec sleep 30", same_error);
    hung.timeout = Duration::from_secs(1);
    assert_eq!(
        run_check_single(&hung.bootstrap_exe, &file, &hung),
        Err("timed out after 1s".to_string())
    );
}
