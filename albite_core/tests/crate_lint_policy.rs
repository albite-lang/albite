//! The crate's clippy policy, pinned (ADR 28.9.26h § 2.3).
//!
//! Three tiers, one assertion each: the crate-level `#![allow(…)]` in
//! `src/lib.rs` names exactly two lints; exactly three module roots confine one
//! lint each; and every other clippy allow sits on one line with a
//! `// Reason:`. A new blanket allow then fails `make test` naming the lint,
//! instead of silently widening what the linter ignores.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The only lints the whole crate may ignore, each with a reason in `lib.rs`.
const CRATE_LEVEL: [&str; 2] = ["clippy::cast_possible_truncation", "clippy::too_many_lines"];

/// `(path under src/, lint)` — each lint's every site sits under that root.
const MODULE_ROOT_CONFINEMENTS: [(&str, &str); 3] = [
    ("ffi/driver/mod.rs", "clippy::not_unsafe_ptr_arg_deref"),
    ("ffi/mod.rs", "clippy::manual_let_else"),
    ("typecheck/mod.rs", "clippy::result_large_err"),
];

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Drop a `//` comment from one line. Lint paths contain `::`, never `//`.
fn strip_line_comment(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

/// The lint names in every `#![allow(` … `)]` of `source`, wherever it sits —
/// `lib.rs` puts its `//!` docs after the attributes, so position is no guide.
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

/// `src/`-relative path with `/` separators, as the tables above spell it.
fn relative(path: &Path) -> String {
    let rel = path.strip_prefix(src_root()).expect("under src/");
    rel.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// One clippy allow found in a file: where, which form, and the whole line.
struct ClippyAllow {
    location: String,
    inner: bool,
    line: String,
}

/// Every line under `src/` (except `lib.rs`) opening a clippy allow.
fn clippy_allows_outside_lib() -> Vec<ClippyAllow> {
    let mut found = Vec::new();
    for path in rust_files(&src_root()) {
        let rel = relative(&path);
        if rel == "lib.rs" {
            continue;
        }
        let source = fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in source.lines().enumerate() {
            let code = line.trim_start();
            let inner = code.starts_with("#![allow(");
            if (inner || code.starts_with("#[allow(")) && line.contains("clippy::") {
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

/// ADR 28.9.26h AC1 — the crate-level list is exactly two names (from 23).
#[test]
fn the_crate_level_allow_list_is_exactly_two_lints() {
    let lib = fs::read_to_string(src_root().join("lib.rs")).expect("lib.rs is readable");
    let actual = inner_allow_lints(&lib);
    let expected: BTreeSet<String> = CRATE_LEVEL.iter().map(ToString::to_string).collect();
    let added: Vec<_> = actual.difference(&expected).collect();
    let dropped: Vec<_> = expected.difference(&actual).collect();
    assert!(
        added.is_empty() && dropped.is_empty(),
        "albite_core's crate-level allow list moved — added {added:?}, dropped {dropped:?}. \
         Fix the sites, or allow per item with a `// Reason:` (ADR 28.9.26h)"
    );
}

/// ADR 28.9.26h AC1 — exactly three module-root confinements, one line each.
#[test]
fn the_module_root_allows_are_exactly_the_three_confinements() {
    let mut actual = BTreeSet::new();
    for allow in clippy_allows_outside_lib().into_iter().filter(|a| a.inner) {
        assert!(
            is_single_line_with_reason(&allow.line),
            "{}: a module-root clippy allow must be one line with `// Reason:`",
            allow.location
        );
        let file = allow.location.rsplit_once(':').map_or("", |(file, _)| file);
        for lint in clippy_lints_on(&allow.line) {
            actual.insert((file.to_string(), lint));
        }
    }
    let expected: BTreeSet<(String, String)> = MODULE_ROOT_CONFINEMENTS
        .iter()
        .map(|(file, lint)| ((*file).to_string(), (*lint).to_string()))
        .collect();
    assert_eq!(
        actual, expected,
        "module-root clippy allows moved (ADR 28.9.26h § 2.3)"
    );
}

/// ADR 28.9.26h AC1 — every per-item clippy allow is one line with a reason.
#[test]
fn every_per_item_clippy_allow_is_one_line_with_a_reason() {
    let offenders: Vec<String> = clippy_allows_outside_lib()
        .into_iter()
        .filter(|allow| !allow.inner && !is_single_line_with_reason(&allow.line))
        .map(|allow| allow.location)
        .collect();
    assert!(
        offenders.is_empty(),
        "per-item clippy allows must close on one line with `// Reason:`: {offenders:?}"
    );
}

/// The parsers, on injected source — the half a must-fail check leans on.
#[test]
fn a_third_crate_level_name_is_read_and_comments_are_not() {
    let source = "#![allow(\n    clippy::too_many_lines, // Reason: a, b\n    clippy::cast_possible_truncation,\n    clippy::doc_markdown, // Reason: x\n)]\n//! docs\n";
    let lints = inner_allow_lints(source);
    assert_eq!(lints.len(), 3, "{lints:?}");
    assert!(lints.contains("clippy::doc_markdown"));
    assert!(!lints.iter().any(|lint| lint.contains("Reason")));
}

#[test]
fn a_multi_line_or_unreasoned_allow_is_malformed() {
    assert!(is_single_line_with_reason(
        "#[allow(clippy::borrowed_box)] // Reason: callers pass &Box"
    ));
    assert!(!is_single_line_with_reason(
        "#[allow(clippy::borrowed_box)]"
    ));
    assert!(!is_single_line_with_reason("#[allow( // Reason: split"));
    assert_eq!(
        clippy_lints_on("#[allow(clippy::a, clippy::b)] // Reason: clippy::c"),
        ["clippy::a", "clippy::b"]
    );
}
