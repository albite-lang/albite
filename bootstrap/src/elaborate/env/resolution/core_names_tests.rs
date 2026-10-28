//! ADR 4.10.26j AC1: Core names are collision-scoped.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::{scoped_spelling, CoreNames};
use crate::elaborate::env::{ModuleContents, ModulePath};

fn module(segments: &[&str]) -> ModulePath {
    ModulePath::new(segments.iter().map(|s| (*s).to_string()).collect())
}

fn def(segments: &[&str], name: &str) -> (ModulePath, String) {
    (module(segments), name.to_string())
}

// 4.10.26j AC1: a name defined in one module keeps its bare name.
#[test]
fn a_name_in_one_module_keeps_its_bare_name() {
    let names = CoreNames::from_definitions([def(&["x"], "helper"), def(&["y"], "other")], []);
    assert!(!names.is_colliding("helper"));
    assert_eq!(names.core_name(&module(&["x"]), "helper"), "helper");
    assert_eq!(names.source_name("helper"), "helper");
    assert!(names.origin("helper").is_none());
    assert!(names.definers("helper").is_empty());
}

// 4.10.26j AC1: a name in two modules is scoped in both, with no `::` or `$`.
#[test]
fn a_name_in_two_modules_is_scoped_in_both() {
    let names = CoreNames::from_definitions([def(&["x"], "helper"), def(&["y"], "helper")], []);
    assert!(names.is_colliding("helper"));
    let in_x = names.core_name(&module(&["x"]), "helper");
    let in_y = names.core_name(&module(&["y"]), "helper");
    assert_eq!(in_x, "_tg_1x_6helper");
    assert_eq!(in_y, "_tg_1y_6helper");
    for scoped in [&in_x, &in_y] {
        assert!(!scoped.contains("::") && !scoped.contains('$'), "{scoped}");
    }
    assert_eq!(names.definers("helper"), &[module(&["x"]), module(&["y"])]);
    assert_eq!(
        names.origin(&in_x),
        Some(&(module(&["x"]), "helper".to_string()))
    );
    assert_eq!(names.source_name(&in_y), "helper");
    // A module that does not define the name gets the bare spelling back.
    assert_eq!(names.core_name(&module(&["z"]), "helper"), "helper");
}

// 4.10.26j AC1: the root module's copy of a colliding name is scoped too.
#[test]
fn a_root_module_copy_is_scoped_by_the_empty_path() {
    let names = CoreNames::from_definitions([def(&[], "helper"), def(&["lib_a"], "helper")], []);
    assert_eq!(
        names.core_name(&ModulePath::root(), "helper"),
        "_tg_6helper"
    );
    assert_eq!(
        names.core_name(&module(&["lib_a"]), "helper"),
        "_tg_5lib_a_6helper"
    );
}

#[test]
fn nested_module_spelling_is_length_prefixed() {
    assert_eq!(
        scoped_spelling(&module(&["compiler", "lexer"]), "scan"),
        "_tg_8compiler_5lexer_4scan"
    );
}

// 4.10.26j AC1: a scoped name equal to a bare name in the program is refused.
#[test]
fn a_bare_name_spelling_a_scoped_name_is_recorded() {
    let names = CoreNames::from_definitions(
        [
            def(&["x"], "helper"),
            def(&["y"], "helper"),
            def(&["z"], "_tg_1x_6helper"),
        ],
        [],
    );
    let clash = names
        .spelt_scoped_name("_tg_1x_6helper")
        .expect("the clash is recorded");
    assert_eq!(clash.scoped_from, (module(&["x"]), "helper".to_string()));
    assert!(names.spelt_scoped_name("_tg_1y_6helper").is_none());
    assert!(names.spelt_scoped_name("helper").is_none());
}

// 4.10.26j AC1: an empty program has an empty collision set.
#[test]
fn an_empty_program_has_an_empty_collision_set() {
    let names = CoreNames::from_definitions(Vec::new(), []);
    assert_eq!(names.collision_set().count(), 0);
}

// 4.10.26j AC1: the table is the same whatever order the modules are listed in.
#[test]
fn the_table_ignores_definition_order() {
    let forward = CoreNames::from_definitions(
        [
            def(&["x"], "helper"),
            def(&["y"], "helper"),
            def(&["y"], "run"),
            def(&["z"], "run"),
            def(&["x"], "solo"),
        ],
        [],
    );
    let backward = CoreNames::from_definitions(
        [
            def(&["x"], "solo"),
            def(&["z"], "run"),
            def(&["y"], "run"),
            def(&["y"], "helper"),
            def(&["x"], "helper"),
        ],
        [],
    );
    let set = |n: &CoreNames| n.collision_set().map(str::to_string).collect::<Vec<_>>();
    assert_eq!(set(&forward), vec!["helper", "run"]);
    assert_eq!(set(&forward), set(&backward));
    assert_eq!(forward.definers("run"), backward.definers("run"));
}

// A module registered under two prefixes is one definer; a re-export is not a
// definition; a names-only module defines nothing.
#[test]
fn the_registry_counts_each_file_once_and_skips_reexports() {
    let mut modules = HashMap::new();
    let mut module_files = HashMap::new();
    let mut file_to_module = HashMap::new();
    let x_file = PathBuf::from("x.tg");
    for (path, file) in [
        (module(&["x"]), x_file.clone()),
        (module(&["main", "x"]), x_file.clone()),
    ] {
        let contents = ModuleContents {
            values: vec!["helper".to_string()],
            ..ModuleContents::default()
        };
        modules.insert(path.clone(), contents);
        module_files.insert(path, file);
    }
    file_to_module.insert(x_file, module(&["x"]));
    let mut reexporter = ModuleContents {
        values: vec!["helper".to_string()],
        ..ModuleContents::default()
    };
    reexporter
        .reexported_value_sources
        .insert("helper".to_string(), (module(&["x"]), "helper".to_string()));
    modules.insert(ModulePath::root(), reexporter);
    let lender = ModuleContents {
        values: vec!["helper".to_string()],
        ..ModuleContents::default()
    };
    modules.insert(module(&["sibling"]), lender);
    let names_only: HashSet<_> = [module(&["sibling"])].into_iter().collect();

    let names =
        CoreNames::from_module_registry(&modules, &module_files, &file_to_module, &names_only);
    assert!(!names.is_colliding("helper"));

    let both =
        CoreNames::from_module_registry(&modules, &module_files, &file_to_module, &HashSet::new());
    assert_eq!(
        both.definers("helper"),
        &[module(&["sibling"]), module(&["x"])]
    );
}

// 9.10.26f AC3: a reserved link name one module defines is scoped there, and
// "colliding" keeps its two-definer meaning.
#[test]
fn a_reserved_single_definer_is_scoped() {
    let names = CoreNames::from_definitions([def(&[], "write"), def(&["x"], "helper")], []);
    assert!(names.is_scoped("write"));
    assert!(!names.is_colliding("write"));
    assert!(names.definers("write").is_empty());
    assert_eq!(names.core_name(&ModulePath::root(), "write"), "_tg_5write");
    assert_eq!(names.source_name("_tg_5write"), "write");
    // Another module asking for its own `write` gets the bare spelling.
    assert_eq!(names.core_name(&module(&["x"]), "write"), "write");
    // A non-reserved single name stays bare and unscoped.
    assert!(!names.is_scoped("helper"));
    assert_eq!(names.core_name(&module(&["x"]), "helper"), "helper");

    let nested = CoreNames::from_definitions([def(&["x"], "write")], []);
    assert_eq!(nested.core_name(&module(&["x"]), "write"), "_tg_1x_5write");
}

// 9.10.26f AC3: bare resolution reads the scoped definers — a reserved name's
// one module, a colliding name's modules, nothing for a plain name.
#[test]
fn scoped_definers_cover_reserved_and_colliding_names() {
    let names = CoreNames::from_definitions(
        [
            def(&["x"], "write"),
            def(&["x"], "helper"),
            def(&["y"], "helper"),
            def(&["y"], "plain"),
        ],
        [],
    );
    assert_eq!(names.scoped_definers("write"), &[module(&["x"])]);
    assert_eq!(
        names.scoped_definers("helper"),
        &[module(&["x"]), module(&["y"])]
    );
    assert!(names.scoped_definers("plain").is_empty());
}

// 9.10.26f AC3: a user-written `_tg_5write` beside a reserved `write` is the
// same Core name, and is recorded for refusal.
#[test]
fn a_bare_name_spelling_a_reserved_scoped_name_is_recorded() {
    let names = CoreNames::from_definitions([def(&[], "write"), def(&["y"], "_tg_5write")], []);
    let clash = names
        .spelt_scoped_name("_tg_5write")
        .expect("the clash is recorded");
    assert_eq!(clash.scoped_from, (ModulePath::root(), "write".to_string()));
}

// 9.10.26f AC3: an `extern "C" fn write` keeps its bare Core name.
#[test]
fn an_extern_reserved_name_stays_bare() {
    let names = CoreNames::from_definitions([def(&["x"], "write")], [def(&["x"], "write")]);
    assert!(!names.is_scoped("write"));
    assert_eq!(names.core_name(&module(&["x"]), "write"), "write");
    assert!(names.scoped_set().is_empty());
}

// 9.10.26f AC3: the registry hands the extern set through, per module.
#[test]
fn the_registry_passes_extern_declarations_through() {
    let mut modules = HashMap::new();
    let mut extern_module = ModuleContents {
        values: vec!["write".to_string(), "abort".to_string()],
        ..ModuleContents::default()
    };
    extern_module.extern_values.insert("write".to_string());
    modules.insert(module(&["ffi"]), extern_module);
    let names = CoreNames::from_module_registry(
        &modules,
        &HashMap::new(),
        &HashMap::new(),
        &HashSet::new(),
    );
    assert!(!names.is_scoped("write"));
    assert_eq!(
        names.core_name(&module(&["ffi"]), "abort"),
        "_tg_3ffi_5abort"
    );
}
