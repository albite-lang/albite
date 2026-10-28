//! The manifest's schema (D2) and lookup (D1): ADR 29.9.26g AC 1–2, over
//! strings and an injected existence probe. Moved with the planner from the
//! bootstrap (ADR 6.10.26j D2), with the `GOOD` fixture and a `parse` helper.

use std::path::{Path, PathBuf};

use super::manifest::{find_manifest, identifier_problem, parse_manifest, Manifest};
use super::{ByteRange, PlanProblem, PlanProblemKind};

/// The reserved words these tests plan with: a stand-in for a lexer's list.
pub(super) const TEST_RESERVED: &[&str] = &["fn", "match", "use"];

pub(super) fn parse(text: &str) -> Result<Manifest, Vec<PlanProblem>> {
    parse_manifest(text, Path::new("/app/albite.toml"), TEST_RESERVED)
}

pub(super) const GOOD: &str = "\
[package]
name = \"app\"
version = \"0.1.0\"

[dependencies]
geometry = { path = \"../geometry\" }
units = { path = \"vendor/units\" }
";

/// The single refusal's reason and the text its span covers.
fn refusal(text: &str) -> (String, String) {
    let errors = parse(text).expect_err("manifest should be refused");
    assert_eq!(errors.len(), 1, "one refusal expected: {errors:?}");
    let error = &errors[0];
    assert_eq!(error.kind.tag(), 120);
    assert_eq!(error.file, Path::new("/app/albite.toml"));
    let PlanProblemKind::ManifestInvalid { reason } = &error.kind else {
        panic!("not ManifestInvalid: {:?}", error.kind);
    };
    assert_eq!(error.message, format!("invalid `albite.toml`: {reason}"));
    let covered = text[error.range.start as usize..error.range.end as usize].to_string();
    (reason.clone(), covered)
}

// 29.9.26g AC1: the accepted schema.
#[test]
fn manifest_accepts_name_version_and_path_dependencies() {
    let manifest = parse(GOOD).expect("valid");
    assert_eq!(manifest.name, "app");
    assert_eq!(manifest.version.as_deref(), Some("0.1.0"));
    let keys: Vec<_> = manifest
        .dependencies
        .iter()
        .map(|d| (d.key.as_str(), d.path.as_str()))
        .collect();
    assert_eq!(
        keys,
        [("geometry", "../geometry"), ("units", "vendor/units")]
    );
    let span = manifest.dependencies[1].key_span;
    assert_eq!(&GOOD[span.start as usize..span.end as usize], "units");
    assert_eq!(manifest.text, GOOD);

    let bare = parse("[package]\nname = \"lib_2\"\n").expect("valid");
    assert_eq!(bare.version, None);
    assert!(bare.dependencies.is_empty());
}

// 29.9.26g AC1: each refusal has its own note and a span on the right key.
#[test]
fn manifest_refusals_each_name_their_cause() {
    let (reason, _) = refusal("[package\nname = 1");
    assert!(reason.starts_with("TOML syntax error"), "{reason}");

    let (reason, covered) = refusal("[package]\nversion = \"1\"\n");
    assert_eq!(
        (reason.as_str(), covered.as_str()),
        ("missing `[package] name`", "[package]")
    );

    let (reason, covered) = refusal("[package]\nname = \"my-project\"\n");
    assert!(reason.contains("is not a module identifier"), "{reason}");
    assert_eq!(covered, "name");

    let (reason, _) = refusal("[package]\nname = \"match\"\n");
    assert!(reason.contains("reserved word"), "{reason}");

    let (reason, covered) = refusal("[package]\nname = \"a\"\nedition = \"x\"\n");
    assert_eq!(reason, "unknown key `edition` in `[package]`");
    assert_eq!(covered, "edition");

    let (reason, covered) = refusal("[package]\nname = \"a\"\n[workspace]\nx = 1\n");
    assert_eq!(reason, "unknown table or key `workspace`");
    assert_eq!(covered, "[workspace]");

    let (reason, covered) = refusal("[package]\nname = \"a\"\n[dependencies]\nstd = \"1.0\"\n");
    assert!(reason.contains("Phase 2"), "{reason}");
    assert_eq!(covered, "std");

    let (reason, covered) =
        refusal("[package]\nname = \"a\"\n[dependencies]\ng = { git = \"https://x\" }\n");
    assert!(
        reason.starts_with("`git`") && reason.contains("Phase 2"),
        "{reason}"
    );
    assert_eq!(covered, "g");

    let (reason, _) =
        refusal("[package]\nname = \"a\"\n[dependencies]\ng = { path = \"x\", version = \"1\" }\n");
    assert!(
        reason.starts_with("`version`") && reason.contains("Phase 2"),
        "{reason}"
    );

    let (reason, _) =
        refusal("[package]\nname = \"a\"\n[dependencies]\ng = { path = \"x\", features = [] }\n");
    assert_eq!(reason, "unknown key `features` in dependency `g`");

    let (reason, _) = refusal("[package]\nname = \"a\"\n[dependencies]\ng = { }\n");
    assert_eq!(reason, "dependency `g` has no `path`");

    let (reason, _) = refusal("[package]\nname = \"a\"\n[dependencies]\ng = { path = 3 }\n");
    assert_eq!(reason, "`path` of dependency `g` must be a string");

    let (reason, _) = refusal("[package]\nname = \"a\"\nversion = 2\n");
    assert_eq!(reason, "`version` must be a string");

    let (reason, covered) = refusal("name = \"a\"\n[package]\nname = \"a\"\n");
    assert_eq!(reason, "unknown table or key `name`");
    assert_eq!(covered, "name");
}

#[test]
fn a_missing_package_table_is_refused() {
    let errors = parse("[dependencies]\n").expect_err("refused");
    let reasons: Vec<_> = errors.iter().map(|e| e.message.clone()).collect();
    assert!(
        reasons
            .iter()
            .any(|m| m.contains("missing `[package]` table")),
        "{reasons:?}"
    );
}

#[test]
fn identifier_rules() {
    assert_eq!(identifier_problem("geometry_2", TEST_RESERVED), None);
    assert_eq!(identifier_problem("_x", TEST_RESERVED), None);
    assert!(identifier_problem("2d", TEST_RESERVED).is_some());
    assert!(identifier_problem("Geometry", TEST_RESERVED).is_some());
    assert!(identifier_problem("", TEST_RESERVED).is_some());
    assert!(identifier_problem("fn", TEST_RESERVED).is_some());
}

// 6.10.26j AC1: the reserved words are the caller's — a keyword is refused
// with the caller's list and accepted with an empty one.
#[test]
fn reserved_words_are_the_callers() {
    let text = "[package]\nname = \"match\"\n\n[dependencies]\nuse = { path = \"x\" }\n";
    let errors = parse(text).expect_err("refused with the list");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().all(|e| e.message.contains("reserved word")));
    let manifest =
        parse_manifest(text, Path::new("/app/albite.toml"), &[]).expect("accepted without");
    assert_eq!(manifest.name, "match");
    assert_eq!(manifest.dependencies[0].key, "use");
    assert_eq!(
        identifier_problem("match", TEST_RESERVED),
        Some("is a reserved word")
    );
    assert_eq!(identifier_problem("match", &[]), None);
}

// 6.10.26j D2: the core renders each kind's message and tag, which both
// compilers print and read.
#[test]
fn every_kind_renders_its_message_and_tag() {
    let cases = [
        (
            PlanProblemKind::ManifestInvalid { reason: "r".into() },
            120,
            "invalid `albite.toml`: r",
        ),
        (
            PlanProblemKind::DependencyNotFound {
                dependency: "d".into(),
                reason: "r".into(),
            },
            121,
            "dependency `d` not found: r",
        ),
        (
            PlanProblemKind::PackageNameConflict {
                dependency: "d".into(),
                reason: "r".into(),
            },
            122,
            "dependency `d` conflicts: r",
        ),
        (
            PlanProblemKind::DependencyCycle {
                chain: vec!["a".into(), "b".into(), "a".into()],
            },
            124,
            "dependency cycle: a → b → a",
        ),
    ];
    for (kind, tag, message) in cases {
        assert_eq!(kind.tag(), tag);
        assert_eq!(kind.message(), message);
        let problem = PlanProblem::new(kind, Path::new("/f"), ByteRange::new(1, 2));
        assert_eq!(problem.message, message);
        assert_eq!(problem.range, ByteRange::new(1, 2));
        assert_eq!(problem.file, Path::new("/f"));
    }
}

// 29.9.26g AC2: the manifest is found per D1.
#[test]
fn manifest_lookup_follows_d1() {
    let probe =
        |files: &'static [&'static str]| move |p: &Path| files.iter().any(|f| Path::new(f) == p);
    // The entry's own directory wins over an ancestor.
    let found = find_manifest(
        Path::new("/app/packages/geometry/lib.tg"),
        probe(&[
            "/app/packages/geometry/albite.toml",
            "/app/albite.toml",
            "/app/main.tg",
        ]),
    );
    assert_eq!(
        found,
        Some(PathBuf::from("/app/packages/geometry/albite.toml"))
    );
    // An ancestor is found from a nested module.
    let found = find_manifest(
        Path::new("/app/sub/x.tg"),
        probe(&["/app/albite.toml", "/app/main.tg"]),
    );
    assert_eq!(found, Some(PathBuf::from("/app/albite.toml")));
    // A `main.tg` without a manifest stops the walk: the one above is never read.
    let found = find_manifest(
        Path::new("/outer/app/main.tg"),
        probe(&["/outer/app/main.tg", "/outer/albite.toml"]),
    );
    assert_eq!(found, None);
    // Nothing anywhere.
    assert_eq!(find_manifest(Path::new("/a/b.tg"), probe(&[])), None);
}

// 29.9.26g AC1: spans land on the key as written, indentation included.
#[test]
fn indented_keys_and_headers_are_spanned_exactly() {
    let text = "  [package]\n  name = \"my-project\"\n";
    let (reason, covered) = refusal(text);
    assert!(reason.contains("module identifier"), "{reason}");
    assert_eq!(covered, "name");

    let text = "[package]\nname = \"a\"\n\n   [dependencies]  \n  g = { path = \"x\" }\n";
    let manifest = parse(text).expect("valid");
    let key = manifest.dependencies[0].key_span;
    assert_eq!(&text[key.start as usize..key.end as usize], "g");
}

// 29.9.26g AC1: a syntax error whose own span is empty still gets a
// one-byte span at the same place, so the renderer has something to point at.
#[test]
fn an_empty_syntax_error_span_is_widened_to_one_byte() {
    let text = "[package]\nname = \"a\"\nx = ";
    let toml_span = toml::from_str::<toml::Table>(text)
        .expect_err("invalid")
        .span()
        .expect("span");
    let errors = parse(text).expect_err("refused");
    let span = errors[0].range;
    assert_eq!(span.start as usize, toml_span.start);
    assert_eq!(span.end as usize, toml_span.end.max(toml_span.start + 1));
    assert!(
        span.start > 0 && span.end == span.start + 1,
        "{span:?} from {toml_span:?}"
    );
}
