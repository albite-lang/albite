//! Golden test execution. Split out of `main.rs` (ADR 16.7.26b file-size
//! paydown). Runs a category's tests (or regenerates them in `--update` mode),
//! including the compile-then-run path for the `compile` category.

use std::path::Path;
use std::process::Command;

use crate::capture::{capture_compile_run, snapshot_bytes, CompileRun, UpdateTally};
use crate::compare::{compare_output, compare_snapshot, TestResult};
use crate::compiler::{read_args_file, run_compiler};
use crate::discover::discover_tests;
use crate::update::update_test;
use crate::{Category, Cli};

/// Run one category's tests. `codegen_available` comes from the run-level
/// preflight (ADR 21.7.26f / D3) rather than a per-category probe, so the
/// binary is inspected once and its findings are named in one place.
/// One category's outcome counts.
///
/// A named record rather than a tuple: `missing` is the fourth counter, which
/// would put the return past the 3-value arity cap, and — more usefully — it
/// makes the tally a *value* the tests below can assert on. `exit_code()` is
/// then the only place the pass/fail policy lives.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CategoryTally {
    pub passed: u32,
    pub failed: u32,
    /// `.tg` fixtures with no `.expected` snapshot. Non-zero fails the run.
    pub missing: u32,
    pub skipped: u32,
}

impl CategoryTally {
    /// Fold another category's counts into this one.
    pub(crate) fn add(&mut self, other: CategoryTally) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.missing += other.missing;
        self.skipped += other.skipped;
    }

    /// An `--update` run's counts as a tally: only its failures — a snapshot
    /// left as it was for want of a result (ADR 5.10.26b D3) — fail the run.
    pub(crate) fn from_updates(updates: UpdateTally) -> CategoryTally {
        CategoryTally {
            failed: updates.failed,
            ..CategoryTally::default()
        }
    }

    /// Whether the run should exit non-zero.
    ///
    /// A missing snapshot counts, which is the whole point: it used to be
    /// tallied as a pass, so a fixture added without `--update` was invisible
    /// to CI (see [`TestResult::MissingSnapshot`]).
    pub(crate) fn is_failure(&self) -> bool {
        self.failed > 0 || self.missing > 0
    }
}

pub(crate) fn run_category(
    cli: &Cli,
    category: &Category,
    codegen_available: bool,
) -> CategoryTally {
    // Before the directory lookup: there is no `json-parity/`, so the generic
    // path would print `(no tests)` and pass (ADR 28.9.26e §2.4).
    if matches!(category, Category::JsonParity) {
        return if cli.update {
            let _ = update_test(cli, category, Path::new(""), Path::new(""));
            CategoryTally::default()
        } else {
            crate::json_parity::run(cli, codegen_available)
        };
    }
    let dir = cli.dir.join(category.dir_name());
    let tests = discover_tests(&dir);

    println!("[{}]", category.as_str());

    if tests.is_empty() {
        println!("  (no tests)");
        println!();
        return CategoryTally::default();
    }

    let mut tally = CategoryTally::default();
    let mut updates = UpdateTally::default();

    for (tg, expected) in &tests {
        if category.needs_codegen() && !codegen_available {
            println!(
                "\x1b[33m⊘\x1b[0m {} (skipped: codegen not available{})",
                tg.display(),
                skip_note(expected.exists())
            );
            tally.skipped += 1;
            continue;
        }

        if cli.update {
            updates.record(update_test(cli, category, tg, expected));
        } else {
            match run_test(cli, category, tg, expected) {
                TestResult::Pass => tally.passed += 1,
                TestResult::Fail => tally.failed += 1,
                TestResult::MissingSnapshot => tally.missing += 1,
            }
        }
    }

    println!();
    if cli.update {
        // A failed update left its snapshot as it was, and fails the run (D3).
        print!("{}", updates.summary_line());
        tally.add(CategoryTally::from_updates(updates));
    } else {
        print!("{}", summary_line(&tally));
    }
    println!();

    tally
}

/// What a codegen-less skip should add about the fixture's snapshot.
///
/// The skip itself is legitimate on an LLVM-less host, but it fires *before*
/// [`compare_output`] is ever reached, so a `compile` fixture with no
/// `.expected` was folded into the skip count and named nothing — the same
/// silent-cap shape [`TestResult::MissingSnapshot`] closed for the other
/// categories.
///
/// Reported, not failed: `--update` cannot generate a `compile` snapshot on a
/// host without codegen either, so failing here would be a red gate the host
/// cannot clear. The codegen-enabled run (devcontainer, CI) reaches
/// `compare_output` and still counts it as `MissingSnapshot`, which does fail.
pub(crate) fn skip_note(expected_exists: bool) -> &'static str {
    if expected_exists {
        ""
    } else {
        "; NO SNAPSHOT — run --update where codegen is available"
    }
}

/// The one-line count summary, as a value so it can be asserted.
pub(crate) fn summary_line(tally: &CategoryTally) -> String {
    let mut out = format!("  {} passed, {} failed", tally.passed, tally.failed);
    if tally.missing > 0 {
        out.push_str(&format!(", {} MISSING SNAPSHOT", tally.missing));
    }
    if tally.skipped > 0 {
        out.push_str(&format!(", {} skipped", tally.skipped));
    }
    out.push('\n');
    out
}

pub(crate) fn clean_cache(compiler: &Path) {
    let _ = Command::new(compiler).args(["cache", "clean"]).output();
}

fn run_test(cli: &Cli, category: &Category, tg: &Path, expected: &Path) -> TestResult {
    if matches!(category, Category::Compile) {
        return run_compile_test(cli, tg, expected);
    }

    let extra_args = read_args_file(tg);
    let actual = run_compiler(&cli.compiler, category.tungsten_cmd(), tg, &extra_args);
    compare_output(tg, expected, &actual)
}

/// The `compile` check: the same capture and serialization `--update` uses
/// (ADR 5.10.26b D1), compared byte for byte with the snapshot file.
fn run_compile_test(cli: &Cli, tg: &Path, expected: &Path) -> TestResult {
    let failure = match capture_compile_run(&cli.compiler, tg) {
        CompileRun::Ran { stdout, stderr } => {
            return compare_snapshot(tg, expected, &snapshot_bytes(&stdout, &stderr))
        }
        CompileRun::CompileFailed => "compile failed".to_string(),
        CompileRun::NoBinary => "no binary produced".to_string(),
        CompileRun::RunFailed(e) => format!("run failed: {e}"),
    };
    println!("\x1b[31m✗\x1b[0m {} ({failure})", tg.display());
    TestResult::Fail
}

#[cfg(test)]
mod tests {
    use super::{skip_note, summary_line, CategoryTally};

    #[test]
    fn a_missing_snapshot_fails_the_run() {
        // The bug this guards: a `.tg` with no `.expected` was counted as a
        // pass and the runner exited 0, so a fixture added without `--update`
        // was invisible to CI while looking green locally (ADR 7.8.26e found
        // seven at once).
        let missing = CategoryTally {
            passed: 39,
            missing: 1,
            ..CategoryTally::default()
        };
        assert!(missing.is_failure(), "a missing snapshot must fail the run");
    }

    #[test]
    fn a_clean_run_succeeds() {
        let clean = CategoryTally {
            passed: 39,
            skipped: 2,
            ..CategoryTally::default()
        };
        assert!(!clean.is_failure());
    }

    #[test]
    fn a_real_diff_still_fails() {
        let failed = CategoryTally {
            passed: 38,
            failed: 1,
            ..CategoryTally::default()
        };
        assert!(failed.is_failure());
    }

    #[test]
    fn skipped_alone_never_fails() {
        // `compile` fixtures are skipped without codegen; that is not an error.
        let skipped = CategoryTally {
            skipped: 5,
            ..CategoryTally::default()
        };
        assert!(!skipped.is_failure());
    }

    #[test]
    fn adding_folds_every_counter() {
        let mut total = CategoryTally {
            passed: 1,
            failed: 2,
            missing: 3,
            skipped: 4,
        };
        total.add(CategoryTally {
            passed: 10,
            failed: 20,
            missing: 30,
            skipped: 40,
        });
        assert_eq!(
            total,
            CategoryTally {
                passed: 11,
                failed: 22,
                missing: 33,
                skipped: 44,
            }
        );
    }

    // ADR 5.10.26b D3: a failed update fails the run; updated and unchanged
    // snapshots do not, and do not count as passes either.
    #[test]
    fn an_update_tally_folds_only_its_failures() {
        use crate::capture::UpdateTally;
        let mut tally = CategoryTally {
            skipped: 1,
            failed: 1,
            ..CategoryTally::default()
        };
        tally.add(CategoryTally::from_updates(UpdateTally {
            updated: 3,
            unchanged: 4,
            failed: 2,
        }));
        assert_eq!(
            tally,
            CategoryTally {
                skipped: 1,
                failed: 3,
                ..CategoryTally::default()
            }
        );
        assert!(tally.is_failure());
        assert!(!CategoryTally::from_updates(UpdateTally {
            updated: 3,
            unchanged: 4,
            failed: 0,
        })
        .is_failure());
    }

    #[test]
    fn the_summary_names_a_missing_snapshot_loudly() {
        let tally = CategoryTally {
            passed: 39,
            missing: 1,
            ..CategoryTally::default()
        };
        let line = summary_line(&tally);
        assert!(line.contains("1 MISSING SNAPSHOT"), "{line}");
        // It must not read as a clean run.
        assert!(line.contains("39 passed, 0 failed"), "{line}");
    }

    #[test]
    fn a_codegen_less_skip_names_a_missing_snapshot() {
        // The residual of the same bug: the compile-category skip fires before
        // the snapshot is consulted, so without this the fixture is counted as
        // "skipped" and nothing says its `.expected` was never written.
        let note = skip_note(false);
        assert!(note.contains("NO SNAPSHOT"), "{note}");
        assert!(
            note.contains("--update"),
            "the remedy must be named: {note}"
        );
    }

    #[test]
    fn a_codegen_less_skip_is_silent_when_the_snapshot_exists() {
        assert_eq!(skip_note(true), "");
    }

    #[test]
    fn the_summary_omits_zero_counters() {
        let line = summary_line(&CategoryTally {
            passed: 3,
            ..CategoryTally::default()
        });
        assert_eq!(line, "  3 passed, 0 failed\n");
    }

    // ADR 30.9.26j §2.3: `compile-error` reads `compile_error/`, runs
    // `compile`, and is skipped without codegen like `compile`.
    #[test]
    fn compile_error_is_a_codegen_category_with_its_own_directory() {
        use crate::Category;
        assert_eq!(Category::CompileError.as_str(), "compile-error");
        assert_eq!(Category::CompileError.dir_name(), "compile_error");
        assert_eq!(Category::CompileError.tungsten_cmd(), "compile");
        assert_eq!(Category::Error.dir_name(), "error");
        assert!(Category::CompileError.needs_codegen());
        assert!(Category::Compile.needs_codegen());
        assert!(!Category::Error.needs_codegen());
        assert!(!Category::Run.needs_codegen());
    }
}
