//! The `json-parity` category (ADR 28.9.26e §2.4): every `error/` fixture
//! checked under both `--error-format`s, failing on any difference in what
//! the two report — and, since ADR 29.9.26f D7, on a located record with no
//! code (rule 4).
//!
//! It has no directory of its own — the generic category path would find no
//! `json-parity/`, print `(no tests)` and pass — so `run_category` dispatches
//! here, and the category fails outright when it compared nothing. The
//! decision is [`compare`], a pure function over two captured runs; [`run`]
//! only spawns the compiler and prints. Since ADR 30.9.26j the category also
//! has a `compile` arm, over `compile_error/` (`compile_arm.rs`).

use std::path::Path;
use std::process::Command;

use crate::compare::strip_ansi;
use crate::discover::discover_tests;
use crate::run::CategoryTally;
use crate::Cli;

mod compile_arm;

/// One compiler run: its exit code and both streams, as captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Captured {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A diagnostic's `(level, code)` — what the two formats must agree on.
type LevelCode = (String, Option<String>);

/// The `(level, code)` of a human header line, if the line is one.
///
/// A header is, ANSI-stripped, `^(?:\[([EW]\d{4})\] )?(Error|Warning): ` and
/// nothing else counts — a parse error's bare `Error:` is `(error, None)`, and
/// `[E0062] Warning:` is a warning whatever its code's prefix says.
fn header(line: &str) -> Option<LevelCode> {
    let (code, rest) = match line.strip_prefix('[') {
        Some(tail) => {
            let (code, rest) = tail.split_at_checked(5)?;
            let digits = code.get(1..)?;
            let well_formed = matches!(code.as_bytes()[0], b'E' | b'W')
                && digits.bytes().all(|b| b.is_ascii_digit());
            (
                Some(code.to_string()),
                rest.strip_prefix("] ").filter(|_| well_formed)?,
            )
        }
        None => (None, line),
    };
    let level = if rest.starts_with("Error: ") {
        "error"
    } else if rest.starts_with("Warning: ") {
        "warning"
    } else {
        return None;
    };
    Some((level.to_string(), code))
}

/// Every header on a stream, ANSI-stripped.
fn headers(stderr: &str) -> Vec<LevelCode> {
    strip_ansi(stderr).lines().filter_map(header).collect()
}

/// A pipeline/I/O failure's `error: <message>` line — not the summary's.
fn has_failure_line(stderr: &str) -> bool {
    strip_ansi(stderr)
        .lines()
        .any(|l| l.starts_with("error: ") && !l.starts_with("error: aborting due to"))
}

/// The JSON run's records as `(level, code, has_span)`, or why they are not.
fn records(stdout: &str) -> Result<Vec<(LevelCode, bool)>, String> {
    stdout
        .lines()
        .map(|line| {
            let record: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| format!("stdout line is not JSON ({e}): {line}"))?;
            if record["schema"] != 1 {
                return Err(format!("record without \"schema\": 1: {line}"));
            }
            let level = record["level"].as_str().unwrap_or_default().to_string();
            let code = record["code"].as_str().map(str::to_string);
            Ok(((level, code), !record["primary_span"].is_null()))
        })
        .collect()
}

fn sorted(mut pairs: Vec<LevelCode>) -> Vec<LevelCode> {
    pairs.sort();
    pairs
}

/// Why a fixture's two runs diverge, or `Ok` when they agree (§2.4's three
/// conditions and the exit-3 rule).
pub(crate) fn compare(human: &Captured, json: &Captured) -> Result<(), String> {
    if human.code != json.code {
        return Err(format!(
            "exit {} (human) vs {} (json)",
            human.code, json.code
        ));
    }
    let found = records(&json.stdout)?;
    if json.code != 0 && !found.iter().any(|((level, _), _)| level == "error") {
        return Err(format!("exit {} with no error record", json.code));
    }
    let json_stderr = strip_ansi(&json.stderr);
    if let Some(line) = json_stderr
        .lines()
        .find(|l| header(l).is_some() || l.starts_with("error: aborting due to"))
    {
        return Err(format!("human rendering on JSON-mode stderr: {line}"));
    }
    if human.code == 3 {
        let one_failure = matches!(found.as_slice(), [((_, None), false)]);
        return match (has_failure_line(&human.stderr), one_failure) {
            (true, true) => Ok(()),
            (false, _) => Err("exit 3 with no `error: ` line on human stderr".to_string()),
            (true, false) => Err("exit 3 needs exactly one code-less, span-less record".into()),
        };
    }
    // Rule 4 (ADR 29.9.26f D7): a located record always has a code. Checked
    // before the multiset, which two renderers regressing together would pass.
    if found
        .iter()
        .any(|((_, code), located)| *located && code.is_none())
    {
        return Err("rule 4: located record without a code".to_string());
    }
    let from_json = sorted(found.into_iter().map(|(pair, _)| pair).collect());
    let from_human = sorted(headers(&human.stderr));
    if from_json != from_human {
        return Err(format!("human {from_human:?} vs json {from_json:?}"));
    }
    Ok(())
}

/// How many of a JSON run's located records carry a syntax-block code
/// (E0110–E0117) — rule 4's vacuity floor (ADR 29.9.26f §2.3). Unparseable
/// output counts nothing; [`compare`] reports it.
pub(crate) fn syntax_coded(json: &Captured) -> u32 {
    let Ok(found) = records(&json.stdout) else {
        return 0;
    };
    let in_block = |code: &str| ("E0110"..="E0117").contains(&code);
    found
        .iter()
        .filter(|((_, code), located)| *located && code.as_deref().is_some_and(in_block))
        .count() as u32
}

/// The category's outcome. It fails when nothing was compared — a vacuous
/// green is the failure F9 warned about — when fewer fixtures were compared
/// than the `error` category discovered, or when no located record carried a
/// syntax-block code, which would leave rule 4 checking no parse error at all.
pub(crate) fn tally(
    compared: u32,
    divergent: u32,
    discovered: u32,
    syntax_coded: u32,
) -> CategoryTally {
    let incomplete = compared == 0 || compared != discovered || syntax_coded == 0;
    CategoryTally {
        passed: compared - divergent,
        failed: divergent + u32::from(incomplete),
        ..CategoryTally::default()
    }
}

fn capture(compiler: &Path, tg: &Path, extra: &[&str]) -> Captured {
    let output = Command::new(compiler)
        .arg("check")
        .arg(tg)
        .arg("--no-cache")
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

/// Run the `check` arm, then the `compile` arm (ADR 30.9.26j D10), and fold
/// their tallies into the category's.
pub(crate) fn run(cli: &Cli, codegen_available: bool) -> CategoryTally {
    let mut total = run_check_arm(cli);
    total.add(compile_arm::run(cli, codegen_available));
    total
}

/// Run both formats cold over every `error/` fixture, ignoring `.args`.
fn run_check_arm(cli: &Cli) -> CategoryTally {
    println!("[json-parity]");
    let fixtures = discover_tests(&cli.dir.join("error"));
    let (mut compared, mut divergent, mut coded) = (0u32, 0u32, 0u32);
    for (tg, _) in &fixtures {
        let human = capture(&cli.compiler, tg, &["--max-errors", "0", "--no-hints"]);
        let json = capture(&cli.compiler, tg, &["--error-format", "json"]);
        compared += 1;
        coded += syntax_coded(&json);
        match compare(&human, &json) {
            Ok(()) => println!("\x1b[32m✓\x1b[0m {}", tg.display()),
            Err(why) => {
                divergent += 1;
                println!("\x1b[31m✗\x1b[0m {} ({why})", tg.display());
            }
        }
    }
    let discovered = fixtures.len() as u32;
    println!();
    println!("  json-parity: {compared} compared, {divergent} divergent");
    if compared == 0 || compared != discovered {
        println!("  \x1b[31mcompared {compared} of {discovered} error fixtures\x1b[0m");
    }
    if coded == 0 {
        println!("  \x1b[31mno located record carried a syntax code: rule 4 checked no parse error\x1b[0m");
    }
    println!();
    tally(compared, divergent, discovered, coded)
}

#[cfg(test)]
mod tests;
