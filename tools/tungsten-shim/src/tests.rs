//! ADR 20.10.26d AC1: the old name warns once and behaves identically.

use super::notice::{notice, DEPRECATION_NOTICE};
use super::{exit_code_of, forward, run, sibling_compiler, EXIT_CANNOT_RUN, EXIT_NO_SIBLING};
use std::path::{Path, PathBuf};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

// 20.10.26d AC1 — the notice names albite, and JSON errors silence it.
#[test]
fn notice_names_albite_for_an_ordinary_invocation() {
    let line = notice(&strings(&["check", "main.tg"])).expect("a human invocation warns");
    assert_eq!(line, DEPRECATION_NOTICE);
    assert!(line.contains("`albite`"), "{line}");
    assert!(line.contains("removed in 2.1"), "{line}");
}

#[test]
fn notice_is_printed_for_no_arguments_at_all() {
    assert_eq!(notice(&[]).as_deref(), Some(DEPRECATION_NOTICE));
}

#[test]
fn notice_is_none_under_error_format_json_as_two_tokens() {
    assert_eq!(
        notice(&strings(&["check", "--error-format", "json", "main.tg"])),
        None
    );
}

#[test]
fn notice_is_none_under_error_format_json_with_equals() {
    assert_eq!(notice(&strings(&["check", "--error-format=json"])), None);
}

#[test]
fn notice_stays_for_a_non_json_error_format() {
    assert!(notice(&strings(&["check", "--error-format", "human"])).is_some());
    assert!(notice(&strings(&["check", "--error-format=human"])).is_some());
}

#[test]
fn notice_needs_the_value_after_the_flag_not_anywhere() {
    // `json` as an ordinary argument, or a dangling flag, is not the JSON mode.
    assert!(notice(&strings(&["json", "--error-format"])).is_some());
    assert!(notice(&strings(&["--error-format", "human", "json"])).is_some());
}

#[test]
fn sibling_compiler_is_albite_in_the_executables_own_directory() {
    let exe = Path::new("/opt/albite/bin/tungsten");
    let expected = PathBuf::from(format!(
        "/opt/albite/bin/albite{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert_eq!(sibling_compiler(exe), expected);
}

#[test]
fn exit_code_of_forwards_a_code_and_maps_a_signal_death_to_one() {
    assert_eq!(exit_code_of(Some(0)), 0);
    assert_eq!(exit_code_of(Some(42)), 42);
    assert_eq!(exit_code_of(None), 1);
    assert_eq!(exit_code_of(Some(-1)), 1);
}

// 20.10.26d AC1 — a missing sibling exits 127.
#[test]
fn forward_to_a_missing_sibling_exits_127() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("albite");
    assert_eq!(forward(&absent, &[], false), EXIT_NO_SIBLING);
    assert_eq!(forward(&absent, &[], true), EXIT_NO_SIBLING);
}

/// A stub `albite` that records its arguments, one per line, and exits 7.
#[cfg(unix)]
fn write_recording_stub(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let record = dir.join("args.txt");
    let stub = dir.join("albite");
    let script = format!(
        "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\"; done > '{}'\nexit 7\n",
        record.display()
    );
    std::fs::write(&stub, script).expect("write stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
    (stub, record)
}

// 20.10.26d AC1 — the arguments pass through and the stub's exit code returns.
#[cfg(unix)]
#[test]
fn forward_passes_arguments_through_and_returns_the_stubs_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (stub, record) = write_recording_stub(dir.path());
    let args = strings(&["check", "a file.tg", "--error-format", "json"]);
    assert_eq!(forward(&stub, &args, false), 7);
    let seen = std::fs::read_to_string(record).expect("the stub ran");
    assert_eq!(
        seen.lines().collect::<Vec<_>>(),
        ["check", "a file.tg", "--error-format", "json"]
    );
}

// The exec arm returns only on failure: a sibling that exists but cannot be
// executed is 126, and this test process is not replaced.
#[cfg(unix)]
#[test]
fn exec_of_an_unexecutable_sibling_reports_126() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let sibling = dir.path().join("albite");
    std::fs::write(&sibling, "not a program").expect("write");
    std::fs::set_permissions(&sibling, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(forward(&sibling, &[], true), EXIT_CANNOT_RUN);
}

// The spawn arm (no exec) reports the same 126 when the sibling cannot run.
#[cfg(unix)]
#[test]
fn spawn_of_an_unexecutable_sibling_reports_126() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let sibling = dir.path().join("albite");
    std::fs::write(&sibling, "not a program").expect("write");
    std::fs::set_permissions(&sibling, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(forward(&sibling, &[], false), EXIT_CANNOT_RUN);
}

// 20.10.26d AC1 — `run` finds the stub beside the given executable path and
// returns its exit code.
#[cfg(unix)]
#[test]
fn run_forwards_to_the_albite_beside_the_executable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_stub, record) = write_recording_stub(dir.path());
    let exe = Ok(dir.path().join("tungsten"));
    assert_eq!(run(&strings(&["check", "x.tg"]), exe, false), 7);
    let seen = std::fs::read_to_string(record).expect("the stub ran");
    assert_eq!(seen.lines().collect::<Vec<_>>(), ["check", "x.tg"]);
}

// 20.10.26d AC1 — an executable path the OS cannot report exits 127.
#[test]
fn run_without_a_known_executable_path_exits_127() {
    let exe = Err(std::io::Error::other("no /proc"));
    assert_eq!(run(&[], exe, false), EXIT_NO_SIBLING);
}
