//! The bootstrap lists the working directory, and stays silent about its
//! siblings (ADR 30.9.26i D4, D7, AC 4).
//!
//! These drive the binary with `Command::current_dir` set inside the fixture
//! workspace, `tests/conformance/workspace/` — never a process-global
//! `chdir`. The fixture is COPIED into a tempdir first: the bootstrap writes
//! its cache beside the entry, which must not land in the repo, and a copy
//! keeps the assertions true in a mutation sweep's copied workspace too.
//!
//! What the merge buys is names, not definitions (D1): an import from an
//! undeclared sibling is value-not-found (`E0001`), and the same import with
//! no such sibling is module-not-found (`E0005`). Before D4, a bare or
//! relative entry whose root resolved to `""` listed nothing and so got
//! `E0005` for both.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIXTURE_FILES: &[&str] = &[
    "main.tg",
    "lexer.tg",
    "doubling.tg",
    "boxes.tg",
    "broken_reexport.tg",
    "sibling_only.tg",
    "qualified_sibling.tg",
    "sibling_import_unused.tg",
    "sibling_ctor.tg",
    "sibling_ctor_misuse.tg",
    "sibling_type.tg",
    "qualified_sibling_ctor.tg",
    "sibling_nullary_ctor.tg",
    "sibling_ctor_pattern.tg",
    "sibling_ctor_pattern_on_nat.tg",
    "qualified_sibling_pattern.tg",
    "sibling_nullary_pattern.tg",
    "sibling_ctor_arity.tg",
    "sibling_ctor_import_unused.tg",
    "sibling_type_misuse.tg",
    "sibling_type_forged.tg",
    "foo_on_local_adt.tg",
    "sibling_pattern_on_local_adt.tg",
    "function_as_ctor_pattern.tg",
    "nested_unknown_pattern.tg",
    "qualified_unknown_module_pattern.tg",
    "type_qualified_pattern.tg",
    "wrong_type_ctor_pattern.tg",
    "mixed_type_ctor_arms.tg",
    "nested_wrong_type_ctor_pattern.tg",
    "bool_unknown_pattern.tg",
    "mixed_spelling_nested_pattern.tg",
    "bare_capital_binding.tg",
    "sub/lexer.tg",
    "sub/test_main_wins.tg",
    "sub/runs_sub_lexer.tg",
    "sub/root_lexer_is_shadowed.tg",
    "sub/uses_root_util.tg",
    "sub/nowhere.tg",
];

fn fixture_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/conformance/workspace")
}

/// A fresh copy of the fixture workspace; the tempdir lives as long as the guard.
fn copied_workspace() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    for file in FIXTURE_FILES {
        fs::copy(fixture_source().join(file), dir.path().join(file))
            .unwrap_or_else(|e| panic!("copy {file}: {e}"));
    }
    dir
}

fn check_from(cwd: &Path, entry: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_albite"))
        .args(["check", entry])
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ADR 30.9.26i AC4: a bare entry sees the working directory's siblings, so
// `use doubling::{double}` is value-not-found, not module-not-found.
#[test]
fn bare_entry_sees_its_siblings() {
    let ws = copied_workspace();
    let out = check_from(ws.path(), "sibling_only.tg");
    let err = stderr_of(&out);
    assert!(!out.status.success(), "sibling_only.tg must be refused");
    assert!(
        err.contains("E0001") && err.contains("double"),
        "stderr: {err}"
    );
    assert!(!err.contains("E0005"), "stderr: {err}");
    // ADR 2.10.26g AC4: the bare entry's empty root lists `.` (30.9.26j).
    assert!(!err.contains("cannot read directory"), "stderr: {err}");
}

// ADR 2.10.26g AC4: a qualified use of a sibling's value names the module and
// the missing item.
#[test]
fn qualified_sibling_is_refused() {
    let ws = copied_workspace();
    let out = check_from(ws.path(), "qualified_sibling.tg");
    let err = stderr_of(&out);
    assert!(
        !out.status.success(),
        "qualified_sibling.tg must be refused"
    );
    assert!(
        err.contains("E0006") && err.contains("double") && err.contains("doubling"),
        "stderr: {err}"
    );
}

// ADR 2.10.26g AC3 (D4): every use of a sibling's constructor that reaches the
// ADT lookup is the user's `E0003`, never an internal error; the arity check
// still fires first, and what was accepted stays accepted.
#[test]
fn sibling_constructor_uses_are_e0003_not_internal() {
    let ws = copied_workspace();
    for entry in [
        "sibling_ctor.tg",
        "sibling_ctor_misuse.tg",
        "qualified_sibling_ctor.tg",
        "sibling_nullary_ctor.tg",
        "sibling_ctor_pattern.tg",
        "sibling_ctor_pattern_on_nat.tg",
        "qualified_sibling_pattern.tg",
    ] {
        let out = check_from(ws.path(), entry);
        let err = stderr_of(&out);
        assert!(!out.status.success(), "{entry} must be refused");
        assert!(err.contains("E0003"), "{entry} stderr: {err}");
        assert!(!err.contains("E9998"), "{entry} stderr: {err}");
        assert!(
            !err.contains("internal compiler error"),
            "{entry} stderr: {err}"
        );
        assert!(err.contains("boxes"), "{entry} names the module: {err}");
    }
    let arity = check_from(ws.path(), "sibling_ctor_arity.tg");
    assert!(!arity.status.success());
    assert!(stderr_of(&arity).contains("E0012"), "{}", stderr_of(&arity));
    for entry in [
        "sibling_ctor_import_unused.tg",
        "sibling_type.tg",
        "sibling_nullary_pattern.tg",
    ] {
        let out = check_from(ws.path(), entry);
        assert!(
            out.status.success(),
            "{entry} must check clean: {}",
            stderr_of(&out)
        );
    }
    for entry in ["sibling_type_misuse.tg", "sibling_type_forged.tg"] {
        let out = check_from(ws.path(), entry);
        assert!(!out.status.success(), "{entry} must be refused");
        assert!(
            stderr_of(&out).contains("E0010"),
            "{entry}: {}",
            stderr_of(&out)
        );
    }
}

// ADR 30.9.26i AC4: a relative entry one level down walks up to the working
// directory's `main.tg` and lists it.
#[test]
fn relative_entry_reaches_the_working_directory() {
    let ws = copied_workspace();
    let out = check_from(ws.path(), "sub/uses_root_util.tg");
    let err = stderr_of(&out);
    assert!(!out.status.success());
    assert!(
        err.contains("E0001") && err.contains("double"),
        "stderr: {err}"
    );
    assert!(!err.contains("E0005"), "stderr: {err}");
}

// ADR 30.9.26i AC4: the control — no sibling is named `nosuch`, so the class
// stays module-not-found after the fix.
#[test]
fn missing_module_stays_module_not_found() {
    let ws = copied_workspace();
    let out = check_from(ws.path(), "sub/nowhere.tg");
    let err = stderr_of(&out);
    assert!(!out.status.success());
    assert!(err.contains("E0005"), "stderr: {err}");
    // ADR 2.10.26g AC4 (D6): at the root the child and raw paths coincide,
    // and the list names the path once.
    assert!(err.contains("cannot find module `nosuch`"), "stderr: {err}");
    assert!(!err.contains("nosuch or nosuch"), "stderr: {err}");
}

// ADR 30.9.26i AC4 / D7: `broken_reexport.tg`'s unresolvable `pub use` is a
// sibling, and the sibling pass prints nothing: no entry's stderr names the
// broken re-export. A failing import's own signature-collection warning is
// outside this rule.
#[test]
fn sibling_reexport_failure_prints_nothing() {
    let ws = copied_workspace();
    for entry in ["main.tg", "sub/runs_sub_lexer.tg"] {
        let out = check_from(ws.path(), entry);
        assert!(
            out.status.success(),
            "{entry} must check clean: {}",
            stderr_of(&out)
        );
    }
    for entry in [
        "main.tg",
        "sub/runs_sub_lexer.tg",
        "sibling_only.tg",
        "sub/uses_root_util.tg",
        "sub/nowhere.tg",
    ] {
        let err = stderr_of(&check_from(ws.path(), entry));
        let sibling_warnings: Vec<&str> = err
            .lines()
            .filter(|l| l.contains("warning:"))
            .filter(|l| {
                l.contains("re-export") || l.contains("gone") || l.contains("broken_reexport")
            })
            .collect();
        assert!(sibling_warnings.is_empty(), "{entry} stderr: {err}");
    }
}

/// `entry` checks clean from the copied workspace.
fn assert_accepted(ws: &Path, entry: &str) {
    let out = check_from(ws, entry);
    assert!(
        out.status.success(),
        "{entry} must check clean: {}",
        stderr_of(&out)
    );
}

// ADR 3.10.26l AC7: a constructor pattern resolving to no constructor — a name
// that exists nowhere, a names-only sibling's, a function's, or one nested
// under a resolved constructor — is the bootstrap's E0003. The self-host's
// guard is held to these verdicts.
#[test]
fn unknown_ctor_patterns_are_e0003() {
    let ws = copied_workspace();
    for entry in [
        "foo_on_local_adt.tg",
        "sibling_pattern_on_local_adt.tg",
        "function_as_ctor_pattern.tg",
        "nested_unknown_pattern.tg",
    ] {
        let out = check_from(ws.path(), entry);
        let err = stderr_of(&out);
        assert!(!out.status.success(), "{entry} must be refused");
        assert!(
            err.contains("E0003") && err.contains("cannot find constructor"),
            "{entry} stderr: {err}"
        );
    }
}

// ADR 3.10.26l AC7: a pattern through a module that does not exist is E0005.
#[test]
fn unknown_module_pattern_is_e0005() {
    let ws = copied_workspace();
    let out = check_from(ws.path(), "qualified_unknown_module_pattern.tg");
    let err = stderr_of(&out);
    assert!(!out.status.success());
    assert!(
        err.contains("E0005") && err.contains("cannot find module"),
        "stderr: {err}"
    );
}

// ADR 3.10.26l AC7: `OptTyq::AlgoTyq(k)` resolves through its type.
#[test]
fn type_qualified_pattern_is_accepted() {
    assert_accepted(copied_workspace().path(), "type_qualified_pattern.tg");
}

/// `entry` is refused with E0022, and the message names both `owner` — the
/// constructor's own type — and the scrutinee's encoding.
fn assert_wrong_type_e0022(ws: &Path, entry: &str, owner: &str) {
    let out = check_from(ws, entry);
    let err = stderr_of(&out);
    assert!(!out.status.success(), "{entry} must be refused");
    assert!(err.contains("E0022"), "{entry} stderr: {err}");
    assert!(
        err.contains(&format!("match arms use constructors of `{owner}`"))
            && err.contains("but the scrutinee has type `(Unit + Nat)`"),
        "{entry} stderr: {err}"
    );
}

// ADR 4.10.26d AC2: another type's constructor no longer matches by index —
// the pin 3.10.26l AC7 set on the false accept, turned around.
#[test]
fn wrong_type_ctor_pattern_is_e0022() {
    let ws = copied_workspace();
    assert_wrong_type_e0022(ws.path(), "wrong_type_ctor_pattern.tg", "OtherWrong");
}

// ADR 4.10.26d AC2: mixed arms (D1 a) and a nested pattern (D1, nested).
#[test]
fn mixed_and_nested_wrong_type_ctor_patterns_are_e0022() {
    let ws = copied_workspace();
    assert_wrong_type_e0022(ws.path(), "mixed_type_ctor_arms.tg", "OtherMixed");
    assert_wrong_type_e0022(ws.path(), "nested_wrong_type_ctor_pattern.tg", "OtherNest");
}

// ADR 3.10.26l AC7: `Nope => 0` is a binding, never a constructor pattern.
#[test]
fn bare_capital_pattern_is_a_binding() {
    assert_accepted(copied_workspace().path(), "bare_capital_binding.tg");
}
