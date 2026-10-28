//! Tests for table mode's expansion and run classification (ADR 3.10.26a
//! AC 1–2). Rows and the table are in table_verdict_tests.rs (AC 3).

use std::path::Path;

use super::*;

/// Write an empty file at `root/relative`, creating parent directories.
fn touch(root: &Path, relative: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
    std::fs::write(path, "").expect("write");
}

fn labels(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(Entry::label).collect()
}

// ── AC 1: expansion ─────────────────────────────────────────────────────

// 3.10.26a AC1
#[test]
fn expand_paths_lists_tg_files_sorted_and_relative() {
    let dir = tempfile::tempdir().expect("tempdir");
    touch(dir.path(), "zeta.tg");
    touch(dir.path(), "alpha.tg");
    touch(dir.path(), "sub/lexer.tg");
    touch(dir.path(), "notes.md");
    touch(dir.path(), "sub/readme.txt");
    let root = dir.path().to_path_buf();
    let entries = expand_paths(std::slice::from_ref(&root)).expect("entries");
    assert_eq!(labels(&entries), ["alpha.tg", "sub/lexer.tg", "zeta.tg"]);
    assert!(entries
        .iter()
        .all(|e| e.working_dir.as_deref() == Some(root.as_path())));
}

// 3.10.26a AC1
#[test]
fn expand_paths_keeps_a_file_as_given() {
    let dir = tempfile::tempdir().expect("tempdir");
    touch(dir.path(), "one.tg");
    let file = dir.path().join("one.tg");
    let entries = expand_paths(std::slice::from_ref(&file)).expect("entries");
    assert_eq!(
        entries,
        [Entry {
            working_dir: None,
            arg: file,
        }]
    );
}

// 3.10.26a AC1
#[test]
fn expand_paths_keeps_input_order_across_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    touch(dir.path(), "b/x.tg");
    touch(dir.path(), "a/y.tg");
    touch(dir.path(), "a/w.tg");
    let entries = expand_paths(&[dir.path().join("b"), dir.path().join("a")]).expect("entries");
    // Input order across arguments; sorted only within one directory.
    assert_eq!(labels(&entries), ["x.tg", "w.tg", "y.tg"]);
    assert_eq!(entries[0].working_dir, Some(dir.path().join("b")));
    assert_eq!(entries[1].working_dir, Some(dir.path().join("a")));
}

// 3.10.26a AC1
#[test]
fn expand_paths_skips_dot_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    touch(dir.path(), "kept.tg");
    touch(dir.path(), ".albite/cached.tg");
    touch(dir.path(), ".git/hooks.tg");
    let entries = expand_paths(&[dir.path().to_path_buf()]).expect("entries");
    assert_eq!(labels(&entries), ["kept.tg"]);
}

// 3.10.26a AC1
#[test]
fn expand_paths_deduplicates_overlapping_inputs() {
    let dir = tempfile::tempdir().expect("tempdir");
    touch(dir.path(), "a.tg");
    touch(dir.path(), "b.tg");
    let file = dir.path().join("b.tg");
    let entries = expand_paths(&[file.clone(), dir.path().to_path_buf()]).expect("entries");
    // The file argument came first and wins; the directory adds only `a.tg`.
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].arg, file);
    assert_eq!(entries[0].working_dir, None);
    assert_eq!(entries[1].label(), "a.tg");
}

// 3.10.26a AC1
#[test]
fn expand_paths_refuses_zero_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    touch(dir.path(), "notes.md");
    let err = expand_paths(&[dir.path().to_path_buf()]).expect_err("zero entries");
    assert!(err.starts_with("no `.tg` entries under "), "{err}");
    assert!(err.contains(&dir.path().display().to_string()), "{err}");
}

// ── AC 2: one run ───────────────────────────────────────────────────────

fn exited(code: i32, stdout: &str, stderr: &str) -> Result<RunCapture, String> {
    Ok(RunCapture {
        status: RunStatus::Exited(code),
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    })
}

/// The bootstrap's real refusal header, coloured even into a pipe.
const BOOTSTRAP_HEADER: &str =
    "\u{1b}[31m[E0003] Error:\u{1b}[0m cannot find constructor `MkBox` in this scope";
/// Its Signature Collection warning, whose continuation contains `error[`.
const SIG_WARNING: &str = "warning: Signature Collection global collection failed with 1 error; \
     cross-module imports may not resolve.\n  first error: error[E0005]: cannot find module \
     `nosuch` (at 161..174)\n";

// 3.10.26a AC2
#[test]
fn exit_zero_with_one_count_line_is_accepted() {
    let run = exited(0, "✓ main.tg: 1 definition(s), all OK\n", "");
    assert_eq!(
        classify_run(&run, "main.tg", Side::Bootstrap),
        Outcome::Accepted(Accepted {
            definitions: 1,
            warning: None,
        })
    );
    // A bare verdict with no suffix reads too.
    assert_eq!(
        parse_count_line("✓ a.tg: 12 definition(s)", "a.tg"),
        Ok(Accepted {
            definitions: 12,
            warning: None,
        })
    );
}

// 3.10.26a AC2
#[test]
fn a_warning_verdict_line_is_accepted_with_its_count() {
    let run = exited(
        0,
        "⚠ main.tg: 5 definition(s), 3 theorem(s) kernel-exempt (needs-delta 3)\n",
        "",
    );
    let outcome = classify_run(&run, "main.tg", Side::Bootstrap);
    assert_eq!(
        outcome,
        Outcome::Accepted(Accepted {
            definitions: 5,
            warning: Some("3 theorem(s) kernel-exempt (needs-delta 3)".to_string()),
        })
    );
    assert_eq!(
        outcome.cell(),
        "ok, 5 ⚠ 3 theorem(s) kernel-exempt (needs-delta 3)"
    );
}

// 3.10.26a AC2
#[test]
fn nested_entry_count_line_matches_its_relative_path() {
    let stdout = "✓ sub/runs_sub_lexer.tg: 2 definition(s), all OK\n";
    assert_eq!(
        parse_count_line(stdout, "sub/runs_sub_lexer.tg").map(|a| a.definitions),
        Ok(2)
    );
    // Another entry's line is not this entry's.
    assert!(parse_count_line(stdout, "runs_sub_lexer.tg").is_err());
    // Malformed shapes never count.
    for line in [
        "✓ a.tg: x definition(s)",
        "✓ a.tg: 3 definitions",
        "✓ a.tg: 3 definition(s)trailing",
        "✓ a.tg 3 definition(s)",
        "✗ a.tg: 3 definition(s)",
    ] {
        assert!(parse_count_line(line, "a.tg").is_err(), "{line}");
    }
}

// 3.10.26a AC2
#[test]
fn exit_one_with_an_error_line_is_refused() {
    let run = exited(1, "", "error[E0302]: unknown value: MkBox\nnote: more\n");
    assert_eq!(
        classify_run(&run, "a.tg", Side::Selfhost),
        Outcome::Refused("error[E0302]: unknown value: MkBox".to_string())
    );
    // A bare `error:` header is a header too; stderr is read before stdout.
    assert_eq!(
        first_error_line("error: no main\n", "error[E1]: later\n"),
        Some("error: no main".to_string())
    );
    assert_eq!(
        first_error_line("", "error[E1]: from stdout\n"),
        Some("error[E1]: from stdout".to_string())
    );
}

// 3.10.26a AC2
#[test]
fn first_error_line_reads_the_bootstraps_rendered_header() {
    let stderr =
        format!("{BOOTSTRAP_HEADER}\n   ╭─[a.tg:6:18]\n\nerror: aborting due to 1 error\n");
    assert_eq!(
        first_error_line(&stderr, ""),
        Some("[E0003] Error: cannot find constructor `MkBox` in this scope".to_string())
    );
    // `[E` must be followed by digits, then `] Error:`.
    assert_eq!(first_error_line("[E] Error: x\n", ""), None);
    assert_eq!(first_error_line("[Ex1] Error: x\n", ""), None);
    assert_eq!(first_error_line("[E12] Warning: x\n", ""), None);
}

// 3.10.26a AC2
#[test]
fn first_error_line_skips_the_aborting_trailer_and_indented_continuations() {
    let stderr = format!("{SIG_WARNING}{BOOTSTRAP_HEADER}\n\nerror: aborting due to 1 error\n");
    assert_eq!(
        first_error_line(&stderr, ""),
        Some("[E0003] Error: cannot find constructor `MkBox` in this scope".to_string())
    );
    assert_eq!(
        first_error_line(SIG_WARNING, "error: aborting due to 1 error\n"),
        None
    );
}

// 3.10.26a AC2
#[test]
fn first_error_line_strips_ansi() {
    assert_eq!(
        strip_ansi("\u{1b}[38;5;246mgrey\u{1b}[0m plain \u{1b}[31mred\u{1b}[0m"),
        "grey plain red"
    );
    assert_eq!(strip_ansi("no escapes"), "no escapes");
    assert_eq!(
        first_error_line("\u{1b}[1merror[E1]: bold\u{1b}[0m\n", ""),
        Some("error[E1]: bold".to_string())
    );
}

// 3.10.26a AC2
#[test]
fn exit_zero_without_a_count_line_is_a_fault() {
    let none = exited(0, "something else\n", "");
    assert_eq!(
        classify_run(&none, "a.tg", Side::Bootstrap),
        Outcome::Fault("exit 0 with no count line".to_string())
    );
    let two = exited(
        0,
        "✓ a.tg: 1 definition(s), all OK\n✓ a.tg: 2 definition(s), all OK\n",
        "",
    );
    assert_eq!(
        classify_run(&two, "a.tg", Side::Bootstrap),
        Outcome::Fault("exit 0 with 2 count lines".to_string())
    );
}

// 3.10.26a AC2
#[test]
fn exit_one_without_a_header_is_a_fault() {
    let run = exited(1, "", "error: aborting due to 1 error\n");
    assert_eq!(
        classify_run(&run, "a.tg", Side::Bootstrap),
        Outcome::Fault("exit 1 with no error header".to_string())
    );
}

// 3.10.26a AC2
#[test]
fn a_signal_or_other_exit_is_a_fault() {
    let three = exited(3, "", "error: cannot read a.tg\n");
    assert_eq!(
        classify_run(&three, "a.tg", Side::Bootstrap),
        Outcome::Fault("exit 3".to_string())
    );
    let killed = Ok(RunCapture {
        status: RunStatus::Signalled(9),
        stdout: String::new(),
        stderr: String::new(),
    });
    assert_eq!(
        classify_run(&killed, "a.tg", Side::Selfhost),
        Outcome::Fault("killed by signal 9".to_string())
    );
    let timed_out = Ok(RunCapture {
        status: RunStatus::TimedOut(7),
        stdout: String::new(),
        stderr: String::new(),
    });
    assert_eq!(
        classify_run(&timed_out, "a.tg", Side::Bootstrap),
        Outcome::Fault("timeout after 7s".to_string())
    );
    assert_eq!(
        classify_run(&Err("no such file".to_string()), "a.tg", Side::Bootstrap),
        Outcome::Fault("launch failed: no such file".to_string())
    );
}

// 3.10.26a AC2
#[test]
fn a_selfhost_segfault_carries_the_stack_hint() {
    let segv = Ok(RunCapture {
        status: RunStatus::Signalled(SIGSEGV),
        stdout: String::new(),
        stderr: String::new(),
    });
    let Outcome::Fault(selfhost) = classify_run(&segv, "a.tg", Side::Selfhost) else {
        panic!("a segfault is a fault");
    };
    // ADR 19.10.26c D5: the runtime sizes albite1's stack, so the hint
    // names its override, not a shell limit.
    assert!(selfhost.contains("ALBITE_STACK_SIZE"), "{selfhost}");
    assert!(!selfhost.contains("ulimit"), "{selfhost}");
    // The hint is the self-host's alone.
    assert_eq!(
        classify_run(&segv, "a.tg", Side::Bootstrap),
        Outcome::Fault(format!("killed by signal {SIGSEGV}"))
    );
}
