//! ADR 20.10.26d AC1, end to end: the built `tungsten` binary, copied beside
//! a stub `albite`, prints the notice once and exits with the stub's code.
//! `src/tests.rs` drives `run`; only this test reaches `main`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// A stub `albite` that exits 7, so a forwarded code cannot be confused with
/// success or with the shim's own 126/127.
fn write_stub(dir: &Path) {
    let stub = dir.join("albite");
    std::fs::write(&stub, "#!/bin/sh\nexit 7\n").expect("write stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

// 20.10.26d AC1 — the old name warns once and returns the compiler's exit code.
#[test]
fn the_built_shim_warns_once_and_returns_the_siblings_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_stub(dir.path());
    let shim = dir.path().join("tungsten");
    std::fs::copy(env!("CARGO_BIN_EXE_tungsten"), &shim).expect("copy the shim");

    let output = Command::new(&shim)
        .arg("check")
        .output()
        .expect("run the shim");

    assert_eq!(output.status.code(), Some(7));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("`tungsten` is now `albite`").count(),
        1,
        "{stderr}"
    );
}
