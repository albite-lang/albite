//! Tests for the native core's exemption from the corpus walker (ADR
//! 28.10.26e D1–D3): skipped by unit, counted apart, never silent.

use std::path::PathBuf;

use super::fixtures::corpus_at as corpus;
use super::*;

const CORE_LL: &str = "__native_core/native_core.ll";

/// The relative paths `scan_ll_corpus` handed to its visitor, sorted.
fn visited(dir: &Path) -> (CorpusScan, Vec<PathBuf>) {
    let mut seen = Vec::new();
    let scan = scan_ll_corpus(dir, |path, _| {
        seen.push(path.strip_prefix(dir).expect("under dir").to_path_buf());
    });
    (scan, seen)
}

// 28.10.26e AC1
#[test]
fn the_core_unit_is_not_visited_and_is_named() {
    let dir = corpus(&[("m0.ll", "define void @f() {\n}\n"), (CORE_LL, "core")]);
    let (scan, seen) = visited(dir.path());
    assert_eq!(seen, vec![PathBuf::from("m0.ll")], "only the program file");
    assert_eq!(scan.files, 1, "a skipped file does not count toward files");
    assert_eq!(scan.skipped_optimised, vec![PathBuf::from(CORE_LL)]);
    assert!(scan.unreadable.is_empty());
}

// 28.10.26e AC2 — D3: the exemption cannot turn an emit with no program IR
// into a pass.
#[test]
fn a_core_only_corpus_is_still_empty() {
    let dir = corpus(&[(CORE_LL, "core")]);
    let (scan, seen) = visited(dir.path());
    assert!(seen.is_empty());
    assert_eq!(scan.files, 0);
    assert_eq!(scan.skipped_optimised.len(), 1);
    for strict in [false, true] {
        assert_eq!(
            AuditVerdict::classify(&scan, false, 0, strict),
            AuditVerdict::EmptyCorpus,
            "strict={strict}"
        );
    }
}

// 28.10.26e AC3 — the must-fail twin against an over-broad match: only the
// top-level directory component is the unit.
#[test]
fn only_the_top_level_unit_directory_matches() {
    let near_misses = [
        "x/__native_core/a.ll",
        "__native_core_x/a.ll",
        "__native_core.ll",
        "a__native_core/b.ll",
    ];
    let mut files: Vec<(&str, &str)> = near_misses.iter().map(|path| (*path, "ir")).collect();
    files.push((CORE_LL, "core"));
    let dir = corpus(&files);
    let (scan, seen) = visited(dir.path());

    let mut expected: Vec<PathBuf> = near_misses.iter().map(PathBuf::from).collect();
    expected.sort();
    let mut seen_sorted = seen.clone();
    seen_sorted.sort();
    assert_eq!(seen_sorted, expected, "every near miss is audited");
    assert_eq!(scan.files, near_misses.len());
    assert_eq!(scan.skipped_optimised, vec![PathBuf::from(CORE_LL)]);
}

#[test]
fn a_nested_core_file_is_skipped_too() {
    let dir = corpus(&[("m0.ll", "ir"), ("__native_core/sub/extra.ll", "core")]);
    let (scan, _) = visited(dir.path());
    assert_eq!(
        scan.skipped_optimised,
        vec![PathBuf::from("__native_core/sub/extra.ll")]
    );
}

#[test]
fn a_path_outside_the_scanned_directory_is_never_the_unit() {
    let dir = Path::new("/corpus");
    assert!(is_optimised_unit(
        dir,
        Path::new("/corpus/__native_core/a.ll")
    ));
    assert!(!is_optimised_unit(
        dir,
        Path::new("/elsewhere/__native_core/a.ll")
    ));
    assert!(!is_optimised_unit(dir, Path::new("/corpus/__native_core")));
}

// 28.10.26e AC4 — D2: skipped is never silent.
#[test]
fn the_skip_report_is_empty_without_a_skip_and_names_each_path() {
    assert!(CorpusScan::default().skip_report().is_empty());

    let scan = CorpusScan {
        files: 3,
        skipped_optimised: vec![PathBuf::from(CORE_LL), PathBuf::from("__native_core/b.ll")],
        ..CorpusScan::default()
    };
    let report = scan.skip_report();
    assert_eq!(report.len(), 1, "one shared line: {report:?}");
    let line = &report[0];
    assert!(
        line.starts_with("note: skipped 2 optimised unit file(s) ("),
        "{line}"
    );
    assert!(
        line.contains(&format!("({CORE_LL}, __native_core/b.ll)")),
        "{line}"
    );
    // What covers it instead, so the note is an answer rather than a gap.
    assert!(line.contains("compile/native_core"), "{line}");
    assert!(line.contains("check-native-core-*"), "{line}");
    assert!(line.contains("28.10.26e"), "{line}");
}

#[test]
fn report_skipped_is_callable_with_and_without_a_skip() {
    CorpusScan::default().report_skipped();
    CorpusScan {
        skipped_optimised: vec![PathBuf::from(CORE_LL)],
        ..CorpusScan::default()
    }
    .report_skipped();
}
