//! Tests: every spelling ADR 19.8.26a moved still parses, grouped and flat.
//!
//! Driven off the real [`Cli`] root rather than a synthetic wrapper, because
//! the top-level namespace *is* the subject here — a `Commands` variant that
//! parses in isolation but collides at the root would pass a wrapper test.
//!
//! One table per direction, so a group added without its alias — or an alias
//! that silently lost an argument — fails here rather than in whichever caller
//! still spells it the old way. A hidden variant appears in no `--help` a human
//! reads, so a test is the only thing that keeps it true.

use clap::{CommandFactory, Parser};

use super::Cli;
use albite_bootstrap::driver::diagnostics::ErrorFormat;

fn parses(args: &[&str]) -> bool {
    Cli::try_parse_from(std::iter::once("tungsten").chain(args.iter().copied())).is_ok()
}

/// Grouped path ⇄ the flat spelling it replaced. Both must parse, forever.
const REGROUPED: &[(&[&str], &[&str])] = &[
    (&["expr", "eval", "1 + 2"], &["eval", "1 + 2"]),
    (&["expr", "repl"], &["repl"]),
    // `clean-project` is the one move that is also a RENAME, so a regression
    // here reads as "command not found" rather than as a relocated path.
    (
        &["cache", "clean-project", "main.tg"],
        &["clean", "main.tg"],
    ),
    (&["cache", "clean-project"], &["clean"]),
    (
        &["info", "type", "members", "constructors", "Option", "o.tg"],
        &["info", "type", "constructors", "Option", "o.tg"],
    ),
    (
        &["info", "type", "members", "field-type", "L.Cons.0", "l.tg"],
        &["info", "type", "field-type", "L.Cons.0", "l.tg"],
    ),
    (
        &["info", "type", "members", "record-fields", "Point", "h.tg"],
        &["info", "type", "record-fields", "Point", "h.tg"],
    ),
    (
        &["info", "type", "members", "visibility", "Token", "o.tg"],
        &["info", "type", "visibility", "Token", "o.tg"],
    ),
];

/// Spellings this ADR did NOT create, which must still be refused.
///
/// The table above is entirely positive, and a table of things that must
/// succeed cannot distinguish a working command tree from one that accepts
/// everything — `parses` hardwired to `true` satisfies every assertion in this
/// file, which is exactly what the mutation sweep found. So the helper's
/// contract is asserted in the other direction too, and the cases are chosen to
/// be the ones a regrouping can plausibly get wrong rather than arbitrary
/// garbage.
const MUST_NOT_PARSE: &[(&[&str], &str)] = &[
    // The re-home must MOVE `clean`, not copy it. A `clean-project` that also
    // resolved at top level would put that namespace straight back to 13 while
    // every functional test stayed green — the one regression `cli-surface`
    // would catch and nothing here would explain.
    //
    // The SECOND operand is load-bearing and must not be dropped: `tungsten`
    // carries an implicit `[FILE]` positional, so a bare `albite
    // clean-project` parses happily — as a request to check a file of that
    // name — and fails only when the driver cannot read it. One argument
    // asserts nothing; two make clap reject the extra and the refusal
    // observable.
    (
        &["clean-project", "main.tg"],
        "`clean-project` must exist only under `cache`",
    ),
    // `members` groups under `type`, not under `info`. The flat aliases this
    // ADR kept are the OLD spellings; inventing a new shorter one would teach a
    // path `--help` never shows.
    (
        &["info", "members", "constructors", "Option", "o.tg"],
        "`members` is a sub-namespace of `info type`, not of `info`",
    ),
    // A near-miss name inside a new namespace. If this parsed, the positive
    // table would be asserting nothing about the namespace's contents.
    (
        &["expr", "evaluate", "1 + 2"],
        "`expr` must not accept a subcommand it does not have",
    ),
    // `cache clean-project` takes at most one operand — the entry file whose
    // project cache to clear. Two would mean the re-home widened the surface.
    (
        &["cache", "clean-project", "a.tg", "b.tg"],
        "`cache clean-project` takes at most one entry file",
    ),
];

#[test]
fn both_spellings_parse_for_every_regrouped_command() {
    for (grouped, flat) in REGROUPED {
        assert!(parses(grouped), "grouped path stopped parsing: {grouped:?}");
        assert!(parses(flat), "hidden alias stopped parsing: {flat:?}");
    }
}

/// Without this, every other assertion in this file is satisfied by a command
/// tree that accepts anything at all.
#[test]
fn the_spellings_this_adr_did_not_create_are_still_refused() {
    for (args, why) in MUST_NOT_PARSE {
        assert!(!parses(args), "{why} — but {args:?} parsed");
    }
}

/// A flag on one spelling and not the other is the drift a parse-only table
/// would otherwise miss, since both halves still parse without it.
#[test]
fn flags_survive_the_move_on_both_spellings() {
    for (grouped, flat) in [
        (
            &[
                "info",
                "type",
                "members",
                "constructors",
                "L",
                "l.tg",
                "--raw",
            ][..],
            &["info", "type", "constructors", "L", "l.tg", "--raw"][..],
        ),
        (
            &["cache", "clean", "--dry-run"][..],
            &["cache", "clean", "--dry-run"][..],
        ),
    ] {
        assert!(
            parses(grouped),
            "flag lost on the grouped path: {grouped:?}"
        );
        assert!(parses(flat), "flag lost on the alias: {flat:?}");
    }
}

/// The aliases must stay *hidden*: a visible one would be counted by
/// `cli-surface` and hand back the headroom this ADR bought.
#[test]
fn every_alias_this_adr_kept_is_hidden_from_the_visible_surface() {
    let visible: Vec<String> = crate::list_commands::leaf_paths(&Cli::command());
    for gone in [
        "eval",
        "repl",
        "clean",
        "info type constructors",
        "info type field-type",
        "info type record-fields",
        "info type visibility",
    ] {
        assert!(
            !visible.contains(&gone.to_string()),
            "`{gone}` is still a VISIBLE leaf — it counts against the namespace \
             cap ADR 19.8.26a paid down, and `--help` now teaches two spellings"
        );
    }
}

/// The grouped paths, conversely, must be visible — an alias-only move would
/// leave the command reachable and undiscoverable at once.
#[test]
fn every_grouped_path_this_adr_added_is_visible() {
    let visible: Vec<String> = crate::list_commands::leaf_paths(&Cli::command());
    for added in [
        "expr eval",
        "expr repl",
        "cache clean-project",
        "info type members constructors",
        "info type members field-type",
        "info type members record-fields",
        "info type members visibility",
    ] {
        assert!(
            visible.contains(&added.to_string()),
            "`{added}` is not a visible leaf — `albite commands --tree` and \
             `--help` would not offer the spelling the surfaces now teach"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// check --error-format (ADR 28.9.26e AC3)
// ─────────────────────────────────────────────────────────────────────

fn check_error_format(args: &[&str]) -> Result<ErrorFormat, clap::Error> {
    let cli = Cli::try_parse_from(
        ["tungsten", "check", "f.tg"]
            .into_iter()
            .chain(args.iter().copied()),
    )?;
    match cli.command {
        Some(super::Commands::Check {
            error_format, json, ..
        }) => Ok(error_format.resolve(json)),
        _ => panic!("`check` parsed as another command"),
    }
}

#[test]
fn check_error_format_defaults_to_human() {
    assert_eq!(check_error_format(&[]).unwrap(), ErrorFormat::Human);
}

#[test]
fn every_json_spelling_selects_json() {
    for spelling in [
        &["--error-format=json"][..],
        &["--error-format", "json"],
        &["--json"],
    ] {
        assert_eq!(
            check_error_format(spelling).unwrap(),
            ErrorFormat::Json,
            "{spelling:?}"
        );
    }
    assert_eq!(
        check_error_format(&["--error-format=human"]).unwrap(),
        ErrorFormat::Human
    );
}

#[test]
fn an_unknown_error_format_is_rejected() {
    let err = check_error_format(&["--error-format=xml"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

// ADR 29.9.26f AC6: `run` takes `--error-format`, defaults to human, and has
// no `--json` alias; `compile` has no `--error-format`.
fn run_error_format(args: &[&str]) -> Result<ErrorFormat, clap::Error> {
    let cli = Cli::try_parse_from(
        ["tungsten", "run", "f.tg"]
            .into_iter()
            .chain(args.iter().copied()),
    )?;
    match cli.command {
        Some(super::Commands::Run { error_format, .. }) => Ok(error_format.resolve(false)),
        _ => panic!("`run` parsed as another command"),
    }
}

// 29.9.26f AC6
#[test]
fn run_error_format_defaults_to_human_and_accepts_json() {
    assert_eq!(run_error_format(&[]).unwrap(), ErrorFormat::Human);
    assert_eq!(
        run_error_format(&["--error-format", "json"]).unwrap(),
        ErrorFormat::Json
    );
    assert_eq!(
        run_error_format(&["--error-format=human"]).unwrap(),
        ErrorFormat::Human
    );
}

// 29.9.26f AC6
#[test]
fn run_has_no_json_alias() {
    let err = run_error_format(&["--json"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
}

// 20.10.26b AC1: `-h` prints `about`, `--help` prints `long_about`, so the
// short form must lead with the new name and the long one must carry the old.
// 27.10.26f AC3: the old name says whose it was, and where it lived.
#[test]
fn root_help_names_albite_formerly_tungsten() {
    let mut cmd = Cli::command();
    let short = cmd.render_help().to_string();
    assert!(
        short.starts_with("The Albite proof language compiler"),
        "{short}"
    );
    let long = cmd.render_long_help().to_string();
    let owner = "formerly Tungsten by Spinning Cat Studios";
    let repo = "Former repository: https://github.com/Spinning-Cat-Studios/tungsten";
    assert!(long.contains(owner) && long.contains(repo), "{long}");
}

// 29.9.26f AC6
#[test]
fn run_help_names_stderr_as_the_record_stream() {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let run = cmd.find_subcommand_mut("run").expect("run exists");
    let help = run.render_long_help().to_string();
    assert!(help.contains("--error-format"), "{help}");
    assert!(help.contains("to stderr"), "{help}");
}

// 30.9.26j AC7 (inverts 29.9.26f AC6's `compile_has_no_error_format`). The
// parse is inlined rather than a helper: a non-`#[test]` helper here would be
// mutated by the codegen-less mutation lane, which cannot compile it.
#[cfg(feature = "codegen")]
#[test]
fn compile_accepts_error_format_json() {
    let error_format = |args: &[&str]| -> Result<super::ErrorFormatArg, clap::Error> {
        let argv: Vec<&str> = ["tungsten", "compile", "f.tg"]
            .into_iter()
            .chain(args.iter().copied())
            .collect();
        match Cli::try_parse_from(argv)?.command {
            Some(super::Commands::Compile { error_format, .. }) => Ok(error_format),
            _ => panic!("expected the compile subcommand"),
        }
    };
    assert_eq!(
        error_format(&["--error-format", "json"]).unwrap(),
        super::ErrorFormatArg::Json
    );
    assert_eq!(error_format(&[]).unwrap(), super::ErrorFormatArg::Human);
    let err = error_format(&["--json"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
}

// 30.9.26j AC7
#[cfg(feature = "codegen")]
#[test]
fn compile_help_names_stderr_as_the_record_stream() {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let compile = cmd.find_subcommand_mut("compile").expect("compile exists");
    let help = compile.render_long_help().to_string();
    assert!(help.contains("--error-format"), "{help}");
    assert!(help.contains("to stderr"), "{help}");
}

fn test_format(args: &[&str]) -> Result<super::TestFormatArg, clap::Error> {
    let argv: Vec<&str> = ["tungsten", "test", "f.tg"]
        .into_iter()
        .chain(args.iter().copied())
        .collect();
    match Cli::try_parse_from(argv)?.command {
        Some(super::Commands::Test { format, .. }) => Ok(format),
        _ => panic!("expected the test subcommand"),
    }
}

// 29.9.26i D1: `--format` defaults to human and accepts json in both spellings.
#[test]
fn test_format_defaults_to_human_and_accepts_json() {
    use super::TestFormatArg;
    assert_eq!(test_format(&[]).unwrap(), TestFormatArg::Human);
    assert_eq!(
        test_format(&["--format=json"]).unwrap(),
        TestFormatArg::Json
    );
    assert_eq!(
        test_format(&["--format", "json"]).unwrap(),
        TestFormatArg::Json
    );
    assert_eq!(
        test_format(&["--format=human"]).unwrap(),
        TestFormatArg::Human
    );
    // 30.9.26k AC1: `junit` in both spellings.
    assert_eq!(
        test_format(&["--format=junit"]).unwrap(),
        TestFormatArg::Junit
    );
    assert_eq!(
        test_format(&["--format", "junit"]).unwrap(),
        TestFormatArg::Junit
    );
}

// 30.9.26k AC1: `xml` is still not a value.
#[test]
fn test_format_rejects_xml() {
    let err = test_format(&["--format=xml"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

// 29.9.26i D1: `test` has no `--error-format` and no `--json` — usage errors.
#[test]
fn test_has_no_error_format_or_json_alias() {
    for args in [&["--error-format", "json"][..], &["--json"][..]] {
        let err = test_format(args).unwrap_err();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "{args:?}"
        );
        assert_eq!(err.exit_code(), 2);
    }
}

#[test]
fn json_beside_an_explicit_error_format_is_a_conflict() {
    let err = check_error_format(&["--json", "--error-format", "human"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}
