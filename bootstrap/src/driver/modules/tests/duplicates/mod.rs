//! Tests for within-file duplicate definitions (ADR 13.10.26f).
//!
//! The walk is asserted over parsed items; the driver's refusal over a temp
//! directory, through `prepare_project`, which every command builds through.

use std::fs;

use crate::driver::modules::companion::{combined_source, companion_collisions};
use crate::driver::modules::definitions::{
    duplicate_definitions, duplicate_errors, within_file_error, Duplicate,
};
use crate::driver::{prepare_project, PipelineError};
use crate::elaborate::ElabErrorKind;

/// The duplicates over one file's parse.
fn duplicates(source: &str) -> Vec<Duplicate> {
    duplicate_definitions(&crate::parse(source).0.items)
}

/// The duplicate names over one file's parse.
fn names(source: &str) -> Vec<String> {
    duplicates(source).into_iter().map(|d| d.name).collect()
}

/// The 1-based line a span starts on.
fn line_of(source: &str, start: u32) -> usize {
    source[..start as usize].matches('\n').count() + 1
}

// 13.10.26f AC2: two `fn`s of one name give one collision at the later site,
// with the first site recorded.
#[test]
fn two_fns_of_one_name_collide_at_the_later() {
    let source = "fn helper() -> Nat { 1 }\nfn helper() -> Nat { 2 }\n";
    let found = duplicates(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "helper");
    assert_eq!(line_of(source, found[0].first.start), 1);
    assert_eq!(line_of(source, found[0].second.start), 2);
    assert_eq!(found[0].owners, None);
}

// 13.10.26f AC2: two types of one name collide; a type alias counts as a type.
#[test]
fn two_types_of_one_name_collide() {
    assert_eq!(names("type T = A | B\ntype T = C(Nat)\n"), ["T"]);
    assert_eq!(names("type T = A | B\ntype T = Nat\n"), ["T"]);
}

// 13.10.26f AC2: a `fn` named like a constructor collides at the `fn`.
#[test]
fn a_fn_named_like_a_constructor_collides() {
    let source = "type T = A | B\nfn A() -> Nat { 3 }\n";
    let found = duplicates(source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "A");
    assert_eq!(line_of(source, found[0].second.start), 2);
    assert_eq!(found[0].owners, None);
}

// 13.10.26f AC2: a constructor declared by two types collides, naming both
// types — the later site's first.
#[test]
fn a_constructor_in_two_types_collides_naming_both() {
    let found = duplicates("type T = A | B\ntype U = A | C\n");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "A");
    assert_eq!(found[0].owners, Some(("U".to_string(), "T".to_string())));
}

// 13.10.26f AC2: a `use` and a definition of one name collide in either order.
#[test]
fn a_use_and_a_definition_collide() {
    assert_eq!(
        names("use util::{helper};\nfn helper() -> Nat { 2 }\n"),
        ["helper"]
    );
    assert_eq!(
        names("fn helper() -> Nat { 2 }\nuse util::helper;\n"),
        ["helper"]
    );
    assert_eq!(
        names("use util::thing as helper;\ntype helper = Nat\n"),
        ["helper"]
    );
}

// 13.10.26f AC2: an `axiom`, a `theorem`, a `lemma` and an `extern fn` are
// values like a `fn`.
#[test]
fn every_value_kind_collides_with_a_fn() {
    assert_eq!(names("axiom ax(): Nat\nfn ax() -> Nat { 1 }\n"), ["ax"]);
    assert_eq!(
        names("theorem t(): 1 == 1 { refl }\nlemma t(): 2 == 2 { refl }\n"),
        ["t"]
    );
    let extern_twice =
        "extern \"C\" fn tg_probe(x: Nat) -> Nat\nfn tg_probe(x: Nat) -> Nat { x }\n";
    assert_eq!(names(extern_twice), ["tg_probe"]);
}

// 13.10.26f AC2: two `use`s of one name are E0007's, not this walk's; a glob
// names nothing; a type and a fn of one name live in different namespaces.
#[test]
fn non_collisions_stay_silent() {
    assert!(names("use util::helper;\nuse other::helper;\n").is_empty());
    assert!(names("use util::*;\nfn helper() -> Nat { 1 }\n").is_empty());
    assert!(names("type Thing = One\nfn Thing() -> Nat { 2 }\n").is_empty());
    assert!(names("fn a() -> Nat { 1 }\nfn b() -> Nat { 2 }\n").is_empty());
}

// 13.10.26f D1: a third definition reports against the earliest, so each
// later site is reported once.
#[test]
fn a_third_definition_reports_against_the_first() {
    let source = "fn f() -> Nat { 1 }\nfn f() -> Nat { 2 }\nfn f() -> Nat { 3 }\n";
    let found = duplicates(source);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|d| line_of(source, d.first.start) == 1));
    assert_eq!(line_of(source, found[1].second.start), 3);
}

// 13.10.26f D1: a pair straddles a split only with the first site before it
// and the second at or after it.
#[test]
fn straddles_reads_both_sites_against_the_base() {
    let (combined, base) = combined_source("fn a() -> Nat { 1 }\n", "fn a() -> Nat { 2 }\n");
    let found = duplicate_definitions(&crate::parse(&combined).0.items);
    assert_eq!(found.len(), 1);
    assert!(found[0].straddles(Some(base)));
    assert!(!found[0].straddles(None));
    assert!(!found[0].straddles(Some(found[0].second.start + 1)));
    assert!(!found[0].straddles(Some(found[0].first.start)));
    assert!(found[0].straddles(Some(found[0].second.start)));
}

// 13.10.26f AC2: the straddling case is unchanged — a duplicate inside the
// companion is within-file, and only the cross-side pair is a collision.
#[test]
fn companion_collisions_are_the_straddling_duplicates() {
    let (combined, base) = combined_source(
        "fn a() -> Nat { 1 }\n",
        "fn a() -> Nat { 2 }\nfn b() -> Nat { 3 }\nfn b() -> Nat { 4 }\n",
    );
    let items = crate::parse(&combined).0.items;
    assert_eq!(duplicate_definitions(&items).len(), 2);
    let collisions = companion_collisions(&items, base);
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0].name, "a");
}

// 13.10.26f D2: the within-file error is E0004 at the later site, with a note
// at the first that names a constructor's type.
#[test]
fn the_within_file_error_points_at_both_sites() {
    let found = duplicates("type T = A | B\ntype U = A | C\n");
    let error = within_file_error(&found[0], std::path::Path::new("m.tg"));
    assert!(matches!(&error.kind, ElabErrorKind::DuplicateDefinition(n) if n == "A"));
    assert_eq!(error.span, found[0].second);
    assert_eq!(error.notes.len(), 1);
    assert_eq!(error.notes[0].span, Some(found[0].first));
    assert!(error.notes[0].message.contains("constructor of `T`"));
    assert!(error
        .help
        .as_deref()
        .is_some_and(|h| h.contains("once per file")));
}

/// The driver's duplicate errors for a temp project whose `main.tg` has the
/// companion `main_test.tg` attached, as `albite test` attaches it.
fn attached_duplicate_errors(sibling: &str, companion: &str) -> Vec<crate::elaborate::ElabError> {
    use crate::driver::modules::companion::{attach_companions, companion_index, pairs_in_tree};
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("main.tg"), sibling).unwrap();
    fs::write(dir.path().join("main_test.tg"), companion).unwrap();
    let mut tree = crate::driver::parse_module_tree(
        &dir.path().join("main.tg"),
        &mut std::collections::HashSet::new(),
        &mut Vec::new(),
        None,
    )
    .unwrap();
    let files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path().canonicalize().unwrap())
        .collect();
    let pairs = pairs_in_tree(&tree, &companion_index(&files));
    let mut source_map = crate::driver::modules::build_source_map(&tree);
    attach_companions(&mut tree, &pairs, &mut source_map).unwrap();
    duplicate_errors(&tree, &source_map)
}

// 13.10.26f D2: one walk renders a straddling pair in 2.10.26j's companion
// form (a cross-file note and the companion help) and a pair inside the
// companion in the within-file form.
#[test]
fn duplicate_errors_render_straddling_and_within_file_pairs_apart() {
    let errors = attached_duplicate_errors(
        "fn a() -> Nat { 1 }\n",
        "fn a() -> Nat { 2 }\nfn b() -> Nat { 3 }\nfn b() -> Nat { 4 }\n",
    );
    assert_eq!(errors.len(), 2);
    assert!(errors[0]
        .help
        .as_deref()
        .is_some_and(|h| h.contains("test companion")));
    assert!(errors[0].notes[0].file_path.is_some());
    assert!(errors[1]
        .help
        .as_deref()
        .is_some_and(|h| h.contains("once per file")));
    assert!(errors[1].notes[0].file_path.is_none());
}

/// Prepare a temp project and say whether it was refused as a parse failure.
fn refused(files: &[(&str, &str)]) -> bool {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    matches!(
        prepare_project(&dir.path().join("main.tg"), false, None),
        Err(PipelineError::ElabFailed(ref m)) if m == "parse errors"
    )
}

// 13.10.26f AC3 (D2): the driver refuses a duplicate in the entry and in a
// submodule, and accepts a file without one.
#[test]
fn prepare_refuses_a_duplicate_in_any_module() {
    let entry_clean = "mod numbers;\nfn main() -> Nat { numbers::one() }\n";
    assert!(!refused(&[
        ("main.tg", entry_clean),
        ("numbers.tg", "pub fn one() -> Nat { 1 }\n")
    ]));
    let entry_twice = "fn one() -> Nat { 2 }\nfn one() -> Nat { 1 }\nfn main() -> Nat { one() }\n";
    assert!(refused(&[("main.tg", entry_twice)]));
    let submodule_twice = "pub fn one() -> Nat { 1 }\npub fn one() -> Nat { 2 }\n";
    assert!(refused(&[
        ("main.tg", entry_clean),
        ("numbers.tg", submodule_twice)
    ]));
}
