//! The runtime crate's lint policy, pinned (ADR 30.9.26l § 2.4).
//!
//! Three tiers, one assertion each: the crate-level `#![allow(…)]` in
//! `src/lib.rs` names exactly `unsafe_code`; no module root confines a clippy
//! lint; and every per-item clippy allow sits on one line with a
//! `// Reason:`. A new blanket allow then fails `make test` naming the lint,
//! instead of silently widening what the linter ignores.
//!
//! The parser helpers are a deliberate copy of
//! `albite_core/tests/crate_lint_policy.rs` (ADR 30.9.26l D10): no shared
//! test-support crate exists, and a `#[path]` reach into another crate's
//! `tests/` would couple two published crates' test trees. The third consumer
//! (bootstrap) is when a shared helper pays for itself.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The only lint the whole crate may ignore, with its reason in `lib.rs`.
/// Non-clippy names count too: `unknown_lints` and
/// `function_casts_as_integer` were deleted by this ADR and must stay gone.
const CRATE_LEVEL: [&str; 1] = ["unsafe_code"];

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Drop a `//` comment from one line. Lint paths contain `::`, never `//`.
fn strip_line_comment(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

/// The lint names in every `#![allow(` … `)]` of `source`, wherever it sits.
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

/// ADR 30.9.26l AC1 — the crate-level list is exactly `unsafe_code` (from 11).
#[test]
fn the_crate_level_allow_list_is_exactly_unsafe_code() {
    let lib = fs::read_to_string(src_root().join("lib.rs")).expect("lib.rs is readable");
    let actual = inner_allow_lints(&lib);
    let expected: BTreeSet<String> = CRATE_LEVEL.iter().map(ToString::to_string).collect();
    let added: Vec<_> = actual.difference(&expected).collect();
    let dropped: Vec<_> = expected.difference(&actual).collect();
    assert!(
        added.is_empty() && dropped.is_empty(),
        "albite_runtime's crate-level allow list moved — added {added:?}, dropped {dropped:?}. \
         Fix the sites, or allow per item with a `// Reason:` (ADR 30.9.26l)"
    );
}

/// ADR 30.9.26l AC1 — no module root confines a clippy lint.
#[test]
fn no_module_root_clippy_allow_exists() {
    let offenders: Vec<String> = clippy_allows_outside_lib()
        .into_iter()
        .filter(|allow| allow.inner)
        .map(|allow| format!("{} {:?}", allow.location, clippy_lints_on(&allow.line)))
        .collect();
    assert!(
        offenders.is_empty(),
        "module-root clippy allows are not part of this crate's policy (ADR 30.9.26l): {offenders:?}"
    );
}

/// ADR 30.9.26l AC1 — every per-item clippy allow is one line with a reason.
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
fn a_second_crate_level_name_is_read_and_comments_are_not() {
    let source =
        "#![allow(unsafe_code)] // Reason: a, b\n#![allow(unknown_lints)] // Reason: x\n//! docs\n";
    let lints = inner_allow_lints(source);
    assert_eq!(
        lints,
        BTreeSet::from(["unknown_lints".to_string(), "unsafe_code".to_string()])
    );
}

#[test]
fn a_multi_line_or_unreasoned_allow_is_malformed() {
    assert!(is_single_line_with_reason(
        "#[allow(clippy::borrow_as_ptr)] // Reason: fault path only"
    ));
    assert!(!is_single_line_with_reason(
        "#[allow(clippy::borrow_as_ptr)]"
    ));
    assert!(!is_single_line_with_reason("#[allow( // Reason: split"));
    assert_eq!(
        clippy_lints_on("#[allow(clippy::a, clippy::b)] // Reason: clippy::c"),
        ["clippy::a", "clippy::b"]
    );
}
