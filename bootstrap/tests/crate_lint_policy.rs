//! The bootstrap package's lint policy, pinned (ADR 1.10.26j § 2.4).
//!
//! The package builds two targets, and each has its own crate-level list: the
//! library root `src/lib.rs` and the binary root `src/main.rs`. Three tiers,
//! one assertion each: each root's `#![allow(…)]` names exactly its kept
//! families; no module root confines a clippy lint; and every per-item clippy
//! allow sits on one line with a `// Reason:`. A new blanket allow then fails
//! `make test` naming the lint, instead of silently widening what the linter
//! ignores.
//!
//! The parser helpers are a third copy of
//! `albite_runtime/tests/crate_lint_policy.rs`, with a root parameter added
//! (ADR 1.10.26j D8): a shared helper would have to ship as a new public
//! crate. G8's closing slice is the trigger for extracting one.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The library's kept names, by § 2.2 family. Non-clippy names count too:
/// `unknown_lints` was deleted by this ADR and must stay gone.
const LIBRARY_CRATE_LEVEL: [&str; 32] = [
    // Docs
    "clippy::doc_markdown",
    "clippy::missing_errors_doc",
    "clippy::missing_panics_doc",
    // API surface
    "clippy::must_use_candidate",
    // Error size
    "clippy::result_large_err",
    // Size, governed by code-health (permanent)
    "clippy::too_many_lines",
    // Signature ripple
    "clippy::match_same_arms",
    "clippy::manual_let_else",
    "clippy::needless_pass_by_value",
    "clippy::unused_self",
    "clippy::only_used_in_recursion",
    "clippy::ptr_arg",
    "clippy::unnecessary_wraps",
    // Mechanical style
    "clippy::uninlined_format_args",
    "clippy::elidable_lifetime_names",
    "clippy::format_push_string",
    "clippy::redundant_closure_for_method_calls",
    "clippy::redundant_closure",
    "clippy::stable_sort_primitive",
    "clippy::wildcard_imports",
    "clippy::map_unwrap_or",
    "clippy::clone_on_copy",
    "clippy::single_match_else",
    "clippy::unnecessary_map_or",
    "clippy::if_not_else",
    "clippy::format_collect",
    "clippy::implicit_clone",
    // Casts
    "clippy::cast_possible_truncation",
    "clippy::cast_precision_loss",
    // rustc, in progress
    "unused_imports",
    "unused_variables",
    "dead_code",
];

/// The binary's kept names: `too_many_lines` (permanent) and three of the
/// library's signature-ripple family.
const BINARY_CRATE_LEVEL: [&str; 4] = [
    "clippy::too_many_lines",
    "clippy::ptr_arg",
    "clippy::match_same_arms",
    "clippy::manual_let_else",
];

/// The two crate roots, the only files allowed an inner clippy allow.
const CRATE_ROOTS: [&str; 2] = ["lib.rs", "main.rs"];

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Drop a `//` comment from one line. Lint paths contain `::`, never `//`.
fn strip_line_comment(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

/// The lint names in every `#![allow(` … `)]` of `source`, wherever it sits.
/// A multi-line block is read up to its closing `)]`, one name per comma.
fn inner_allow_lints(source: &str) -> BTreeSet<String> {
    let mut lints = BTreeSet::new();
    let mut rest = source;
    while let Some(start) = rest.find("#![allow(") {
        let after = &rest[start + "#![allow(".len()..];
        let end = after.find(")]").expect("an inner allow closes");
        for line in after[..end].lines() {
            for name in strip_line_comment(line).split(',') {
                let name = name.trim();
                if !name.is_empty() {
                    lints.insert(name.to_string());
                }
            }
        }
        rest = &after[end..];
    }
    lints
}

/// Every `.rs` file under `dir`, recursively, sorted for a stable report.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("src/ is readable") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// `src/`-relative path with `/` separators.
fn relative(path: &Path) -> String {
    let rel = path.strip_prefix(src_root()).expect("under src/");
    rel.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether `line` opens a clippy allow: `Some(true)` for an inner
/// `#![allow(`, `Some(false)` for an outer `#[allow(`, `None` otherwise. The
/// attribute must open the line, so an allow quoted inside a string literal
/// (a test fixture's `let source = "…#[allow(clippy::…)]…"`) is not one.
fn clippy_allow_kind(line: &str) -> Option<bool> {
    let code = line.trim_start();
    let inner = code.starts_with("#![allow(");
    ((inner || code.starts_with("#[allow(")) && line.contains("clippy::")).then_some(inner)
}

/// One clippy allow found in a file: where, which form, and the whole line.
struct ClippyAllow {
    location: String,
    inner: bool,
    line: String,
}

/// Every line under `src/`, except the two crate roots, opening a clippy allow.
fn clippy_allows_outside_roots() -> Vec<ClippyAllow> {
    let mut found = Vec::new();
    for path in rust_files(&src_root()) {
        let rel = relative(&path);
        if CRATE_ROOTS.contains(&rel.as_str()) {
            continue;
        }
        let source = fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in source.lines().enumerate() {
            if let Some(inner) = clippy_allow_kind(line) {
                found.push(ClippyAllow {
                    location: format!("{rel}:{}", index + 1),
                    inner,
                    line: line.to_string(),
                });
            }
        }
    }
    found
}

/// The clippy lints named on one attribute line, comment stripped.
fn clippy_lints_on(line: &str) -> Vec<String> {
    strip_line_comment(line)
        .split(|c: char| c == '(' || c == ')' || c == ',')
        .map(str::trim)
        .filter(|name| name.starts_with("clippy::"))
        .map(str::to_string)
        .collect()
}

/// A clippy allow is well-formed when it closes on its own line and carries a
/// same-line `// Reason:` — so no lint can hide on a continuation line.
fn is_single_line_with_reason(line: &str) -> bool {
    strip_line_comment(line).contains(")]") && line.contains("// Reason:")
}

/// The difference between a root's crate-level list and its expected names,
/// as a failure message, or `None` when they are equal.
fn crate_level_drift(root: &str, expected: &[&str]) -> Option<String> {
    let source = fs::read_to_string(src_root().join(root)).expect("a crate root is readable");
    let actual = inner_allow_lints(&source);
    let expected: BTreeSet<String> = expected.iter().map(ToString::to_string).collect();
    let added: Vec<_> = actual.difference(&expected).collect();
    let dropped: Vec<_> = expected.difference(&actual).collect();
    (!added.is_empty() || !dropped.is_empty()).then(|| {
        format!(
            "src/{root}'s crate-level allow list moved — added {added:?}, dropped {dropped:?}. \
             Fix the sites, or allow per item with a `// Reason:` (ADR 1.10.26j)"
        )
    })
}

/// ADR 1.10.26j AC1 — the library's crate-level list is exactly § 2.2's 32 (from 49).
#[test]
fn the_library_crate_level_allow_list_is_exactly_the_kept_families() {
    if let Some(drift) = crate_level_drift("lib.rs", &LIBRARY_CRATE_LEVEL) {
        panic!("{drift}");
    }
}

/// ADR 1.10.26j AC1 — the binary's crate-level list is exactly four names (from 10).
/// ADR 2.10.26k AC5 — the codegen arm's paydown left it at four.
#[test]
fn the_binary_crate_level_allow_list_is_exactly_four_names() {
    if let Some(drift) = crate_level_drift("main.rs", &BINARY_CRATE_LEVEL) {
        panic!("{drift}");
    }
}

/// ADR 1.10.26j AC1 — no module root confines a clippy lint.
#[test]
fn no_module_root_clippy_allow_exists() {
    let offenders: Vec<String> = clippy_allows_outside_roots()
        .into_iter()
        .filter(|allow| allow.inner)
        .map(|allow| format!("{} {:?}", allow.location, clippy_lints_on(&allow.line)))
        .collect();
    assert!(
        offenders.is_empty(),
        "module-root clippy allows are not part of this package's policy (ADR 1.10.26j): {offenders:?}"
    );
}

/// ADR 1.10.26j AC1 — every per-item clippy allow is one line with a reason.
#[test]
fn every_per_item_clippy_allow_is_one_line_with_a_reason() {
    let offenders: Vec<String> = clippy_allows_outside_roots()
        .into_iter()
        .filter(|allow| !allow.inner && !is_single_line_with_reason(&allow.line))
        .map(|allow| allow.location)
        .collect();
    assert!(
        offenders.is_empty(),
        "per-item clippy allows must close on one line with `// Reason:`: {offenders:?}"
    );
}

/// ADR 1.10.26j AC1 — the parsers, on injected source: a multi-line inner block
/// is read name by name, and a comment is not.
#[test]
fn a_multi_line_inner_block_is_read_and_comments_are_not() {
    let source = "#![allow(\n    // Reason: clippy::hidden is prose\n    clippy::a,\n    \
                  clippy::b\n)]\n#![allow(dead_code)] // Reason: x\n//! docs\n";
    assert_eq!(
        inner_allow_lints(source),
        BTreeSet::from([
            "clippy::a".to_string(),
            "clippy::b".to_string(),
            "dead_code".to_string(),
        ])
    );
}

/// ADR 1.10.26j AC1 — a multi-line or unreasoned outer allow is malformed, and an
/// allow quoted in a string literal is not an allow at all.
#[test]
fn a_multi_line_or_unreasoned_allow_is_malformed() {
    assert!(is_single_line_with_reason(
        "#[allow(clippy::similar_names)] // Reason: a unit pair"
    ));
    assert!(!is_single_line_with_reason(
        "#[allow(clippy::similar_names)]"
    ));
    assert!(!is_single_line_with_reason("#[allow( // Reason: split"));
    assert_eq!(
        clippy_lints_on("#[allow(clippy::a, clippy::b)] // Reason: clippy::c"),
        ["clippy::a", "clippy::b"]
    );
    assert_eq!(
        clippy_allow_kind("    #[allow(clippy::a)] // Reason: x"),
        Some(false)
    );
    assert_eq!(clippy_allow_kind("#![allow(clippy::a)]"), Some(true));
    assert_eq!(clippy_allow_kind("#[allow(dead_code)]"), None);
    assert_eq!(
        clippy_allow_kind(
            r##"    let source = "#[no_mangle]\n#[allow(clippy::missing_safety_doc)]\n\"##
        ),
        None
    );
}
