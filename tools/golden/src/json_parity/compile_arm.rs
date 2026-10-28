//! The `json-parity` category's `compile` arm (ADR 30.9.26j D10).
//!
//! Every `compile_error/` fixture whose name does not start with `json_` runs
//! `compile` twice with its `.args` — once as is, once with `--error-format
//! json` appended — and the two runs must report the same failures. A
//! `json_*` fixture is a golden twin, not a pair to compare.
//!
//! `compile` writes its records to **stderr** (its stdout is its product), so
//! the JSON run's records are read from there, and rule 2 is strengthened:
//! every JSON-mode stderr line must be a schema-1 record. Rule 5 is new: a
//! back-end failure has no human header to compare against, so the number of
//! unlocated error records must equal the number of human `error: ` failure
//! lines. The decision is [`compare_compile`], a pure function over two
//! captured runs; [`run`] only spawns the compiler and prints.

use std::path::Path;
use std::process::Command;

use super::{headers, records, sorted, Captured};
use crate::compare::strip_ansi;
use crate::compiler::read_args_file;
use crate::discover::discover_tests;
use crate::run::CategoryTally;
use crate::Cli;

/// A fixture is run as a pair unless it is a `json_*` golden twin.
pub(crate) fn is_pair_fixture(tg: &Path) -> bool {
    !tg.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("json_"))
}

/// The human run's `error: ` failure lines — not the `aborting due to`
/// summary, and not the `compilation failed:` restatement of records the
/// front end already wrote (30.9.26j D5).
fn failure_lines(stderr: &str) -> usize {
    strip_ansi(stderr)
        .lines()
        .filter(|l| {
            l.starts_with("error: ")
                && !l.starts_with("error: aborting due to")
                && !l.starts_with("error: compilation failed:")
        })
        .count()
}

/// Why a fixture's two `compile` runs diverge, or `Ok` when they agree.
pub(crate) fn compare_compile(human: &Captured, json: &Captured) -> Result<(), String> {
    // Rule 1: the same exit code, and a failing run has an error record.
    if human.code != json.code {
        return Err(format!(
            "rule 1: exit {} (human) vs {} (json)",
            human.code, json.code
        ));
    }
    // Rule 2, strengthened: every JSON-mode stderr line is a schema-1 record.
    let found = records(&json.stderr).map_err(|why| format!("rule 2: {why}"))?;
    if json.code != 0 && !found.iter().any(|((level, _), _)| level == "error") {
        return Err(format!("rule 1: exit {} with no error record", json.code));
    }
    // Rule 4: a located record always has a code.
    if found
        .iter()
        .any(|((_, code), located)| *located && code.is_none())
    {
        return Err("rule 4: located record without a code".to_string());
    }
    // Rule 3: located records against the human headers, as a multiset.
    let located = sorted(
        found
            .iter()
            .filter(|(_, located)| *located)
            .map(|(pair, _)| pair.clone())
            .collect(),
    );
    let from_human = sorted(headers(&human.stderr));
    if located != from_human {
        return Err(format!("rule 3: human {from_human:?} vs json {located:?}"));
    }
    // Rule 5: one unlocated error record per human failure line.
    let unlocated_errors = found
        .iter()
        .filter(|((level, _), located)| !*located && level == "error")
        .count();
    let human_failures = failure_lines(&human.stderr);
    if unlocated_errors != human_failures {
        return Err(format!(
            "rule 5: {human_failures} human `error: ` line(s) vs {unlocated_errors} unlocated error record(s)"
        ));
    }
    Ok(())
}

/// How many of a JSON run's records carry a build-block code (E0140–E0149) —
/// the arm's vacuity floor. Unparseable output counts nothing;
/// [`compare_compile`] reports it.
pub(crate) fn build_coded(json: &Captured) -> u32 {
    let Ok(found) = records(&json.stderr) else {
        return 0;
    };
    let in_block = |code: &str| ("E0140"..="E0149").contains(&code);
    found
        .iter()
        .filter(|((_, code), _)| code.as_deref().is_some_and(in_block))
        .count() as u32
}

/// The arm's outcome. Without codegen every pair is skipped, which is not a
/// failure; with it, comparing nothing or meeting no build code fails.
pub(crate) fn compile_tally(
    compared: u32,
    divergent: u32,
    build_coded: u32,
    skipped: u32,
) -> CategoryTally {
    let vacuous = skipped == 0 && (compared == 0 || build_coded == 0);
    CategoryTally {
        passed: compared - divergent,
        failed: divergent + u32::from(vacuous),
        skipped,
        ..CategoryTally::default()
    }
}

fn capture(compiler: &Path, tg: &Path, args: &[String], extra: &[&str]) -> Captured {
    let output = Command::new(compiler)
        .arg("compile")
        .arg(tg)
        .args(args)
        .args(extra)
        .output();
    match output {
        Ok(o) => Captured {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => Captured {
            code: -1,
            stdout: String::new(),
            stderr: format!("failed to run compiler: {e}"),
        },
    }
}

/// Run both formats over every non-`json_*` `compile_error/` fixture.
pub(crate) fn run(cli: &Cli, codegen_available: bool) -> CategoryTally {
    println!("[json-parity: compile]");
    let fixtures: Vec<_> = discover_tests(&cli.dir.join("compile_error"))
        .into_iter()
        .filter(|(tg, _)| is_pair_fixture(tg))
        .collect();
    let (mut compared, mut divergent, mut coded, mut skipped) = (0u32, 0u32, 0u32, 0u32);
    for (tg, _) in &fixtures {
        if !codegen_available {
            println!(
                "\x1b[33m⊘\x1b[0m {} (skipped: codegen not available)",
                tg.display()
            );
            skipped += 1;
            continue;
        }
        let args = read_args_file(tg);
        let human = capture(
            &cli.compiler,
            tg,
            &args,
            &["--max-errors", "0", "--no-hints"],
        );
        let json = capture(&cli.compiler, tg, &args, &["--error-format", "json"]);
        compared += 1;
        coded += build_coded(&json);
        match compare_compile(&human, &json) {
            Ok(()) => println!("\x1b[32m✓\x1b[0m {}", tg.display()),
            Err(why) => {
                divergent += 1;
                println!("\x1b[31m✗\x1b[0m {} ({why})", tg.display());
            }
        }
    }
    println!();
    println!("  json-parity compile: {compared} compared, {divergent} divergent");
    if skipped == 0 && coded == 0 {
        println!(
            "  \x1b[31mno record carried a build code: the arm checked no back-end failure\x1b[0m"
        );
    }
    println!();
    compile_tally(compared, divergent, coded, skipped)
}

#[cfg(test)]
#[path = "compile_arm_tests.rs"]
mod tests;
