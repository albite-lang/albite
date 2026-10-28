//! Tests for test companion files (ADR 2.10.26j).
//!
//! The rule, the policy, the combined parse and collisions are asserted over
//! injected file lists, sources and items; the attach step's refusals over a
//! temp directory. Relocation and cache eligibility: `companion_relocate.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::driver::modules::companion::{
    combined_source, companion_collisions, companion_index, companions_in_tree, pairs_in_tree,
    policy_for, CompanionPolicy,
};
use crate::driver::{prepare_project_with, Mode, PipelineError};

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

fn pairs(entries: &[(&str, &str)]) -> BTreeMap<PathBuf, PathBuf> {
    entries
        .iter()
        .map(|(s, c)| (PathBuf::from(s), PathBuf::from(c)))
        .collect()
}

// ---------------------------------------------------------------------------
// AC 1 — the companion rule and the policy
// ---------------------------------------------------------------------------

// ADR 2.10.26j AC1: `a/foo_test.tg` pairs with `a/foo.tg`.
#[test]
fn companion_index_pairs_a_test_file_with_its_neighbour() {
    let index = companion_index(&paths(&["a/foo.tg", "a/foo_test.tg", "a/bar.tg"]));
    assert_eq!(index, pairs(&[("a/foo.tg", "a/foo_test.tg")]));
}

// ADR 2.10.26j AC1: `mod_test.tg` pairs with `mod.tg` by the same rule.
#[test]
fn companion_index_pairs_mod_test_with_mod() {
    let index = companion_index(&paths(&["a/mod.tg", "a/mod_test.tg"]));
    assert_eq!(index, pairs(&[("a/mod.tg", "a/mod_test.tg")]));
}

// ADR 2.10.26j AC1: D1's look-alikes with no neighbour pair with nothing.
#[test]
fn companion_index_skips_a_file_with_no_neighbour() {
    let files = paths(&[
        "src/compiler/parser/self_test.tg",
        "src/compiler/parser/mod.tg",
        "examples/compile_test.tg",
    ]);
    assert!(companion_index(&files).is_empty());
}

// ADR 2.10.26j AC1: the neighbour must sit in the same directory.
#[test]
fn companion_index_skips_a_neighbour_in_another_directory() {
    let files = paths(&["a/foo_test.tg", "b/foo.tg"]);
    assert!(companion_index(&files).is_empty());
}

// ADR 2.10.26j AC1: a stem holding a `.` is not a companion, even beside a
// file that would otherwise be its sibling.
#[test]
fn companion_index_skips_a_dotted_stem() {
    let files = paths(&["x.tungsten.tg", "x.tungsten_test.tg"]);
    assert!(companion_index(&files).is_empty());
    // An empty stem is no companion either.
    assert!(companion_index(&paths(&["_test.tg", ".tg"])).is_empty());
}

// ADR 2.10.26j AC1: a test entry gets no companion.
#[test]
fn companion_index_gives_a_test_entry_no_companion() {
    let files = paths(&[
        "test_foo.tg",
        "test_foo_test.tg",
        "mustfail_x.tg",
        "mustfail_x_test.tg",
    ]);
    assert!(companion_index(&files).is_empty());
}

// ADR 2.10.26j AC1: only `test` attaches.
#[test]
fn policy_attaches_only_under_test() {
    assert_eq!(policy_for(Mode::Test), CompanionPolicy::Attach);
    assert_eq!(policy_for(Mode::Check), CompanionPolicy::Ignore);
    assert_eq!(policy_for(Mode::Run), CompanionPolicy::Ignore);
}

// ---------------------------------------------------------------------------
// AC 3 — the combined parse and the attach step
// ---------------------------------------------------------------------------

/// Every item's span start, in order.
fn item_starts(source: &str) -> Vec<u32> {
    use crate::span::Spanned;
    crate::parse(source)
        .0
        .items
        .iter()
        .map(|i| i.span().start)
        .collect()
}

// ADR 2.10.26j AC3 (P0 item 1): parsing `a ++ "\n" ++ b` yields `a`'s items
// with `a`'s spans followed by `b`'s shifted by `len(a) + 1` — including a
// sibling ending in a dangling doc comment, and a real module.
#[test]
fn combined_parse_keeps_the_sibling_and_shifts_the_companion() {
    let real = include_str!("../../../../../src/compiler/driver/util/json.tg");
    let cases = [
        ("fn a() -> Nat { 1 }\n", "pub fn test_a() -> Unit { () }\n"),
        ("fn a() -> Nat { 1 }\n/// dangling", "fn b() -> Nat { 2 }"),
        (
            "fn a() -> Nat { 1 } // no newline",
            "use x::{y};\nfn b() -> Nat { 2 }",
        ),
        (real, "fn companion_probe() -> Nat { 0 }\n"),
    ];
    for (sibling, companion) in cases {
        let (combined, base) = combined_source(sibling, companion);
        assert_eq!(base as usize, sibling.len() + 1);
        let (_, errors) = crate::parse(&combined);
        assert!(errors.is_empty(), "combined parse failed: {errors:?}");
        let mut expected = item_starts(sibling);
        expected.extend(item_starts(companion).iter().map(|s| s + base));
        assert_eq!(item_starts(&combined), expected);
    }
}

/// A temp project: each `(name, text)` written under one directory.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

fn tree_of(entry: &Path) -> crate::driver::ParsedModule {
    let mut visited = std::collections::HashSet::new();
    crate::driver::parse_module_tree(entry, &mut visited, &mut Vec::new(), None).unwrap()
}

fn canonical_index(dir: &Path) -> BTreeMap<PathBuf, PathBuf> {
    let files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path().canonicalize().unwrap())
        .collect();
    companion_index(&files)
}

// ADR 2.10.26j AC3 (D3): a companion the tree declares as a module stays an
// ordinary module — the `mod` declaration wins.
#[test]
fn a_declared_module_is_not_attached_as_a_companion() {
    let dir = project(&[
        ("main.tg", "pub mod widget;\npub mod widget_test;\n"),
        ("widget.tg", "pub fn w() -> Nat { 1 }\n"),
        ("widget_test.tg", "pub fn t() -> Nat { 2 }\n"),
    ]);
    let tree = tree_of(&dir.path().join("main.tg"));
    let index = canonical_index(dir.path());
    assert_eq!(index.len(), 1, "the files themselves do pair");
    assert!(pairs_in_tree(&tree, &index).is_empty());

    // Control: without the declaration, the pair attaches.
    let dir = project(&[
        ("main.tg", "pub mod widget;\n"),
        ("widget.tg", "pub fn w() -> Nat { 1 }\n"),
        ("widget_test.tg", "pub fn t() -> Nat { 2 }\n"),
    ]);
    let tree = tree_of(&dir.path().join("main.tg"));
    let found = pairs_in_tree(&tree, &canonical_index(dir.path()));
    assert_eq!(found.len(), 1);
    assert!(found[0].companion.ends_with("widget_test.tg"));
    assert!(found[0].sibling.ends_with("widget.tg"));
}

// ADR 2.10.26j AC3 (D3): a companion's `mod` is refused, naming the companion.
#[test]
fn a_companion_declaring_a_module_is_refused() {
    let dir = project(&[
        ("main.tg", "pub fn main() -> Nat { 0 }\n"),
        (
            "main_test.tg",
            "mod extra;\npub fn test_a() -> Unit { () }\n",
        ),
        ("extra.tg", "pub fn e() -> Nat { 0 }\n"),
    ]);
    let entry = dir.path().join("main.tg");
    match prepare_project_with(&entry, false, None, CompanionPolicy::Attach) {
        Err(PipelineError::CompanionDeclaresModule { companion, name }) => {
            assert!(companion.ends_with("main_test.tg"));
            assert_eq!(name, "extra");
        }
        Err(other) => panic!("expected CompanionDeclaresModule, got {other}"),
        Ok(_) => panic!("expected CompanionDeclaresModule, got a prepared project"),
    }
    // Under Ignore the companion is never read.
    assert!(prepare_project_with(&entry, false, None, CompanionPolicy::Ignore).is_ok());
}

// ---------------------------------------------------------------------------
// AC 5 — collisions
// ---------------------------------------------------------------------------

/// The collision names over a combined sibling + companion parse.
fn collision_names(sibling: &str, companion: &str) -> Vec<String> {
    let (combined, base) = combined_source(sibling, companion);
    let items = crate::parse(&combined).0.items;
    companion_collisions(&items, base)
        .into_iter()
        .map(|c| c.name)
        .collect()
}

// ADR 2.10.26j AC5: a function, a type and an extern defined on both sides
// each collide, with spans on the right side of `base`.
#[test]
fn collisions_name_each_shared_top_level_name() {
    let sibling = "fn helper() -> Nat { 1 }\ntype Shape = Dot\n\
                   extern \"C\" fn tg_probe(x: Nat) -> Nat\n";
    let companion = "fn helper() -> Nat { 2 }\ntype Shape = Line\n\
                     extern \"C\" fn tg_probe(x: Nat) -> Nat\n";
    assert_eq!(
        collision_names(sibling, companion),
        ["helper", "Shape", "tg_probe"]
    );
    let (combined, base) = combined_source(sibling, companion);
    let items = crate::parse(&combined).0.items;
    for collision in companion_collisions(&items, base) {
        assert!(collision.companion_span.start >= base);
        assert!(collision.sibling_span.start < base);
        assert_eq!(collision.owners, None);
    }
}

// ADR 2.10.26j AC5: distinct names, and a type sharing a function's name
// (separate namespaces), do not collide.
#[test]
fn no_collision_between_distinct_names() {
    assert!(collision_names("fn a() -> Nat { 1 }\n", "fn b() -> Nat { 2 }\n").is_empty());
    assert!(collision_names("type Thing = One\n", "fn Thing() -> Nat { 2 }\n").is_empty());
}

// ADR 2.10.26j AC5 (P0 item 8): a constructor shared across the pair collides,
// naming both declaring types.
#[test]
fn a_shared_constructor_name_is_a_collision() {
    let (combined, base) = combined_source("type T = A | B\n", "type U = A | C\n");
    let items = crate::parse(&combined).0.items;
    let found = companion_collisions(&items, base);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "A");
    assert_eq!(found[0].owners, Some(("U".to_string(), "T".to_string())));
}

// ADR 2.10.26j AC5: duplicates within the companion alone are the §3 hole,
// not a pair collision.
#[test]
fn a_duplicate_within_the_companion_alone_is_not_a_pair_collision() {
    let companion = "fn twice() -> Nat { 1 }\nfn twice() -> Nat { 2 }\n";
    assert!(collision_names("fn other() -> Nat { 0 }\n", companion).is_empty());
}

// ADR 2.10.26j AC5 (P0 item 8, D5): a sibling's import against a companion's
// definition collides.
#[test]
fn a_sibling_import_against_a_companion_definition_collides() {
    let names = collision_names("use util::{helper};\n", "fn helper() -> Nat { 2 }\n");
    assert_eq!(names, ["helper"]);
    let aliased = collision_names("use util::thing as helper;\n", "fn helper() -> Nat { 2 }\n");
    assert_eq!(aliased, ["helper"]);
}

// ADR 2.10.26j AC5 (P0 item 8, D5): a companion's import against a sibling's
// definition collides; import against import is E0007's, not this check's.
#[test]
fn a_companion_import_against_a_sibling_definition_collides() {
    let names = collision_names("fn helper() -> Nat { 2 }\n", "use util::helper;\n");
    assert_eq!(names, ["helper"]);
    assert!(collision_names("use util::helper;\n", "use other::helper;\n").is_empty());
    assert!(collision_names("fn a() -> Nat { 2 }\n", "use util::*;\n").is_empty());
}

// ADR 2.10.26j AC6 (D3): a companion's parse error is reported in its own
// lines — the standalone parse fails, so nothing is attached.
#[test]
fn a_companion_parse_error_is_reported_in_its_own_lines() {
    use crate::driver::modules::companion::{companion_parse_failures, CompanionPair};
    let dir = project(&[
        ("main.tg", "pub fn main() -> Nat { 0 }\n"),
        ("main_test.tg", "pub fn ok() -> Nat { 1 }\nfn broken( {\n"),
    ]);
    let pair = CompanionPair {
        sibling: dir.path().join("main.tg"),
        companion: dir.path().join("main_test.tg"),
    };
    let failures = companion_parse_failures(&[pair]);
    assert_eq!(failures.len(), 1);
    let (file, text, errors) = &failures[0];
    assert!(file.ends_with("main_test.tg"));
    assert!(
        text.starts_with("pub fn ok()"),
        "the companion's own text, not the combined"
    );
    let line_two_starts = text.find('\n').unwrap() as u32;
    assert!(errors
        .iter()
        .all(|e| e.span.start > line_two_starts && (e.span.start as usize) <= text.len()));

    let entry = dir.path().join("main.tg");
    assert!(matches!(
        prepare_project_with(&entry, false, None, CompanionPolicy::Attach),
        Err(PipelineError::ElabFailed(ref m)) if m == "parse errors"
    ));
}

// ---------------------------------------------------------------------------
// AC 7 — the guard's tree (the cache's half is in `companion_relocate.rs`)
// ---------------------------------------------------------------------------

// ADR 2.10.26j AC7 (D8): only the companions of modules in the tree.
#[test]
fn companions_in_tree_reads_only_the_trees_modules() {
    let index = pairs(&[
        ("a/main.tg", "a/main_test.tg"),
        ("a/far.tg", "a/far_test.tg"),
    ]);
    let found = companions_in_tree(&paths(&["a/main.tg", "a/near.tg"]), &index);
    assert_eq!(found, paths(&["a/main_test.tg"]));
}
