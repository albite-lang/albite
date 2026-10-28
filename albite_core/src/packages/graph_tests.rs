//! The graph walk over in-memory probes (ADR 3.10.26m AC 1–3): chains,
//! diamonds, cycles, and the naming rules, with slice 1's per-package checks
//! (29.9.26g AC 3) carried over to every depth.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::packages::manifest_tests::TEST_RESERVED;

/// An in-memory filesystem: files by path, directories implied by prefixes,
/// and an alias table standing in for symlinks (`canonical` follows it).
#[derive(Default)]
struct Files {
    files: HashMap<PathBuf, String>,
    aliases: HashMap<PathBuf, PathBuf>,
    unresolvable: HashSet<PathBuf>,
}

impl Files {
    fn new(entries: &[(&str, &str)]) -> Self {
        let files = entries
            .iter()
            .map(|(p, t)| (PathBuf::from(p), (*t).to_string()))
            .collect();
        Self {
            files,
            ..Self::default()
        }
    }
    fn alias(mut self, from: &str, to: &str) -> Self {
        self.aliases.insert(PathBuf::from(from), PathBuf::from(to));
        self
    }
    fn exists(&self, p: &Path) -> bool {
        let p = self.canonical_of(p);
        self.files.keys().any(|k| k == &p || k.starts_with(&p))
    }
    fn canonical_of(&self, p: &Path) -> PathBuf {
        let folded = fold_dots(p);
        self.aliases.get(&folded).cloned().unwrap_or(folded)
    }
    fn canonical(&self, p: &Path) -> Result<PathBuf, String> {
        if self.unresolvable.contains(p) {
            return Err("permission denied".into());
        }
        Ok(self.canonical_of(p))
    }
    fn read(&self, p: &Path) -> Option<String> {
        self.files.get(&self.canonical_of(p)).cloned()
    }
}

/// A package directory's two files: its manifest and an empty `lib.tg`.
fn package(dir: &str, name: &str, deps: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut manifest = format!("[package]\nname = \"{name}\"\n");
    if !deps.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for (key, path) in deps {
            manifest.push_str(&format!("{key} = {{ path = \"{path}\" }}\n"));
        }
    }
    vec![
        (format!("{dir}/albite.toml"), manifest),
        (format!("{dir}/lib.tg"), String::new()),
    ]
}

/// A filesystem holding `packages`, and the application at `/ws/app`.
fn files(packages: &[Vec<(String, String)>]) -> Files {
    let flat: Vec<(String, String)> = packages.iter().flatten().cloned().collect();
    let mut entries: Vec<(&str, &str)> =
        flat.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    entries.push(("/ws/app/main.tg", ""));
    Files::new(&entries)
}

fn app(deps: &[(&str, &str)]) -> Manifest {
    let text = &package("/ws/app", "app", deps)[0].1;
    parse_manifest(text, Path::new("/ws/app/albite.toml"), TEST_RESERVED).expect("valid")
}

fn plan_with(
    fs: &Files,
    app: &Manifest,
    app_modules: &[&str],
) -> Result<DependencyPlan, Vec<PlanProblem>> {
    let modules: HashSet<String> = app_modules.iter().map(|s| (*s).to_string()).collect();
    let probes = Probes {
        canonical: &|p| fs.canonical(p),
        exists: &|p| fs.exists(p),
        read: &|p| fs.read(p),
        reserved: TEST_RESERVED,
    };
    plan_graph(app, &modules, &probes)
}

/// Each problem's tag as the bootstrap's code.
fn codes(errors: &[PlanProblem]) -> Vec<String> {
    errors
        .iter()
        .map(|e| format!("E0{}", e.kind.tag()))
        .collect()
}

fn names(plan: &DependencyPlan) -> Vec<&str> {
    plan.packages.iter().map(|p| p.name.as_str()).collect()
}

/// The source text an error's span covers, in the manifest it names.
fn spanned<'a>(fs: &'a Files, error: &PlanProblem) -> (&'a str, PathBuf) {
    let file = error.file.clone();
    let text = &fs.files[&file];
    (
        &text[error.range.start as usize..error.range.end as usize],
        file,
    )
}

// 3.10.26m AC1: a chain plans three packages, each with its own list.
#[test]
fn a_chain_plans_every_package_with_its_dependencies() {
    let fs = files(&[
        package("/ws/a", "a", &[("b", "../b")]),
        package("/ws/b", "b", &[]),
    ]);
    let plan = plan_with(&fs, &app(&[("a", "../a")]), &[]).expect("planned");
    assert_eq!(names(&plan), ["a", "b"]);
    assert_eq!(plan.application_declares, [0]);
    assert_eq!(plan.packages[0].declares, [1]);
    assert!(plan.packages[1].declares.is_empty());
    // `..` is folded away in the path diagnostics print.
    assert_eq!(plan.packages[1].library_root, PathBuf::from("/ws/b/lib.tg"));
    assert_eq!(
        plan.packages[1].introduced_by.0,
        PathBuf::from("/ws/a/albite.toml")
    );
}

// 3.10.26m AC1: a diamond plans its shared package once, by canonical
// directory, even when the two dependents spell its path differently.
#[test]
fn a_diamond_plans_its_shared_package_once() {
    let fs = files(&[
        package("/ws/a", "a", &[("c", "../c")]),
        package("/ws/b", "b", &[("c", "../link/c")]),
        package("/ws/c", "c", &[]),
    ])
    .alias("/ws/link/c", "/ws/c");
    let plan = plan_with(&fs, &app(&[("a", "../a"), ("b", "../b")]), &[]).expect("planned");
    assert_eq!(names(&plan), ["a", "c", "b"]);
    assert_eq!(plan.packages[0].declares, [1]);
    assert_eq!(plan.packages[2].declares, [1]);
    assert_eq!(plan.application_declares, [0, 2]);
}

// 3.10.26m AC1: each package's own refusals carry slice 1's codes at every
// depth, and every one in the graph is collected.
#[test]
fn every_refusal_in_the_graph_is_collected() {
    let fs = files(&[
        package("/ws/a", "a", &[("gone", "../gone"), ("misnamed", "../m")]),
        package("/ws/m", "other", &[]),
        vec![("/ws/bad/albite.toml".into(), "[package\n".into())],
        vec![("/ws/bad/lib.tg".into(), String::new())],
        vec![(
            "/ws/nolib/albite.toml".into(),
            "[package]\nname = \"nolib\"\n".into(),
        )],
        vec![("/ws/nomanifest/lib.tg".into(), String::new())],
    ]);
    let deps = [
        ("a", "../a"),
        ("bad", "../bad"),
        ("nolib", "../nolib"),
        ("nomanifest", "../nomanifest"),
    ];
    let errors = plan_with(&fs, &app(&deps), &[]).expect_err("refused");
    assert_eq!(
        codes(&errors),
        ["E0121", "E0122", "E0120", "E0121", "E0121"]
    );
    assert!(
        errors[0].message.contains("does not exist"),
        "{}",
        errors[0].message
    );
    let (key, file) = spanned(&fs, &errors[1]);
    assert_eq!(key, "misnamed");
    assert_eq!(file, PathBuf::from("/ws/a/albite.toml"));
    assert!(errors[1].message.contains("named `other`"));
    assert_eq!(errors[2].file, Path::new("/ws/bad/albite.toml"));
    assert!(
        errors[3].message.contains("no `lib.tg`"),
        "{}",
        errors[3].message
    );
    assert!(
        errors[4].message.contains("no `albite.toml`"),
        "{}",
        errors[4].message
    );
}

// 3.10.26m D2 (2): a directory that cannot be canonicalised is E0121 with
// the I/O reason, never a panic.
#[test]
fn an_unresolvable_directory_is_e0121_with_its_reason() {
    let mut fs = files(&[package("/ws/a", "a", &[])]);
    fs.unresolvable.insert(PathBuf::from("/ws/app/../a"));
    let errors = plan_with(&fs, &app(&[("a", "../a")]), &[]).expect_err("refused");
    assert_eq!(codes(&errors), ["E0121"]);
    assert!(
        errors[0].message.contains("permission denied"),
        "{}",
        errors[0].message
    );
}

/// The cycle error's chain, which must be the only refusal.
fn cycle(errors: &[PlanProblem]) -> Vec<String> {
    assert_eq!(codes(errors), ["E0124"], "{errors:?}");
    match &errors[0].kind {
        PlanProblemKind::DependencyCycle { chain } => chain.clone(),
        other => panic!("not a cycle: {other:?}"),
    }
}

// 3.10.26m AC2: `a → b → a` names the chain, spanned on the closing key in
// the manifest that holds it.
#[test]
fn a_two_package_cycle_names_its_chain() {
    let fs = files(&[
        package("/ws/a", "a", &[("b", "../b")]),
        package("/ws/b", "b", &[("a", "../a")]),
    ]);
    let errors = plan_with(&fs, &app(&[("a", "../a")]), &[]).expect_err("cycle");
    assert_eq!(cycle(&errors), ["app", "a", "b", "a"]);
    assert_eq!(errors[0].message, "dependency cycle: app → a → b → a");
    let (key, file) = spanned(&fs, &errors[0]);
    assert_eq!((key, file), ("a", PathBuf::from("/ws/b/albite.toml")));
}

// 3.10.26m AC2: a package naming itself, and a library naming the
// application's directory (which has no `lib.tg`), are cycles too.
#[test]
fn a_self_dependency_and_the_application_directory_are_cycles() {
    let fs = files(&[package("/ws/a", "a", &[("a", ".")])]);
    let errors = plan_with(&fs, &app(&[("a", "../a")]), &[]).expect_err("cycle");
    assert_eq!(cycle(&errors), ["app", "a", "a"]);

    let fs = files(&[package("/ws/a", "a", &[("app", "../app")])]);
    let errors = plan_with(&fs, &app(&[("a", "../a")]), &[]).expect_err("cycle");
    assert_eq!(cycle(&errors), ["app", "a", "app"]);
}

// 3.10.26m AC3: two directories with one name are E0122 naming both.
#[test]
fn two_directories_with_one_name_are_e0122() {
    let fs = files(&[
        package("/ws/a", "a", &[("c", "../c")]),
        package("/ws/b", "b", &[("c", "../vendor/c")]),
        package("/ws/c", "c", &[]),
        package("/ws/vendor/c", "c", &[]),
    ]);
    let errors = plan_with(&fs, &app(&[("a", "../a"), ("b", "../b")]), &[]).expect_err("refused");
    assert_eq!(codes(&errors), ["E0122"]);
    let message = &errors[0].message;
    assert!(
        message.contains("`/ws/c`") && message.contains("`/ws/vendor/c`"),
        "{message}"
    );
    let (key, file) = spanned(&fs, &errors[0]);
    assert_eq!((key, file), ("c", PathBuf::from("/ws/b/albite.toml")));
}

// 3.10.26m AC3: a transitive package named like an application top-level
// module is E0122, as slice 1 refuses a direct one.
#[test]
fn a_transitive_package_named_like_an_app_module_is_e0122() {
    let fs = files(&[
        package("/ws/a", "a", &[("util", "../util")]),
        package("/ws/util", "util", &[]),
    ]);
    let errors = plan_with(&fs, &app(&[("a", "../a")]), &["util"]).expect_err("refused");
    assert_eq!(codes(&errors), ["E0122"]);
    assert!(
        errors[0].message.contains("top-level module"),
        "{}",
        errors[0].message
    );
    let (key, file) = spanned(&fs, &errors[0]);
    assert_eq!((key, file), ("util", PathBuf::from("/ws/a/albite.toml")));
}

// 3.10.26m D2: a broken package reached twice is reported once.
#[test]
fn a_refused_package_reached_twice_is_reported_once() {
    let fs = files(&[
        package("/ws/a", "a", &[("c", "../c")]),
        package("/ws/b", "b", &[("c", "../c")]),
        vec![(
            "/ws/c/albite.toml".into(),
            "[package]\nname = \"c\"\n".into(),
        )],
    ]);
    let errors = plan_with(&fs, &app(&[("a", "../a"), ("b", "../b")]), &[]).expect_err("refused");
    assert_eq!(codes(&errors), ["E0121"]);
}

// 3.10.26m D2: `.` and `..` fold lexically; a leading `..` stays.
#[test]
fn fold_dots_folds_lexically() {
    assert_eq!(fold_dots(Path::new("a/./b/../c")), PathBuf::from("a/c"));
    assert_eq!(fold_dots(Path::new("../x")), PathBuf::from("../x"));
    assert_eq!(fold_dots(Path::new("a/..")), PathBuf::from("."));
    assert_eq!(fold_dots(Path::new("/ws/app/../b")), PathBuf::from("/ws/b"));
}

// 3.10.26m D2: the printed directory is the folded one only while it names
// the same directory — behind a symlink it is the path as written.
#[test]
fn the_display_directory_keeps_a_symlinked_spelling() {
    let fs = files(&[package("/ws/real/c", "c", &[])]).alias("/ws/link", "/ws/real");
    let probes = Probes {
        canonical: &|p| fs.canonical(p),
        exists: &|p| fs.exists(p),
        read: &|p| fs.read(p),
        reserved: TEST_RESERVED,
    };
    let plain = Path::new("/ws/app/../real/c");
    assert_eq!(
        display_dir(plain, Path::new("/ws/real/c"), &probes),
        PathBuf::from("/ws/real/c")
    );
    // `/ws/link/..` is `/ws` lexically, but `/ws/real/..` = `/ws` physically;
    // here the folded path names another directory, so the spelling stays.
    let tricky = Path::new("/ws/link/../c");
    assert_eq!(
        display_dir(tricky, Path::new("/ws/real/c"), &probes),
        PathBuf::from("/ws/link/../c")
    );
}
