//! ADR 2.10.26d AC 2 — the register loads, refuses what D4 refuses, and only
//! shrinks.

use std::collections::BTreeSet;
use std::path::Path;

use crate::elaborate::kernel::{register_key, repository_root, Register, RegisterClass};

/// The register's row count. **Shrink-only**: removing a row lowers this in
/// the same change; raising it is a reviewed edit that must cite the ADR
/// adding the row, which names itself in that row's `removed_by` (D4).
/// 3 — ADR 2.10.26d: `two_is_two`, `length_two`, `via_two` (needs-delta).
/// 0 — ADR 2.10.26l: all three removed; the kernel unfolds what it admits.
const KERNEL_EXEMPTION_CEILING: usize = 0;

fn row(file: &str, theorem: &str, cause: &str, extra: &str) -> String {
    format!(
        "[[exemption]]\nfile = \"{file}\"\ntheorem = \"{theorem}\"\ncause = \"{cause}\"\n{extra}\n"
    )
}

fn register(rows: &[String]) -> Result<Register, String> {
    Register::parse(&format!("schema = 1\n{}", rows.concat()))
}

fn delta(theorem: &str) -> String {
    row("a.tg", theorem, "needs-delta", "removed_by = \"2.10.26l\"")
}

// 2.10.26d AC2; 2.10.26l AC 6: the real register loads, and is empty — no
// `needs-delta` or `self-reference` row, and no row owned by 2.10.26l.
#[test]
fn the_real_register_loads() {
    let real = Register::real();
    assert!(real.rows().is_empty(), "{:?}", real.rows());
    assert!(real.rows().iter().all(|row| row.class.admitted_today()));
    assert_eq!(Register::active(), real, "no drill has injected one here");
}

// 2.10.26d AC2: classes not admitted today, and an unknown spelling.
#[test]
fn a_cause_not_admitted_today_is_refused() {
    for cause in ["named-type", "self-reference"] {
        let error = register(&[row("a.tg", "t", cause, "removed_by = \"x\"")]).unwrap_err();
        assert!(error.contains("not admitted today"), "{cause}: {error}");
    }
    let error = register(&[row("a.tg", "t", "needs_delta", "removed_by = \"x\"")]).unwrap_err();
    assert!(error.contains("not a register class"), "{error}");
    assert!(RegisterClass::NeedsDelta.admitted_today());
    assert!(RegisterClass::CitesExempt.admitted_today());
}

// 2.10.26d AC2
#[test]
fn a_duplicate_key_is_refused() {
    let error = register(&[delta("t"), delta("t")]).unwrap_err();
    assert!(error.contains("listed twice"), "{error}");
    let other_file = row("b.tg", "t", "needs-delta", "removed_by = \"x\"");
    assert!(
        register(&[delta("t"), other_file]).is_ok(),
        "the key is (file, theorem)"
    );
}

// 2.10.26d AC2
#[test]
fn a_missing_removed_by_is_refused() {
    let error = register(&[row("a.tg", "t", "needs-delta", "")]).unwrap_err();
    assert!(error.contains("removed_by"), "{error}");
}

// 2.10.26d AC2: `cites` only, and always, on a cites-exempt row.
#[test]
fn cites_on_the_wrong_row_is_refused() {
    let on_delta = row(
        "a.tg",
        "t",
        "needs-delta",
        "removed_by = \"x\"\ncites = [\"u\"]",
    );
    let error = register(&[on_delta, delta("u")]).unwrap_err();
    assert!(error.contains("only a cites-exempt row"), "{error}");
    let without = row("a.tg", "t", "cites-exempt", "removed_by = \"x\"");
    let error = register(&[without]).unwrap_err();
    assert!(error.contains("needs `cites`"), "{error}");
    let empty = row(
        "a.tg",
        "t",
        "cites-exempt",
        "removed_by = \"x\"\ncites = []",
    );
    assert!(register(&[empty]).is_err(), "an empty list cites nothing");
}

// 2.10.26d AC2
#[test]
fn a_cited_name_that_is_not_a_row_is_refused() {
    let cites = row(
        "a.tg",
        "t",
        "cites-exempt",
        "removed_by = \"x\"\ncites = [\"ghost\"]",
    );
    let error = register(&[cites]).unwrap_err();
    assert!(error.contains("`ghost`, which is not a row"), "{error}");
}

// 2.10.26d AC2: a cites-exempt row's theorem is exempt too.
#[test]
fn the_exempt_set_is_transitive() {
    let cites = row(
        "a.tg",
        "leans",
        "cites-exempt",
        "removed_by = \"x\"\ncites = [\"base\"]",
    );
    let loaded = register(&[
        delta("base"),
        cites,
        row("b.tg", "far", "needs-delta", "removed_by = \"x\""),
    ])
    .expect("a well-formed register");
    let built = BTreeSet::from(["a.tg".to_string()]);
    let exempt = loaded.exempt_names(&built);
    assert_eq!(
        exempt,
        BTreeSet::from(["base".to_string(), "leans".to_string()])
    );
    assert_eq!(
        loaded.row("a.tg", "leans").map(|r| r.class),
        Some(RegisterClass::CitesExempt)
    );
    assert!(loaded.row("b.tg", "base").is_none(), "keyed by file too");
}

// 2.10.26d AC2: the key is the path under the root; outside it, none.
#[test]
fn a_file_outside_the_repository_matches_no_row() {
    let root = Path::new("/repo");
    assert_eq!(
        register_key(Path::new("/repo/tests/golden/check/x.tg"), root).as_deref(),
        Some("tests/golden/check/x.tg")
    );
    assert_eq!(
        register_key(Path::new("/elsewhere/tests/golden/check/x.tg"), root),
        None
    );
    assert_eq!(register_key(root, root), None, "the root itself is no file");
    assert!(repository_root().join("bootstrap/Cargo.toml").is_file());
}

/// The schema is pinned, and unknown keys are refused.
#[test]
fn a_wrong_schema_or_an_unknown_key_is_refused() {
    assert!(Register::parse("schema = 2\n")
        .unwrap_err()
        .contains("schema 2"));
    let typo = row(
        "a.tg",
        "t",
        "needs-delta",
        "removed_by = \"x\"\nremove_by = \"y\"",
    );
    assert!(register(&[typo]).is_err());
    assert_eq!(Register::parse("schema = 1\n"), Ok(Register::EMPTY));
}

// 2.10.26d AC2
#[test]
fn the_register_only_shrinks() {
    assert_eq!(
        Register::real().rows().len(),
        KERNEL_EXEMPTION_CEILING,
        "remove a row → lower the ceiling; add one → an ADR raises it (D4)"
    );
}

// 2.10.26d AC2: parse only (cost 2) — each row's file declares its theorem.
#[test]
fn every_exemption_names_a_declared_theorem() {
    for exemption in Register::real().rows() {
        let path = repository_root().join(&exemption.file);
        let source =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", exemption.file));
        let (ast, errors) = crate::parse(&source);
        assert!(errors.is_empty(), "{} parses", exemption.file);
        let declared = ast.items.iter().any(|item| match item {
            crate::ast::Item::Theorem(t) | crate::ast::Item::Lemma(t) => {
                t.name.name == exemption.theorem
            }
            _ => false,
        });
        assert!(
            declared,
            "{} declares `{}`",
            exemption.file, exemption.theorem
        );
    }
}

/// Injection is scoped: restored after the body, so no other test sees it.
#[test]
fn an_injected_register_is_restored_after_its_body() {
    let seen = Register::inject(Register::EMPTY, || Register::active().rows().len());
    assert_eq!(seen, 0);
    assert_eq!(Register::active(), Register::real());
}
