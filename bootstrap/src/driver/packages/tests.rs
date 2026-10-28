//! What `mount_packages` decides once the graph is planned — D6's graph-wide
//! collision check, D4's library-module clash and the package table — and the
//! one `PlanProblem` → `ElabError` site (ADR 29.9.26g AC 1–3, 3.10.26m,
//! 6.10.26j AC 1).
//!
//! The planner itself, the walk and the manifest schema, moved to
//! `albite_core::packages` with its tests (ADR 6.10.26j D2).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use albite_core::packages::{ByteRange, PlanProblem, PlanProblemKind};

use super::*;
use crate::elaborate::{ElabErrorKind, PackageErrorKind};

/// The moved planner, called as the bootstrap calls it: with `ALL_KEYWORDS`.
fn parse(text: &str) -> Result<Manifest, Vec<ElabError>> {
    parse_manifest(text, Path::new("/app/albite.toml"), ALL_KEYWORDS).map_err(plan_problem_errors)
}

// The application manifest `sibling_root` and `announcement` read; the core's
// manifest tests keep their own copy, which the move took with them.
const GOOD: &str = "\
[package]
name = \"app\"
version = \"0.1.0\"

[dependencies]
geometry = { path = \"../geometry\" }
units = { path = \"vendor/units\" }
";

fn codes(errors: &[ElabError]) -> Vec<&'static str> {
    errors.iter().map(|e| e.kind.code()).collect()
}

/// A planned package named `name`, whose manifest is `text` at `path`,
/// introduced at `key` in `by`'s text.
fn planned(
    name: &str,
    manifest: (&str, &str),
    by: (&str, &str),
    declares: Vec<usize>,
) -> PlannedPackage {
    let (path, text) = manifest;
    let (by_path, by_text) = by;
    let start = by_text.find(&format!("{name} =")).expect("key in text") as u32;
    PlannedPackage {
        name: name.to_string(),
        library_root: PathBuf::from(path).with_file_name("lib.tg"),
        manifest: parse_manifest(text, Path::new(path), ALL_KEYWORDS).expect("valid"),
        declares,
        introduced_by: (
            PathBuf::from(by_path),
            ByteRange::new(start, start + name.len() as u32),
        ),
    }
}

// 6.10.26j AC1: the one conversion keeps kind, span and file, and the core's
// message is the bootstrap's own rendering for every kind the planner raises.
#[test]
fn a_plan_problem_converts_with_its_kind_span_and_message() {
    let kinds = [
        PlanProblemKind::ManifestInvalid { reason: "r".into() },
        PlanProblemKind::DependencyNotFound {
            dependency: "d".into(),
            reason: "r".into(),
        },
        PlanProblemKind::PackageNameConflict {
            dependency: "d".into(),
            reason: "r".into(),
        },
        PlanProblemKind::DependencyCycle {
            chain: vec!["app".into(), "a".into(), "app".into()],
        },
    ];
    let expected_codes = ["E0120", "E0121", "E0122", "E0124"];
    for (kind, code) in kinds.into_iter().zip(expected_codes) {
        let problem = PlanProblem::new(kind, Path::new("/ws/albite.toml"), ByteRange::new(4, 9));
        let core_message = problem.message.clone();
        let error = plan_problem_error(problem);
        assert_eq!(error.kind.code(), code);
        assert_eq!(error.message, core_message);
        let ElabErrorKind::Package(kind) = &error.kind else {
            panic!("not a package error: {:?}", error.kind);
        };
        assert_eq!(kind.default_message(), core_message);
        assert_eq!((error.span.start, error.span.end), (4, 9));
        assert_eq!(
            error.file_path.as_deref(),
            Some(Path::new("/ws/albite.toml"))
        );
    }
}

// 6.10.26j AC1: a package named after a reserved word is refused with the
// bootstrap's list and accepted with an empty one.
#[test]
fn the_bootstrap_plans_with_its_own_keywords() {
    let text = "[package]\nname = \"match\"\n";
    let errors = parse(text).expect_err("`match` is a keyword");
    assert_eq!(codes(&errors), ["E0120"]);
    assert!(
        errors[0].message.contains("reserved word"),
        "{}",
        errors[0].message
    );
    let path = Path::new("/app/albite.toml");
    assert!(parse_manifest(text, path, &[]).is_ok());
}

const LIB_A: &str = "[package]\nname = \"lib_a\"\n\n[dependencies]\nlib_b = { path = \"../b\" }\nlib_c = { path = \"../c\" }\n";
const LIB_B: &str = "[package]\nname = \"lib_b\"\n";
const LIB_C: &str = "[package]\nname = \"lib_c\"\n";
const APP: &str = "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"../a\" }\n";

/// `app → lib_a → {lib_b, lib_c}`, walked in that order.
fn transitive_plan() -> DependencyPlan {
    DependencyPlan {
        packages: vec![
            planned(
                "lib_a",
                ("/a/albite.toml", LIB_A),
                ("/app/albite.toml", APP),
                vec![1, 2],
            ),
            planned(
                "lib_b",
                ("/b/albite.toml", LIB_B),
                ("/a/albite.toml", LIB_A),
                vec![],
            ),
            planned(
                "lib_c",
                ("/c/albite.toml", LIB_C),
                ("/a/albite.toml", LIB_A),
                vec![],
            ),
        ],
        application_declares: vec![0],
        notes: Vec::new(),
    }
}

fn names(entries: &[(&str, &str)]) -> BTreeMap<String, Definition> {
    entries
        .iter()
        .map(|(name, file)| {
            let definition = Definition {
                file: PathBuf::from(file),
                span: Span::new(3, 9),
            };
            ((*name).to_string(), definition)
        })
        .collect()
}

fn collision(error: &ElabError) -> (String, [String; 2]) {
    match &error.kind {
        ElabErrorKind::Package(PackageErrorKind::CrossPackageNameCollision { name, packages }) => {
            (name.clone(), packages.clone())
        }
        other => panic!("unexpected {other:?}"),
    }
}

// 29.9.26g AC3: D8 reports a name two packages share — with both definitions,
// including one only an application sibling defines — and nothing else.
#[test]
fn cross_package_collisions_follow_d8() {
    let app = parse(APP).expect("valid");
    let plan = transitive_plan();
    let app_names = names(&[("main", "/app/main.tg"), ("helper", "/app/sibling.tg")]);
    let packages = vec![
        (
            &plan.packages[0],
            names(&[("helper", "/a/lib.tg"), ("area", "/a/lib.tg")]),
        ),
        (
            &plan.packages[1],
            names(&[("area", "/b/lib.tg"), ("convert", "/b/lib.tg")]),
        ),
    ];
    let errors = cross_package_collisions(&app, &app_names, &packages);
    assert_eq!(codes(&errors), ["E0123", "E0123"]);
    assert_eq!(
        collision(&errors[0]),
        ("helper".into(), ["app".into(), "lib_a".into()])
    );
    assert_eq!(
        collision(&errors[1]),
        ("area".into(), ["lib_a".into(), "lib_b".into()])
    );
    let note_files: Vec<_> = errors[0]
        .notes
        .iter()
        .filter_map(|n| n.file_path.clone())
        .collect();
    assert_eq!(
        note_files,
        [PathBuf::from("/app/sibling.tg"), PathBuf::from("/a/lib.tg")]
    );
    assert_eq!(
        errors[0].file_path.as_deref(),
        Some(Path::new("/app/albite.toml"))
    );

    // A name duplicated inside one package is not a cross-package collision.
    let clean = vec![(&plan.packages[0], names(&[("area", "/a/lib.tg")]))];
    assert!(cross_package_collisions(&app, &names(&[("main", "/app/main.tg")]), &clean).is_empty());
}

// 3.10.26m D6: two transitive packages define `helper`, which neither the
// application nor `lib_a` does: the later one, `lib_c`, is spanned on its key
// in `lib_a/albite.toml`.
#[test]
fn a_collision_between_transitive_packages_spans_the_later_key() {
    let app = parse(APP).expect("valid");
    let plan = transitive_plan();
    let packages = vec![
        (&plan.packages[0], names(&[("a", "/a/lib.tg")])),
        (&plan.packages[1], names(&[("helper", "/b/lib.tg")])),
        (&plan.packages[2], names(&[("helper", "/c/lib.tg")])),
    ];
    let errors = cross_package_collisions(&app, &names(&[("main", "/app/main.tg")]), &packages);
    assert_eq!(codes(&errors), ["E0123"]);
    assert_eq!(
        collision(&errors[0]),
        ("helper".into(), ["lib_b".into(), "lib_c".into()])
    );
    assert_eq!(
        errors[0].file_path.as_deref(),
        Some(Path::new("/a/albite.toml"))
    );
    let span = errors[0].span;
    assert_eq!(&LIB_A[span.start as usize..span.end as usize], "lib_c");
}

// 3.10.26m AC3 (D4): a library whose dependency key equals one of its own
// top-level modules is E0122, spanned on that key in its own manifest.
#[test]
fn a_library_key_equal_to_its_own_module_is_e0122() {
    let plan = transitive_plan();
    let own: Vec<HashSet<String>> = vec![
        ["lib_c".to_string(), "shapes".to_string()].into(),
        HashSet::new(),
        ["lib_a".to_string()].into(),
    ];
    let errors = library_module_clashes(&plan, &own);
    assert_eq!(codes(&errors), ["E0122"]);
    assert_eq!(
        errors[0].file_path.as_deref(),
        Some(Path::new("/a/albite.toml"))
    );
    let span = errors[0].span;
    assert_eq!(&LIB_A[span.start as usize..span.end as usize], "lib_c");
    assert!(
        errors[0].message.contains("`lib_a` already has"),
        "{}",
        errors[0].message
    );
    assert!(
        library_module_clashes(&plan, &[HashSet::new(), HashSet::new(), HashSet::new()]).is_empty()
    );
}

// 3.10.26m D5: a nested dependency plans, and the table records who
// declared whom — the application its direct dependency only.
#[test]
fn the_package_table_records_each_package_declarations() {
    let app = parse(APP).expect("valid");
    let table = package_table(&app, &transitive_plan());
    let of = |name: &str| table.package_of(&ModulePath::from_name(name));
    assert_eq!(of("lib_c"), PackageId::Dependency(2));
    assert!(table.may_reach(PackageId::Application, of("lib_a")));
    assert!(!table.may_reach(PackageId::Application, of("lib_c")));
    assert!(table.may_reach(of("lib_a"), of("lib_b")));
    assert!(table.may_reach(of("lib_a"), of("lib_c")));
    assert!(!table.may_reach(of("lib_b"), of("lib_c")));
    assert_eq!(table.package_name(PackageId::Application), Some("app"));
    assert_eq!(table.declared_names(of("lib_a")), ["lib_b", "lib_c"]);
}

fn parsed(path: &str, source: &str, submodules: Vec<ParsedModule>) -> ParsedModule {
    let (source_file, errors) = crate::parser::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    ParsedModule {
        path: PathBuf::from(path),
        visibility: crate::ast::Visibility::Public,
        source_file,
        submodules,
        mount_name: None,
    }
}

// 4.10.26j AC5 (D6): D8's input narrowed to externs — every C symbol a tree's
// externs declare, submodules included, first declaration kept. Functions,
// theorems, lemmas and axioms may share a name across packages now, so none
// is collected; an explicit `symbol` is the key, not the extern's own name.
#[test]
fn extern_names_collects_only_extern_symbols_across_a_tree() {
    let inner = parsed(
        "/lib/inner.tg",
        "fn helper() -> Nat { 1 }\naxiom t(n: Nat): Bool\nextern \"C\" fn tg_x() -> Nat\n",
        vec![],
    );
    let mut root = parsed(
        "/lib/lib.tg",
        "type Shape = Square(Nat)\npub fn area() -> Nat { 1 }\nextern \"C\" fn tg_x() -> Nat\n\
         extern \"C\" fn local_name() -> Nat\n",
        vec![inner],
    );
    // The parser never sets `symbol` yet; the AST field is what is keyed on.
    for item in &mut root.source_file.items {
        if let Item::ExternFn(extern_fn) = item {
            if extern_fn.name.name == "local_name" {
                extern_fn.symbol = Some("tg_renamed".to_string());
            }
        }
    }
    let found = extern_names(&[&root]);
    let keys: Vec<_> = found.keys().map(String::as_str).collect();
    assert_eq!(keys, ["tg_renamed", "tg_x"]);
    assert_eq!(found["tg_x"].file, PathBuf::from("/lib/lib.tg"));
    assert!(found["tg_x"].span.end > found["tg_x"].span.start);
}

// 29.9.26g D1: siblings are discovered from the manifest's directory when
// there is one, else from today's workspace root, unchanged.
#[test]
fn sibling_root_follows_the_manifest() {
    let entry = Path::new("/nonexistent-29-9-26g/app/sub/x.tg");
    let manifest = parse(GOOD).expect("valid");
    assert_eq!(
        sibling_root(entry, &Ok(Some(manifest))),
        PathBuf::from("/app")
    );
    let fallback = PathBuf::from("/nonexistent-29-9-26g/app/sub");
    assert_eq!(sibling_root(entry, &Ok(None)), fallback);
    assert_eq!(sibling_root(entry, &Err(Vec::new())), fallback);
}

// 29.9.26g D11: the `--verbose` line names the dependencies, not the
// application, and prints nothing without the flag or a mounted package.
#[test]
fn announcement_names_the_mounted_packages() {
    let app = parse(GOOD).expect("valid");
    let geometry = parse("[package]\nname = \"geometry\"\n").expect("valid");
    let mut table = PackageTable::default();
    table.add_dependency("geometry", ModulePath::from_name("geometry"));
    let mounted = MountedPackages {
        table,
        manifests: vec![app.clone(), geometry],
    };
    assert_eq!(
        mounted.announcement(true).as_deref(),
        Some("Mounted 1 package(s): [\"geometry\"]")
    );
    assert_eq!(mounted.announcement(false), None);
    let empty = MountedPackages {
        table: PackageTable::default(),
        manifests: vec![app],
    };
    assert_eq!(empty.announcement(true), None);
    let echoed = mounted.announce(false);
    assert_eq!(echoed.manifests.len(), 2);
}
