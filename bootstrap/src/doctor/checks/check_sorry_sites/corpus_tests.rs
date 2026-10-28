//! Tests for `doctor check sorry-sites <dir>` (ADR 15.10.26a).
//!
//! The corpora are tempdirs written here and elaborated for real, because
//! the claim is about what a directory of real entries censuses to; the walk's
//! failure arm is the one place a listing is injected (AC 3a).

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::{
    census_corpus, census_directory, render_corpus, CorpusEntry, CorpusReport, EntryOutcome,
};
use crate::doctor::checks::check_sorry_sites::{cmd_check_sorry_sites, SorryCensus};
use crate::utils::{walk_tg_tree, ListedChild, TgTree};

const CLEAN: &str = "pub fn fine() -> Nat { 1 }\n";
const HOLED: &str = "pub fn holey() -> Nat { sorry }\n";
const BROKEN: &str = "pub fn broken() -> Nat { true }\n";
const SHARED: &str = "pub fn shared_hole() -> Nat { sorry }\n";
const IMPORTS_SHARED: &str = "mod shared;\n\npub fn use_it() -> Nat { shared::shared_hole() }\n";

/// Write each `(relative path, source)` under a fresh tempdir.
fn corpus(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    dir
}

fn report_of(dir: &tempfile::TempDir) -> CorpusReport {
    census_directory(dir.path()).expect("corpus has entries")
}

fn entry<'a>(report: &'a CorpusReport, suffix: &str) -> &'a CorpusEntry {
    report
        .entries
        .iter()
        .find(|e| e.entry.ends_with(suffix))
        .unwrap_or_else(|| panic!("no entry ending {suffix}: {report:#?}"))
}

fn censused(entry: &CorpusEntry) -> &SorryCensus {
    match &entry.outcome {
        EntryOutcome::Censused(census) => census,
        EntryOutcome::DidNotElaborate(why) => panic!("{} did not elaborate: {why}", entry.entry),
    }
}

// 15.10.26a AC1: a directory is censused per entry, nested ones included, and
// the totals are the sums of the per-entry censuses.
#[test]
fn sorry_sites_corpus_censuses_each_entry_and_sums_them() {
    let dir = corpus(&[
        ("clean.tg", CLEAN),
        ("holed.tg", HOLED),
        ("nested/also_clean.tg", CLEAN),
    ]);
    let report = report_of(&dir);
    assert_eq!(report.entries.len(), 3);
    assert_eq!(censused(entry(&report, "holed.tg")).authored, 1);
    assert!(censused(entry(&report, "nested/also_clean.tg"))
        .rows
        .is_empty());
    let summed: usize = report.entries.iter().map(|e| censused(e).authored).sum();
    assert_eq!(report.totals.authored, summed);
    assert_eq!(
        (
            report.totals.read,
            report.totals.elaborated,
            report.totals.with_holes
        ),
        (3, 3, 1)
    );
    let text = render_corpus(&report);
    assert!(
        text.contains("entries: 3 read, 3 elaborated, 1 carry a hole\n"),
        "{text}"
    );
    assert!(
        text.contains("holed.tg  1 of 1 definition(s) carry a sorry: 1 authored, 0 synthesised, 0 unclassified\n"),
        "{text}"
    );
    assert!(
        text.contains("clean.tg  clean (1 definition(s))\n"),
        "{text}"
    );
    assert!(
        text.ends_with("totals (summed over entries): 1 authored, 0 synthesised, 0 unclassified\n"),
        "{text}"
    );
    assert_eq!(report.exit_code(), 0);
}

// 15.10.26a AC2: an entry that does not elaborate is a row with its first
// error header; the entries sorted after it are still censused; exit 1.
#[test]
fn sorry_sites_corpus_reports_a_failing_entry_and_goes_on() {
    let dir = corpus(&[("a_broken.tg", BROKEN), ("b_holed.tg", HOLED)]);
    let report = report_of(&dir);
    let EntryOutcome::DidNotElaborate(header) = &entry(&report, "a_broken.tg").outcome else {
        panic!("a_broken.tg elaborated: {report:#?}");
    };
    assert!(!header.is_empty() && !header.contains('\n'), "{header:?}");
    assert_eq!(censused(entry(&report, "b_holed.tg")).authored, 1);
    assert_eq!(
        (
            report.totals.read,
            report.totals.elaborated,
            report.totals.with_holes
        ),
        (2, 1, 1)
    );
    let text = render_corpus(&report);
    assert!(
        text.contains(&format!("a_broken.tg  did not elaborate: {header}\n")),
        "{text}"
    );
    assert_eq!(report.exit_code(), 1);
    assert_eq!(
        cmd_check_sorry_sites(&dir.path().to_path_buf(), false, false),
        ExitCode::from(1)
    );
}

// 15.10.26a AC3: an empty directory and a missing path both exit 2, naming
// the path; a dot-directory's `.tg` is never read.
#[test]
fn sorry_sites_corpus_refuses_empty_and_missing_paths() {
    let empty = corpus(&[("notes.txt", "not tungsten")]);
    let why = census_directory(empty.path()).unwrap_err();
    assert!(why.contains(&empty.path().display().to_string()), "{why}");
    assert_eq!(
        cmd_check_sorry_sites(&empty.path().to_path_buf(), false, false),
        ExitCode::from(2)
    );
    let missing = empty.path().join("no_such_dir");
    assert_eq!(
        cmd_check_sorry_sites(&missing, false, false),
        ExitCode::from(2)
    );

    let dotted = corpus(&[(".hidden/holed.tg", HOLED), ("clean.tg", CLEAN)]);
    let report = report_of(&dotted);
    assert_eq!(report.entries.len(), 1, "{report:#?}");
    assert!(report.entries[0].entry.ends_with("clean.tg"));
}

/// A listing over an in-memory tree in which `bad` cannot be listed.
fn injected_listing(dir: &Path, root: &Path) -> io::Result<Vec<ListedChild>> {
    let child = |name: &str, is_dir| ListedChild {
        name: name.into(),
        is_dir,
    };
    match dir.strip_prefix(root).unwrap().to_str().unwrap() {
        "" => Ok(vec![
            child("z.tg", false),
            child("bad", true),
            child("good", true),
            child(".git", true),
            child("readme.md", false),
        ]),
        "good" => Ok(vec![child("a.tg", false)]),
        "bad" => Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
        other => panic!("the walk listed {other:?}, which it should have skipped"),
    }
}

// 15.10.26a AC3a: a subdirectory whose listing fails is named under
// `unreadable`, the other entries are still censused, and the exit is 1.
#[test]
fn sorry_sites_corpus_names_an_unreadable_directory() {
    let root = PathBuf::from("/corpus");
    let tree = walk_tg_tree(&root, &mut |dir| injected_listing(dir, &root));
    assert_eq!(
        tree.files,
        vec![PathBuf::from("good/a.tg"), PathBuf::from("z.tg")]
    );
    assert_eq!(
        tree.unreadable,
        vec![(PathBuf::from("/corpus/bad"), "denied".to_string())]
    );
    let mut seen = Vec::new();
    let mut elaborate = |path: &Path| {
        seen.push(path.to_path_buf());
        Ok(SorryCensus {
            examined: 1,
            rows: Vec::new(),
            authored: 0,
            synthesised: 0,
            unclassified: 0,
            rests_on: Vec::new(),
        })
    };
    let report = census_corpus(&root, &tree, &mut elaborate).unwrap();
    assert_eq!(seen, vec![root.join("good/a.tg"), root.join("z.tg")]);
    assert_eq!(report.unreadable.len(), 1);
    assert_eq!(report.unreadable[0].path, "/corpus/bad");
    assert_eq!(report.exit_code(), 1);
    let text = render_corpus(&report);
    assert!(
        text.contains("  unreadable: /corpus/bad: denied\n"),
        "{text}"
    );
    assert!(
        text.contains("entries: 2 read, 2 elaborated, 0 carry a hole\n"),
        "{text}"
    );
}

fn census_with(authored: usize, synthesised: usize, unclassified: usize) -> SorryCensus {
    SorryCensus {
        examined: 1,
        rows: Vec::new(),
        authored,
        synthesised,
        unclassified,
        rests_on: Vec::new(),
    }
}

// 15.10.26a D3: every class is summed over entries, including the two a real
// corpus rarely produces, so the totals line cannot hard-wire them to zero.
#[test]
fn sorry_sites_corpus_sums_every_hole_class() {
    let root = PathBuf::from("/corpus");
    let tree = TgTree {
        files: vec![PathBuf::from("a.tg"), PathBuf::from("b.tg")],
        unreadable: Vec::new(),
    };
    let mut censuses = vec![census_with(1, 2, 3), census_with(4, 5, 6)].into_iter();
    let report = census_corpus(&root, &tree, &mut |_| Ok(censuses.next().unwrap())).unwrap();
    let totals = &report.totals;
    assert_eq!(
        (totals.authored, totals.synthesised, totals.unclassified),
        (5, 7, 9)
    );
    assert!(render_corpus(&report)
        .ends_with("totals (summed over entries): 5 authored, 7 synthesised, 9 unclassified\n"));
}

// 15.10.26a AC3a, the refusal arm: no entry at all, only an unreadable root,
// is a refusal naming both — never an empty success.
#[test]
fn sorry_sites_corpus_with_only_an_unreadable_root_refuses() {
    let root = PathBuf::from("/corpus");
    let tree = TgTree {
        files: Vec::new(),
        unreadable: vec![(root.clone(), "denied".to_string())],
    };
    let why = census_corpus(&root, &tree, &mut |_| unreachable!()).unwrap_err();
    assert!(why.contains("no `.tg` entries under /corpus"), "{why}");
    assert!(why.contains("unreadable: /corpus: denied"), "{why}");
}

fn json_of(report: &CorpusReport) -> serde_json::Value {
    serde_json::to_value(report).unwrap()
}

// 15.10.26a AC3b: the directory JSON is D3a's one object, for a clean corpus,
// a mixed one, and two entries reaching one holed module — counted per entry.
#[test]
fn sorry_sites_corpus_json_has_the_declared_shape() {
    let clean = json_of(&report_of(&corpus(&[("clean.tg", CLEAN)])));
    let mut keys: Vec<&str> = clean
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["entries", "root", "totals", "unreadable"]);
    assert_eq!(clean["totals"]["read"], 1);
    assert_eq!(clean["totals"]["with_holes"], 0);
    assert_eq!(clean["entries"][0]["census"]["examined"], 1);

    let mixed = json_of(&report_of(&corpus(&[("a.tg", BROKEN), ("b.tg", HOLED)])));
    assert!(mixed["entries"][0]["error"].is_string(), "{mixed:#}");
    assert!(mixed["entries"][0].get("census").is_none());
    assert_eq!(mixed["entries"][1]["census"]["authored"], 1);
    for key in [
        "read",
        "elaborated",
        "with_holes",
        "authored",
        "synthesised",
        "unclassified",
    ] {
        assert!(mixed["totals"][key].is_u64(), "totals.{key}: {mixed:#}");
    }
    assert_eq!(mixed["totals"]["elaborated"], 1);

    // `shared.tg` is an entry too, so three entries reach its one hole.
    let shared = report_of(&corpus(&[
        ("left.tg", IMPORTS_SHARED),
        ("right.tg", IMPORTS_SHARED),
        ("shared.tg", SHARED),
    ]));
    for side in ["left.tg", "right.tg", "shared.tg"] {
        assert_eq!(censused(entry(&shared, side)).authored, 1, "{side}");
    }
    assert_eq!(json_of(&shared)["totals"]["authored"], 3);
}
