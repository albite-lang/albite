//! The corpus census (ADR 2.10.26o): AC 1's register diff over injected
//! verdicts, one test per arm of D4's table, and AC 2's walk over a tempdir
//! corpus with an injected register and root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::*;
use crate::doctor::checks::check_kernel_census::corpus::{
    census_corpus, cmd_check_kernel_census_corpus, render_corpus, CorpusRun,
};
use crate::doctor::checks::check_kernel_census::entry_plan::{plan_entries, walk_roots};
use crate::doctor::checks::check_kernel_census::register_diff::{
    register_diff, DiffCategory, FileStatus, NotExaminedWhy, Observation, RegisterDiff,
};
use crate::elaborate::kernel::{Exemption, GateVerdict, Register, RegisterClass, Violation};

const FILE: &str = "tests/golden/check/fixture.tg";

fn row(theorem: &str, class: RegisterClass) -> Exemption {
    Exemption {
        file: FILE.to_string(),
        theorem: theorem.to_string(),
        class,
        removed_by: "2.10.26l".to_string(),
        // The diff never reads a row's `cites`: the gate's verdict carries them.
        cites: Vec::new(),
    }
}

fn refused(class: Option<RegisterClass>) -> GateVerdict {
    GateVerdict::Refused {
        cause: Cause::NeedsConversion,
        class,
    }
}

/// The diff of one theorem in `FILE`, seen once, against `rows`.
fn diff_of(theorem: &str, seen: Observation, rows: &[Exemption]) -> RegisterDiff {
    let mut observed = BTreeMap::new();
    observed.insert(
        (FILE.to_string(), theorem.to_string()),
        vec![("entry.tg".to_string(), seen)],
    );
    let files = BTreeMap::from([(FILE.to_string(), FileStatus::Censused)]);
    register_diff(rows, &observed, &files)
}

pub(super) fn categories(diff: &RegisterDiff) -> Vec<DiffCategory> {
    diff.lines.iter().map(|line| line.category).collect()
}

fn decided(verdict: GateVerdict) -> Observation {
    Observation::Decided(verdict)
}

/// 2.10.26o AC1: `Exempt` is *matched*, and a clean diff reads `matches`.
#[test]
fn an_exempt_verdict_is_matched() {
    let rows = [row("t", RegisterClass::NeedsDelta)];
    let diff = diff_of(
        "t",
        decided(GateVerdict::Exempt(RegisterClass::NeedsDelta)),
        &rows,
    );
    assert_eq!(categories(&diff), [DiffCategory::Matched]);
    assert_eq!(diff.verdict_line(), "register: matches");
    // Whole, so a zero `entries disagree` count never reaches the line.
    assert_eq!(
        diff.render(),
        "  register:  rows 1 · matched 1 · missing 0 · stale 0 · absent 0 · class mismatch 0 · not examined 0\n  register: matches\n"
    );
}

/// 2.10.26o AC1: a refusal with no row, and `CitesWithoutRow`, are *missing row*.
#[test]
fn a_refusal_or_an_uncited_citation_without_a_row_is_missing() {
    let diff = diff_of("t", decided(refused(Some(RegisterClass::NeedsDelta))), &[]);
    assert_eq!(categories(&diff), [DiffCategory::MissingRow]);
    assert_eq!(diff.lines[0].detail, "needs-delta");
    let cites = Violation::CitesWithoutRow {
        cited: vec!["two_is_two".to_string()],
    };
    let diff = diff_of("t", decided(GateVerdict::RegisterViolation(cites)), &[]);
    assert_eq!(categories(&diff), [DiffCategory::MissingRow]);
    assert_eq!(diff.verdict_line(), "register: MISMATCH (1 missing)");
}

/// 2.10.26o AC1: `Stale` and `CitesCleared` are *stale*.
#[test]
fn stale_and_cleared_rows_are_stale() {
    let rows = [row("t", RegisterClass::NeedsDelta)];
    let diff = diff_of(
        "t",
        decided(GateVerdict::RegisterViolation(Violation::Stale)),
        &rows,
    );
    assert_eq!(categories(&diff), [DiffCategory::Stale]);
    assert_eq!(
        diff.lines[0].render(),
        "tests/golden/check/fixture.tg::t → stale (admitted)"
    );
    let rows = [row("t", RegisterClass::CitesExempt)];
    let cleared = decided(GateVerdict::RegisterViolation(Violation::CitesCleared));
    assert_eq!(
        categories(&diff_of("t", cleared, &rows)),
        [DiffCategory::Stale]
    );
}

/// 2.10.26o AC1: a classless refusal with a row, `Drifted` and `CitesMismatch`
/// are all *class mismatch*.
#[test]
fn every_disagreeing_class_is_a_class_mismatch() {
    let rows = [row("t", RegisterClass::NeedsDelta)];
    let diff = diff_of("t", decided(refused(None)), &rows);
    assert_eq!(categories(&diff), [DiffCategory::ClassMismatch]);
    assert_eq!(diff.lines[0].detail, "needs-conversion");
    let drifted = Violation::Drifted {
        row: RegisterClass::NeedsDelta,
        observed: "named-type".to_string(),
    };
    let diff = diff_of("t", decided(GateVerdict::RegisterViolation(drifted)), &rows);
    assert_eq!(categories(&diff), [DiffCategory::ClassMismatch]);
    assert_eq!(diff.lines[0].detail, "named-type");
    let rows = [row("t", RegisterClass::CitesExempt)];
    let mismatch = Violation::CitesMismatch {
        listed: vec!["a".to_string()],
        cited: vec!["b".to_string()],
    };
    let diff = diff_of(
        "t",
        decided(GateVerdict::RegisterViolation(mismatch)),
        &rows,
    );
    assert_eq!(categories(&diff), [DiffCategory::ClassMismatch]);
    assert_eq!(diff.lines[0].detail, "cites b");
}

/// 2.10.26o AC1: a row in a census-ed file whose theorem never appeared is
/// *absent*, and MISMATCH outranks INCOMPLETE.
#[test]
fn a_missing_theorem_is_absent_and_mismatch_outranks_incomplete() {
    let mut elsewhere = row("u", RegisterClass::NeedsDelta);
    elsewhere.file = "elsewhere.tg".to_string();
    let rows = [row("gone", RegisterClass::NeedsDelta), elsewhere];
    let diff = diff_of("t", decided(GateVerdict::Pass), &rows);
    assert_eq!(
        categories(&diff),
        [
            DiffCategory::NotExamined(NotExaminedWhy::OutsideCorpus),
            DiffCategory::Absent
        ]
    );
    assert_eq!(diff.verdict_line(), "register: MISMATCH (1 absent)");
    let rendered = diff.render();
    assert!(rendered.contains("    tests/golden/check/fixture.tg::gone → absent (no such theorem)"));
    assert!(rendered.contains("    elsewhere.tg::u → not examined (outside the corpus)"));
}

/// 2.10.26o AC1: a row is *not examined* when its file is outside the corpus,
/// does not elaborate or is a companion, or its theorem is a hole or a
/// tactic proof; alone, that reads INCOMPLETE.
#[test]
fn rows_the_run_could_not_reach_are_not_examined() {
    let rows = [row("t", RegisterClass::NeedsDelta)];
    let not_examined = |diff: &RegisterDiff| match categories(diff)[..] {
        [DiffCategory::NotExamined(why)] => why,
        _ => panic!("{diff:?}"),
    };
    let hole = diff_of("t", decided(GateVerdict::SorryStatus), &rows);
    assert_eq!(not_examined(&hole), NotExaminedWhy::Hole);
    let tactic = diff_of("t", Observation::Tactic, &rows);
    assert_eq!(not_examined(&tactic), NotExaminedWhy::Tactic);
    assert_eq!(
        tactic.verdict_line(),
        "register: INCOMPLETE (1 row(s) not examined)"
    );
    assert!(tactic
        .render()
        .contains("fixture.tg::t → not examined (tactic)"));
    for (status, why) in [
        (
            FileStatus::DoesNotElaborate,
            NotExaminedWhy::DoesNotElaborate,
        ),
        (FileStatus::Companion, NotExaminedWhy::Companion),
    ] {
        let files = BTreeMap::from([(FILE.to_string(), status)]);
        let diff = register_diff(&rows, &BTreeMap::new(), &files);
        assert_eq!(not_examined(&diff), why);
    }
    let outside = register_diff(&rows, &BTreeMap::new(), &BTreeMap::new());
    assert_eq!(not_examined(&outside), NotExaminedWhy::OutsideCorpus);
}

/// 2.10.26o AC1: `Pass`, and a `SorryStatus` with no row, are in no category;
/// two entries that disagree are both named.
#[test]
fn pass_and_an_unregistered_hole_are_in_no_category() {
    assert!(diff_of("t", decided(GateVerdict::Pass), &[])
        .lines
        .is_empty());
    let hole = diff_of("t", decided(GateVerdict::SorryStatus), &[]);
    assert!(hole.lines.is_empty());
    assert!(diff_of("t", Observation::Tactic, &[]).lines.is_empty());

    let mut observed = BTreeMap::new();
    observed.insert(
        (FILE.to_string(), "t".to_string()),
        vec![
            ("a.tg".to_string(), decided(GateVerdict::Pass)),
            ("b.tg".to_string(), decided(refused(None))),
        ],
    );
    let diff = register_diff(&[], &observed, &BTreeMap::new());
    assert_eq!(categories(&diff), [DiffCategory::EntriesDisagree; 2]);
    assert_eq!(diff.lines[0].detail, "via a.tg, no category");
    assert_eq!(
        diff.lines[1].detail,
        "via b.tg, missing row: needs-conversion"
    );
    assert_eq!(
        diff.verdict_line(),
        "register: MISMATCH (2 entries disagree)"
    );
    assert!(diff
        .render()
        .contains("not examined 0 · entries disagree 2\n"));
}

// ── AC 2: the walk over a tempdir corpus ──

/// 2.10.26o AC2: modules, companions and hidden files are never entries.
#[test]
fn the_plan_keeps_only_entry_files() {
    let (_dir, root) = corpus();
    let candidates = walk_roots(&[root.clone()]).expect("walks");
    assert!(!candidates
        .iter()
        .any(|c| c.starts_with(root.join(".hidden"))));
    let plan = plan_entries(&candidates);
    let names: Vec<String> = plan
        .entries
        .iter()
        .map(|e| {
            e.strip_prefix(&root)
                .expect("under root")
                .display()
                .to_string()
        })
        .collect();
    assert_eq!(
        names,
        [
            "app/main.tg",
            "bad/broken.tg",
            "far/main.tg",
            "plain/empty.tg",
            "shared/a.tg",
            "shared/b.tg"
        ]
    );
    assert_eq!(plan.companions, [root.join("shared/common_test.tg")]);
    assert_eq!(plan.trees[&root.join("app/main.tg")].len(), 3);
}

/// 2.10.26o AC2: the counts, the once-per-entry census, and a nested refusal
/// keyed by its defining file — *missing row*, since ADR 2.10.26l left no
/// real refusal of an admitted class (`matched` is pinned above, by hand).
#[test]
fn the_corpus_counts_every_outcome_and_keys_a_nested_refusal() {
    let (_dir, root) = corpus();
    let report = census_corpus(&[root.clone()], false, &Register::EMPTY, &root).expect("a census");
    assert_eq!(report.files.len(), 6, "each entry once: {:?}", report.files);
    let rendered = render_corpus(&report, true);
    assert!(
        rendered.contains(
            "6 files: 4 census-ed, 1 empty, 1 not elaborating; 1 refusals (named-type-reference 1)\n"
        ),
        "{rendered}"
    );
    assert!(rendered.contains("  plain/empty.tg: empty\n"), "{rendered}");
    assert!(
        // The cold run's own first error, not the rendered run's placeholder.
        rendered.contains("  bad/broken.tg: does not elaborate: compilation failed: error[E0010]"),
        "{rendered}"
    );
    assert!(rendered.contains(&format!("keyed:     against {}", root.display())));
    let line = report.diff.lines[0].render();
    assert_eq!(
        line,
        "app/util/deep/mod.tg::px_refl → missing row (named-type)"
    );
    assert_eq!(report.diff.verdict_line(), "register: MISMATCH (1 missing)");
    // The cache root is an entry's parent directory (P0.1).
    assert!(
        !root.join("app/.tungsten").exists(),
        "the run wrote a cache"
    );
}

/// 2.10.26o AC2: a row in a file that does not elaborate is *not examined*,
/// never *stale*.
#[test]
fn a_row_in_a_broken_file_is_not_examined() {
    let (_dir, root) = corpus();
    let register = register(&needs_delta_row("bad/broken.tg", "lost"));
    let report = census_corpus(&[root.clone()], false, &register, &root).expect("a census");
    assert_eq!(
        categories(&report.diff),
        [
            DiffCategory::MissingRow,
            DiffCategory::NotExamined(NotExaminedWhy::DoesNotElaborate)
        ]
    );
    // The record theorem is the one mismatch; the broken file's row is not
    // examined, never stale.
    assert_eq!(report.diff.verdict_line(), "register: MISMATCH (1 missing)");
}

/// 2.10.26o AC2: an empty root and a missing root both exit 2.
#[test]
fn an_empty_or_missing_root_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), ".hidden/only.tg", "fn main() -> Nat { 0 }\n");
    assert_eq!(
        cmd_check_kernel_census_corpus(&[dir.path().to_path_buf()], false, false),
        ExitCode::from(2)
    );
    let missing = dir.path().join("no_such_root");
    assert_eq!(
        cmd_check_kernel_census_corpus(&[missing.clone()], false, false),
        ExitCode::from(2)
    );
    let refusal =
        census_corpus(&[missing], false, &Register::EMPTY, dir.path()).expect_err("refused");
    assert!(refusal.contains("no_such_root"), "{refusal}");
}

/// 9.10.26p D4: a file one entry reaches by two import paths is observed once
/// per path; the identical pair is dropped, a differing one is kept.
#[test]
fn a_twice_reached_theorem_is_observed_once_per_verdict() {
    let mut run = CorpusRun::default();
    run.record(FILE, "t", "entry.tg", decided(GateVerdict::Pass));
    run.record(FILE, "t", "entry.tg", decided(GateVerdict::Pass));
    let key = (FILE.to_string(), "t".to_string());
    assert_eq!(run.observed[&key].len(), 1);
    run.record(FILE, "t", "entry.tg", decided(refused(None)));
    run.record(FILE, "t", "other.tg", decided(GateVerdict::Pass));
    assert_eq!(run.observed[&key].len(), 3);
}
