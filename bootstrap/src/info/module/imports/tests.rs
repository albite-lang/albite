use super::*;
use albite_bootstrap::ast::SourceFile;
use albite_bootstrap::driver::{AdtTypes, ProjectOutput, RecordTypes, TypeAliases};
use albite_bootstrap::elaborate::{CoreDef, TypeProvenance};
use albite_bootstrap::span::Span;
use std::collections::HashMap;
use std::path::PathBuf;

fn leaf_module(name: &str) -> ParsedModule {
    ParsedModule {
        path: PathBuf::from(format!("{name}.tg")),
        visibility: albite_bootstrap::ast::Visibility::Public,
        source_file: SourceFile {
            items: vec![],
            span: Span { start: 0, end: 0 },
        },
        submodules: vec![],
        mount_name: None,
    }
}

fn empty_project() -> ProjectOutput {
    ProjectOutput {
        defs: vec![],
        codegen_units: vec![],
        record_types: RecordTypes::new(),
        adt_types: AdtTypes::new(),
        type_aliases: TypeAliases::new(),
        type_provenance: TypeProvenance {
            mu_origins: HashMap::new(),
        },
        source_map: albite_bootstrap::driver::SourceMap::new(),
        encoded_types: HashMap::new(),
        mutual_recursion_groups: HashMap::new(),
        type_visibilities: HashMap::new(),
        record_field_visibilities: HashMap::new(),
        value_import_targets: std::collections::BTreeMap::new(),
        termination_meta: std::collections::HashMap::new(),
    }
}

#[test]
fn classify_import_adt() {
    let mut project = empty_project();
    let ctor = albite_bootstrap::elaborate::Constructor {
        name: "Some".to_string(),
        fields: vec![],
        index: 0,
        visibility: None,
        span: Span { start: 0, end: 0 },
    };
    project
        .adt_types
        .insert("Option".to_string(), (vec!["T".to_string()], vec![ctor]));
    let ctors = BTreeMap::new();
    let status = classify_import("Option", &project, &ctors);
    assert!(!status.is_stub);
    assert!(status.description.contains("ADT"));
    assert!(status.description.contains("1 variant"));
    assert!(status.description.contains("1 param"));
}

#[test]
fn classify_import_record() {
    let mut project = empty_project();
    project.record_types.insert(
        "Point".to_string(),
        vec![
            ("x".to_string(), albite_core::types::Type::Nat),
            ("y".to_string(), albite_core::types::Type::Nat),
        ],
    );
    let ctors = BTreeMap::new();
    let status = classify_import("Point", &project, &ctors);
    assert!(!status.is_stub);
    assert!(status.description.contains("record"));
    assert!(status.description.contains("2 field"));
}

#[test]
fn classify_import_constructor() {
    let project = empty_project();
    let mut ctors = BTreeMap::new();
    ctors.insert("Some".to_string(), "Option".to_string());
    let status = classify_import("Some", &project, &ctors);
    assert!(!status.is_stub);
    assert!(status.description.contains("constructor"));
    assert!(status.description.contains("Option"));
}

#[test]
fn classify_import_value() {
    let mut project = empty_project();
    project.defs.push(CoreDef {
        name: "my_func".to_string(),
        ty: albite_core::types::Type::Nat,
        term: albite_core::terms::SpannedTerm::new(
            albite_core::terms::Term::Zero,
            albite_core::terms::TermSpan::new(0, 0),
        ),
        span: Span { start: 0, end: 0 },
    });
    let ctors = BTreeMap::new();
    let status = classify_import("my_func", &project, &ctors);
    assert!(!status.is_stub);
    assert!(status.description.contains("value"));
}

#[test]
fn classify_import_missing() {
    let project = empty_project();
    let ctors = BTreeMap::new();
    let status = classify_import("Unknown", &project, &ctors);
    assert!(status.is_stub);
    assert!(status.description.contains("not resolved"));
}

#[test]
fn find_module_exact_match() {
    let tree = ParsedModule {
        path: PathBuf::from("main.tg"),
        visibility: albite_bootstrap::ast::Visibility::Public,
        source_file: SourceFile {
            items: vec![],
            span: Span { start: 0, end: 0 },
        },
        submodules: vec![leaf_module("parser"), leaf_module("lexer")],
        mount_name: None,
    };
    let results = find_modules_by_path(&tree, "main::parser", "");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "main::parser");
}

#[test]
fn find_module_short_name() {
    let tree = ParsedModule {
        path: PathBuf::from("main.tg"),
        visibility: albite_bootstrap::ast::Visibility::Public,
        source_file: SourceFile {
            items: vec![],
            span: Span { start: 0, end: 0 },
        },
        submodules: vec![leaf_module("parser")],
        mount_name: None,
    };
    let results = find_modules_by_path(&tree, "parser", "");
    assert_eq!(results.len(), 1);
}

#[test]
fn find_module_ambiguous() {
    let tree = ParsedModule {
        path: PathBuf::from("main.tg"),
        visibility: albite_bootstrap::ast::Visibility::Public,
        source_file: SourceFile {
            items: vec![],
            span: Span { start: 0, end: 0 },
        },
        submodules: vec![
            ParsedModule {
                path: PathBuf::from("elab/mod.tg"),
                visibility: albite_bootstrap::ast::Visibility::Public,
                source_file: SourceFile {
                    items: vec![],
                    span: Span { start: 0, end: 0 },
                },
                submodules: vec![leaf_module("ffi")],
                mount_name: None,
            },
            ParsedModule {
                path: PathBuf::from("driver/mod.tg"),
                visibility: albite_bootstrap::ast::Visibility::Public,
                source_file: SourceFile {
                    items: vec![],
                    span: Span { start: 0, end: 0 },
                },
                submodules: vec![leaf_module("ffi")],
                mount_name: None,
            },
        ],
        mount_name: None,
    };
    let results = find_modules_by_path(&tree, "ffi", "");
    assert_eq!(results.len(), 2);
}

#[test]
fn resolve_exact_over_ambiguous() {
    let m1 = leaf_module("ffi");
    let m2 = leaf_module("ffi");
    let candidates = vec![
        ("main::elab::ffi".to_string(), &m1),
        ("main::driver::ffi".to_string(), &m2),
    ];
    // Exact match should resolve
    let result = resolve_module_target(&[("main::elab::ffi".to_string(), &m1)], "main::elab::ffi");
    assert!(result.is_some());

    // Ambiguous should fail
    let result = resolve_module_target(&candidates, "ffi");
    assert!(result.is_none());
}
