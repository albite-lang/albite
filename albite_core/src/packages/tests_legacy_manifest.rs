//! The manifest's two names (ADR 20.10.26i D1, D2): which file a directory is
//! read under, and the non-fatal notes the plan carries for the old one.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::packages::manifest_tests::TEST_RESERVED;

/// A manifest naming `deps` as `key = { path = "…" }`.
fn manifest_text(name: &str, deps: &[(&str, &str)]) -> String {
    let mut text = format!("[package]\nname = \"{name}\"\n\n[dependencies]\n");
    for (key, path) in deps {
        text.push_str(&format!("{key} = {{ path = \"{path}\" }}\n"));
    }
    text
}

/// Plan the application at `app_path` over an in-memory filesystem whose
/// directories are implied by the files under them; nothing is a symlink.
fn plan_over(files: &[(&str, String)], app_path: &str) -> DependencyPlan {
    let files: HashMap<PathBuf, String> = files
        .iter()
        .map(|(p, t)| (PathBuf::from(p), t.clone()))
        .collect();
    let exists = |p: &Path| {
        let p = fold_dots(p);
        files.keys().any(|k| k == &p || k.starts_with(&p))
    };
    let probes = Probes {
        canonical: &|p| Ok(fold_dots(p)),
        exists: &exists,
        read: &|p| files.get(&fold_dots(p)).cloned(),
        reserved: TEST_RESERVED,
    };
    let app = parse_manifest(
        &files[Path::new(app_path)],
        Path::new(app_path),
        TEST_RESERVED,
    )
    .expect("a valid application manifest");
    plan_graph(&app, &HashSet::<String>::new(), &probes).expect("the graph plans")
}

fn note_files(plan: &DependencyPlan) -> Vec<&Path> {
    plan.notes.iter().map(|n| n.file.as_path()).collect()
}

// ADR 20.10.26i AC1: the manifest choice covers all four cases.
#[test]
fn choose_manifest_covers_all_four_cases() {
    assert_eq!(choose_manifest(true, false), ManifestChoice::New);
    assert_eq!(choose_manifest(false, true), ManifestChoice::Legacy);
    assert_eq!(
        choose_manifest(true, true),
        ManifestChoice::NewShadowsLegacy
    );
    assert_eq!(choose_manifest(false, false), ManifestChoice::None);
}

// ADR 20.10.26i AC1: each choice picks its file and carries its note.
#[test]
fn manifest_in_reads_the_chosen_file_with_its_note() {
    let dir = Path::new("/p");
    let only = |present: &'static [&'static str]| {
        move |p: &Path| present.iter().any(|f| Path::new(f) == p)
    };
    let (path, note) = manifest_in(dir, only(&["/p/albite.toml"])).expect("new");
    assert_eq!((path.as_path(), note), (Path::new("/p/albite.toml"), None));

    let (path, note) = manifest_in(dir, only(&["/p/tungsten.toml"])).expect("legacy");
    assert_eq!(path, Path::new("/p/tungsten.toml"));
    assert_eq!(
        note,
        Some(PlanNote::legacy_name(Path::new("/p/tungsten.toml")))
    );

    let both = only(&["/p/albite.toml", "/p/tungsten.toml"]);
    let (path, note) = manifest_in(dir, both).expect("both");
    assert_eq!(path, Path::new("/p/albite.toml"));
    assert_eq!(
        note,
        Some(PlanNote::shadowed(Path::new("/p/tungsten.toml")))
    );

    assert_eq!(manifest_in(dir, only(&[])), None);
}

// ADR 20.10.26i AC1: the ancestor walk takes the old name at any level, and
// whatever stopped it before still stops it.
#[test]
fn find_manifest_takes_either_name_per_level() {
    let probe =
        |files: &'static [&'static str]| move |p: &Path| files.iter().any(|f| Path::new(f) == p);
    let found = find_manifest(Path::new("/app/sub/x.tg"), probe(&["/app/tungsten.toml"]));
    assert_eq!(found, Some(PathBuf::from("/app/tungsten.toml")));
    let found = find_manifest(
        Path::new("/app/x.tg"),
        probe(&["/app/albite.toml", "/app/tungsten.toml"]),
    );
    assert_eq!(found, Some(PathBuf::from("/app/albite.toml")));
    let found = find_manifest(
        Path::new("/outer/app/main.tg"),
        probe(&["/outer/app/main.tg", "/outer/tungsten.toml"]),
    );
    assert_eq!(found, None);
}

// ADR 20.10.26i AC2: a diamond's shared legacy manifest is noted once, and a
// new-named graph carries no notes at all.
#[test]
fn a_diamond_notes_its_legacy_manifest_once() {
    let files = [
        (
            "/ws/albite.toml",
            manifest_text("app", &[("a", "a"), ("b", "b")]),
        ),
        ("/ws/a/albite.toml", manifest_text("a", &[("c", "../c")])),
        ("/ws/a/lib.tg", String::new()),
        ("/ws/b/albite.toml", manifest_text("b", &[("c", "../c")])),
        ("/ws/b/lib.tg", String::new()),
        ("/ws/c/tungsten.toml", manifest_text("c", &[])),
        ("/ws/c/lib.tg", String::new()),
    ];
    let plan = plan_over(&files, "/ws/albite.toml");
    assert_eq!(plan.packages.len(), 3);
    assert_eq!(note_files(&plan), [Path::new("/ws/c/tungsten.toml")]);
    assert!(plan.notes[0].message.contains("rename it to `albite.toml`"));

    let mut renamed = files.clone();
    renamed[5].0 = "/ws/c/albite.toml";
    assert!(plan_over(&renamed, "/ws/albite.toml").notes.is_empty());
}

// ADR 20.10.26i AC2: the application's own manifest is noted, read under the
// old name or shadowing one, and a dependency's shadowed file too.
#[test]
fn the_application_and_a_shadowed_dependency_are_noted() {
    let app = manifest_text("app", &[("a", "a")]);
    let files = [
        ("/ws/tungsten.toml", app.clone()),
        ("/ws/a/albite.toml", manifest_text("a", &[])),
        ("/ws/a/tungsten.toml", String::from("stale")),
        ("/ws/a/lib.tg", String::new()),
    ];
    let plan = plan_over(&files, "/ws/tungsten.toml");
    assert_eq!(
        plan.notes,
        [
            PlanNote::legacy_name(Path::new("/ws/tungsten.toml")),
            PlanNote::shadowed(Path::new("/ws/a/tungsten.toml")),
        ]
    );

    let files = [
        ("/ws/albite.toml", manifest_text("app", &[])),
        ("/ws/tungsten.toml", app),
    ];
    let plan = plan_over(&files, "/ws/albite.toml");
    assert_eq!(note_files(&plan), [Path::new("/ws/tungsten.toml")]);
    assert!(plan.notes[0].message.contains("is ignored"));
}

// ADR 20.10.26i D1: the dependency error names both files.
#[test]
fn a_missing_manifest_names_both_files() {
    let files: HashMap<PathBuf, String> = [
        ("/ws/albite.toml", manifest_text("app", &[("a", "a")])),
        ("/ws/a/lib.tg", String::new()),
    ]
    .into_iter()
    .map(|(p, t)| (PathBuf::from(p), t))
    .collect();
    let probes = Probes {
        canonical: &|p| Ok(fold_dots(p)),
        exists: &|p| files.keys().any(|k| k == p || k.starts_with(p)),
        read: &|p| files.get(p).cloned(),
        reserved: TEST_RESERVED,
    };
    let app_path = Path::new("/ws/albite.toml");
    let app = parse_manifest(&files[app_path], app_path, TEST_RESERVED).expect("valid");
    let errors = plan_graph(&app, &HashSet::<String>::new(), &probes).expect_err("refused");
    assert!(
        errors[0]
            .message
            .contains("no `albite.toml` (or `tungsten.toml`) in `/ws/a`"),
        "{}",
        errors[0].message
    );
}
