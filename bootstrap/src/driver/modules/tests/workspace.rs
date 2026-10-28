//! Tests for the workspace root listing (ADR 30.9.26i D4, AC 4) and the
//! merge's names-only mark (ADR 2.10.26g D4).
//!
//! Pure functions over injected paths: nothing here changes the process's
//! working directory. The end-to-end half, through the binary with
//! `Command::current_dir`, is `bootstrap/tests/workspace_fixture.rs`.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use crate::driver::modules::workspace::{
    discover_sibling_modules, listing_dir, merge_module_info, sibling_path,
};
use crate::driver::modules::ModuleInfo;
use crate::elaborate::{ModuleContents, ModulePath};

// ADR 30.9.26i AC4: the empty root — a bare entry's — lists the working directory.
#[test]
fn empty_root_lists_the_working_directory() {
    assert_eq!(listing_dir(Path::new("")), Path::new("."));
}

// ADR 30.9.26i AC4: every non-empty root is listed as given.
#[test]
fn non_empty_roots_are_unchanged() {
    for root in [".", "src/compiler", "/tmp", "a/b/../c"] {
        assert_eq!(
            listing_dir(Path::new(root)),
            Path::new(root),
            "root {root:?}"
        );
    }
}

// ADR 30.9.26i AC4: a bare entry's sibling is `lexer.tg`, not `./lexer.tg`,
// and a rooted one is unchanged.
#[test]
fn bare_siblings_carry_no_dot_prefix() {
    let bare = sibling_path(Path::new(""), OsStr::new("lexer.tg"));
    assert_eq!(bare, PathBuf::from("lexer.tg"));
    assert!(!bare.to_string_lossy().starts_with("./"));
    assert_eq!(
        sibling_path(Path::new("src/compiler"), OsStr::new("main.tg")),
        PathBuf::from("src/compiler/main.tg")
    );
}

// ADR 30.9.26i D6: a rooted discovery still lists files and `*/mod.tg`, sorted,
// spelled under the root, and skips a root-level `mod.tg`.
#[test]
fn rooted_discovery_is_sorted_and_spelled_under_the_root() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("util.tg"), "").unwrap();
    fs::write(dir.path().join("mod.tg"), "").unwrap();
    fs::write(dir.path().join("notes.txt"), "").unwrap();
    fs::create_dir(dir.path().join("codegen")).unwrap();
    fs::write(dir.path().join("codegen/mod.tg"), "").unwrap();
    fs::create_dir(dir.path().join("empty")).unwrap();

    let found = discover_sibling_modules(dir.path());
    assert_eq!(
        found,
        vec![
            dir.path().join("codegen/mod.tg"),
            dir.path().join("util.tg")
        ]
    );
}

// ADR 2.10.26j AC2 (D7): a paired companion never becomes a sibling module.
#[test]
fn sibling_scan_skips_a_companion() {
    let dir = TempDir::new().unwrap();
    for name in ["util.tg", "util_test.tg", "main.tg"] {
        fs::write(dir.path().join(name), "").unwrap();
    }
    let found = discover_sibling_modules(dir.path());
    assert_eq!(
        found,
        vec![dir.path().join("main.tg"), dir.path().join("util.tg")]
    );
}

// ADR 2.10.26j AC2 (D7): a `*_test.tg` with no neighbour is still a module.
#[test]
fn sibling_scan_keeps_an_unpaired_test_file() {
    let dir = TempDir::new().unwrap();
    for name in ["self_test.tg", "test_entry_test.tg", "test_entry.tg"] {
        fs::write(dir.path().join(name), "").unwrap();
    }
    let found = discover_sibling_modules(dir.path());
    assert_eq!(
        found,
        vec![
            dir.path().join("self_test.tg"),
            dir.path().join("test_entry.tg"),
            dir.path().join("test_entry_test.tg"),
        ]
    );
}

fn module_path(name: &str) -> ModulePath {
    ModulePath::from_segments(&[name.to_string()])
}

/// `info` plus a module at `name` read from `file`, as the registration pass
/// records one: in `modules`, `module_files` and `file_to_module`.
fn with_module(mut info: ModuleInfo, name: &str, file: &str) -> ModuleInfo {
    info.modules
        .insert(module_path(name), ModuleContents::default());
    info.module_files
        .insert(module_path(name), PathBuf::from(file));
    info.file_to_module
        .insert(PathBuf::from(file), module_path(name));
    info
}

// ADR 2.10.26g AC3 (D4): a module only the sibling contributes is marked.
#[test]
fn merge_marks_inserted_sibling_paths_names_only() {
    let main = with_module(ModuleInfo::default(), "lexer", "lexer.tg");
    let sibling = with_module(ModuleInfo::default(), "boxes", "boxes.tg");
    let merged = merge_module_info(main, sibling);
    assert!(merged.modules.contains_key(&module_path("boxes")));
    assert!(merged.names_only.contains(&module_path("boxes")));
    assert_eq!(merged.names_only.len(), 1);
}

// ADR 2.10.26g AC3 (D4): a path main already registers is never marked, and
// keeps main's contents (the merge's `or_insert` rule).
#[test]
fn merge_never_marks_a_priority_path() {
    let main = with_module(ModuleInfo::default(), "lexer", "lexer.tg");
    let sibling = with_module(ModuleInfo::default(), "lexer", "sub/lexer.tg");
    let merged = merge_module_info(main, sibling);
    assert!(merged.names_only.is_empty());
    assert_eq!(
        merged.module_files.get(&module_path("lexer")),
        Some(&PathBuf::from("lexer.tg"))
    );
}

// ADR 2.10.26g AC3 (D4): the bootstrap has no same-file drop, so a sibling
// module whose FILE main already owns is still inserted — but not marked.
#[test]
fn merge_inserts_but_never_marks_a_module_whose_file_main_owns() {
    let main = with_module(ModuleInfo::default(), "lexer", "lexer.tg");
    let sibling = with_module(ModuleInfo::default(), "main_lexer", "lexer.tg");
    let merged = merge_module_info(main, sibling);
    assert!(merged.modules.contains_key(&module_path("main_lexer")));
    assert!(merged.names_only.is_empty());
}

// ADR 2.10.26g AC3 (D4): the fallback's own marks are ignored — the merge
// decides from what it inserts.
#[test]
fn merge_ignores_fallback_names_only() {
    let main = with_module(ModuleInfo::default(), "lexer", "lexer.tg");
    let mut sibling = with_module(ModuleInfo::default(), "lexer", "sub/lexer.tg");
    sibling.names_only.insert(module_path("lexer"));
    sibling.names_only.insert(module_path("elsewhere"));
    let merged = merge_module_info(main, sibling);
    assert!(merged.names_only.is_empty());
}
