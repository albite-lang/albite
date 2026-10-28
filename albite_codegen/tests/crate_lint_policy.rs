//! The codegen crate's lint policy, pinned (ADR 2.10.26k § 2.4).
//!
//! Three tiers, one assertion each: `src/lib.rs`'s `#![allow(…)]` names
//! exactly the two kept lints; no module root confines a policed lint; and
//! every per-item allow of one sits on one line with a `// Reason:`. A new
//! blanket allow then fails `cargo test -p albite_codegen` naming the lint,
//! instead of silently widening what the linter ignores.
//!
//! The parser helpers are a fourth copy of
//! `bootstrap/tests/crate_lint_policy.rs`, with its root parameter dropped
//! (ADR 2.10.26k D8). This copy diverges: the per-item tier also reads the
//! rustc names `dead_code` and `unused_imports`, so a future shared helper must
//! take the policed names as a parameter. G8's closing slice extracts it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The kept names: `too_many_lines` (permanent, code-health's) and
/// `cast_possible_truncation` (LLVM's u32 index boundary). Non-clippy names
/// count too: `unknown_lints` and `dead_code` were deleted and must stay gone.
const CRATE_LEVEL: [&str; 2] = ["clippy::too_many_lines", "clippy::cast_possible_truncation"];

/// The crate root, the only file allowed an inner allow.
const CRATE_ROOT: &str = "lib.rs";

/// The lint names an allow attribute is policed for: every clippy lint, plus
/// the two rustc lints this crate allows per item.
const POLICED_PREFIXES: [&str; 3] = ["clippy::", "dead_code", "unused_imports"];

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

/// The policed lints named on one attribute line, comment stripped.
fn policed_lints_on(line: &str) -> Vec<String> {
    strip_line_comment(line)
        .split(|c: char| c == '(' || c == ')' || c == ',')
        .map(str::trim)
        .filter(|name| {
            POLICED_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .map(str::to_string)
        .collect()
}

/// Whether `line` opens an allow of a policed lint: `Some(true)` for an inner
/// `#![allow(`, `Some(false)` for an outer `#[allow(`, `None` otherwise. The
/// attribute must open the line, so an allow quoted inside a string literal is
/// not one.
fn policed_allow_kind(line: &str) -> Option<bool> {
    let code = line.trim_start();
    let inner = code.starts_with("#![allow(");
    let opens = inner || code.starts_with("#[allow(");
    let polices = POLICED_PREFIXES.iter().any(|prefix| line.contains(prefix));
    (opens && polices).then_some(inner)
}

/// One policed allow found in a file: where, which form, and the whole line.
struct PolicedAllow {
    location: String,
    inner: bool,
    line: String,
}

/// Every line under `src/`, except the crate root, opening a policed allow.
fn policed_allows_outside_root() -> Vec<PolicedAllow> {
    let mut found = Vec::new();
    for path in rust_files(&src_root()) {
        let rel = relative(&path);
        if rel == CRATE_ROOT {
            continue;
        }
        let source = fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in source.lines().enumerate() {
            if let Some(inner) = policed_allow_kind(line) {
                found.push(PolicedAllow {
                    location: format!("{rel}:{}", index + 1),
                    inner,
                    line: line.to_string(),
                });
            }
        }
    }
    found
}

/// An allow is well-formed when it closes on its own line and carries a
/// same-line `// Reason:` — so no lint can hide on a continuation line.
fn is_single_line_with_reason(line: &str) -> bool {
    strip_line_comment(line).contains(")]") && line.contains("// Reason:")
}

/// ADR 2.10.26k AC1 — the crate-level list is exactly two names (from 21).
#[test]
fn the_crate_level_allow_list_is_exactly_two_names() {
    let source = fs::read_to_string(src_root().join(CRATE_ROOT)).expect("lib.rs is readable");
    let actual = inner_allow_lints(&source);
    let expected: BTreeSet<String> = CRATE_LEVEL.iter().map(ToString::to_string).collect();
    let added: Vec<_> = actual.difference(&expected).collect();
    let dropped: Vec<_> = expected.difference(&actual).collect();
    assert!(
        added.is_empty() && dropped.is_empty(),
        "src/lib.rs's crate-level allow list moved — added {added:?}, dropped {dropped:?}. \
         Fix the sites, or allow per item with a `// Reason:` (ADR 2.10.26k)"
    );
}

/// ADR 2.10.26k AC1 — no module root confines a policed lint.
#[test]
fn no_module_root_clippy_allow_exists() {
    let offenders: Vec<String> = policed_allows_outside_root()
        .into_iter()
        .filter(|allow| allow.inner)
        .map(|allow| format!("{} {:?}", allow.location, policed_lints_on(&allow.line)))
        .collect();
    assert!(
        offenders.is_empty(),
        "module-root allows are not part of this crate's policy (ADR 2.10.26k): {offenders:?}"
    );
}

/// ADR 2.10.26k AC1 — every per-item allow of a clippy lint, `dead_code` or
/// `unused_imports` is one line with a reason.
#[test]
fn every_per_item_allow_is_one_line_with_a_reason() {
    let offenders: Vec<String> = policed_allows_outside_root()
        .into_iter()
        .filter(|allow| !allow.inner && !is_single_line_with_reason(&allow.line))
        .map(|allow| allow.location)
        .collect();
    assert!(
        offenders.is_empty(),
        "per-item allows must close on one line with `// Reason:`: {offenders:?}"
    );
}

/// ADR 2.10.26k AC1 — the parsers, on injected source: a multi-line inner block
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

/// ADR 2.10.26k AC1 — a multi-line or unreasoned allow is malformed (a rustc
/// `dead_code` one included), and an allow quoted in a string literal is not
/// an allow at all.
#[test]
fn a_multi_line_or_unreasoned_allow_is_malformed() {
    assert!(is_single_line_with_reason(
        "#[allow(clippy::similar_names)] // Reason: a unit pair"
    ));
    assert!(!is_single_line_with_reason(
        "#[allow(clippy::similar_names)]"
    ));
    assert!(!is_single_line_with_reason("#[allow( // Reason: split"));
    let unreasoned_dead_code = "    #[allow(dead_code)]";
    assert_eq!(policed_allow_kind(unreasoned_dead_code), Some(false));
    assert!(!is_single_line_with_reason(unreasoned_dead_code));
    assert_eq!(
        policed_lints_on("#[allow(dead_code, clippy::a)] // Reason: clippy::c"),
        ["dead_code", "clippy::a"]
    );
    assert_eq!(
        policed_allow_kind("    #[allow(unused_imports)] // Reason: x"),
        Some(false)
    );
    assert_eq!(policed_allow_kind("#![allow(clippy::a)]"), Some(true));
    assert_eq!(policed_allow_kind("#[allow(unused_mut)]"), None);
    assert_eq!(
        policed_allow_kind(r##"    let source = "#[no_mangle]\n#[allow(clippy::a)]\n\"##),
        None
    );
}
