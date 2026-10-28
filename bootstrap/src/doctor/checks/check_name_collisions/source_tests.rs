//! Tests: the parse-side half — the `extern "C"` walk and the command's two
//! outcomes.
//!
//! These write a real module tree to a temporary directory, because the two
//! things they assert cannot be reached from a hand-built `ModuleInfo`:
//! `ModuleContents` records visibility but *not* extern-ness, so the extern
//! walk has no seam short of an AST; and the parse failure that produces
//! `Outcome::Unparsable` needs a file to fail on.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::census::{CollisionClass, ReexportHandling, Severity};
use super::{census_of_file, census_of_tree, run, Outcome};
use crate::driver::modules::parse_module_tree;

/// Write a module tree and return its directory and entry file.
fn tree(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    for (name, source) in files {
        std::fs::write(dir.path().join(name), source).expect("write fixture");
    }
    let entry = dir.path().join(files[0].0);
    (dir, entry)
}

/// Census a written tree the way the command does.
fn census_of(entry: &Path) -> super::census::Census {
    let mut visited = std::collections::HashSet::new();
    let mut chain = Vec::new();
    let parsed = parse_module_tree(entry, &mut visited, &mut chain, None).expect("parses");
    census_of_tree(&parsed, ReexportHandling::Subtract)
}

/// D3 class (c): two `extern "C" fn` of one name. The Tungsten identifier IS
/// the C symbol (ADR 7.8.26b D3), so this is a duplicate *link* symbol as well
/// as a shadowed binding, and no visibility change fixes it.
///
/// This is the only test of the extern walk, and it is a real one: extern-ness
/// comes from the AST, so a walk that returned nothing would silently demote
/// this finding to `scoped` — information where a rename is owed.
#[test]
fn two_extern_c_definitions_of_one_name_are_the_link_class() {
    let (_dir, entry) = tree(&[
        ("main.tg", "mod alpha;\nmod beta;\nfn main() -> Nat { 0 }\n"),
        (
            "alpha.tg",
            "pub extern \"C\" fn tg_report(n: Nat) -> Nat;\n",
        ),
        ("beta.tg", "pub extern \"C\" fn tg_report(n: Nat) -> Nat;\n"),
    ]);

    let census = census_of(&entry);

    let finding = census
        .collisions
        .iter()
        .find(|c| c.name == "tg_report")
        .expect("the extern pair is a finding");
    assert_eq!(finding.class, CollisionClass::ExternSymbol);
    assert!(
        finding.sites.iter().all(|site| site.is_extern_c),
        "both sites are extern: {:?}",
        finding.sites
    );
}

/// The same two definitions as plain `fn`s are NOT the link class — which is
/// what makes the test above an assertion about the extern walk rather than
/// about the collision.
#[test]
fn the_same_pair_as_plain_fns_is_not_the_link_class() {
    let (_dir, entry) = tree(&[
        ("main.tg", "mod alpha;\nmod beta;\nfn main() -> Nat { 0 }\n"),
        ("alpha.tg", "pub fn tg_report(n: Nat) -> Nat { n }\n"),
        ("beta.tg", "pub fn tg_report(n: Nat) -> Nat { n }\n"),
    ]);

    let finding = census_of(&entry)
        .collisions
        .into_iter()
        .find(|c| c.name == "tg_report")
        .expect("still a collision");
    assert_ne!(finding.class, CollisionClass::ExternSymbol);
    assert!(finding.sites.iter().all(|site| !site.is_extern_c));
}

/// `--json` reaches the JSON renderer through `run`. The renderer is unit
/// tested directly, but nothing else drives the flag from the command's own
/// entry point — and the flag is D3's whole gating story: "a caller wanting a
/// gate reads `--json`".
#[test]
fn the_json_flag_reaches_the_json_renderer() {
    let (_dir, entry) = tree(&[
        ("main.tg", "mod alpha;\nmod beta;\nfn main() -> Nat { 0 }\n"),
        ("alpha.tg", "fn shared() -> Nat { 1 }\n"),
        ("beta.tg", "fn shared() -> Nat { 2 }\n"),
    ]);

    let Outcome::Reported(human) = run(&entry, Severity::All, false, false) else {
        panic!("a parsable tree reports");
    };
    let Outcome::Reported(json) = run(&entry, Severity::All, true, false) else {
        panic!("a parsable tree reports");
    };
    assert_ne!(human, json, "the flag selects a different renderer");

    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["collision_count"], 1);
    assert_eq!(parsed["collisions"][0]["name"], "shared");
    assert!(
        serde_json::from_str::<serde_json::Value>(&human).is_err(),
        "and the default is emphatically not JSON"
    );
}

/// `--include-reexports` reaches the measurement arm through `run` too, so the
/// two runs AC 1 compares are genuinely two runs.
#[test]
fn the_include_reexports_flag_reaches_the_measurement_arm() {
    let (_dir, entry) = tree(&[
        ("main.tg", "mod alpha;\nmod beta;\nfn main() -> Nat { 0 }\n"),
        ("alpha.tg", "pub fn shared() -> Nat { 1 }\n"),
        ("beta.tg", "pub use alpha::shared;\n"),
    ]);

    let subtracted = run(&entry, Severity::All, true, false);
    let kept = run(&entry, Severity::All, true, true);
    assert_ne!(
        subtracted, kept,
        "the re-exported copy is subtracted by default and kept by the flag"
    );
}

/// A tree that parses reports, findings or not — the check is advisory (D3).
#[test]
fn a_parsable_tree_reports() {
    let (_dir, entry) = tree(&[("main.tg", "fn main() -> Nat { 0 }\n")]);

    let Outcome::Reported(report) = run(&entry, Severity::All, false, false) else {
        panic!("a parsable tree reports");
    };
    assert!(report.contains("examined 1 module(s)"));
}

/// A file that cannot be parsed at all is **bad input**, not a clean run — the
/// one thing this command exits non-zero for.
#[test]
fn an_unparsable_entry_file_is_bad_input_not_a_clean_run() {
    let outcome = run(
        Path::new("/nonexistent/definitely-not-here.tg"),
        Severity::All,
        false,
        false,
    );
    let Outcome::Unparsable(message) = &outcome else {
        panic!("expected bad input, got {outcome:?}");
    };
    assert!(message.starts_with("error: "), "{message}");
    assert_ne!(
        outcome,
        run(
            &tree(&[("main.tg", "fn main() -> Nat { 0 }\n")]).1,
            Severity::All,
            false,
            false
        ),
        "bad input and a clean run are different outcomes"
    );
}

/// Write a project whose files sit at relative paths, creating directories.
fn project(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    for (name, source) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("create dirs");
        std::fs::write(path, source).expect("write fixture");
    }
    let entry = dir.path().join(files[0].0);
    (dir, entry)
}

/// An application declaring `lib_a` and `lib_b`, each with a private `helper`.
const APP_MANIFEST: &str = "[package]\nname = \"app\"\n\n[dependencies]\n\
    lib_a = { path = \"packages/lib_a\" }\nlib_b = { path = \"packages/lib_b\" }\n";

/// A package manifest naming itself and nothing else.
fn package_manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\n")
}

/// The human report, which every package test reads.
fn human_report(entry: &Path) -> Outcome {
    run(entry, Severity::All, false, false)
}

/// ADR 15.10.26n AC1: a repeat inside two packages is reported, and the
/// summary line says how many packages were read. The must-fail half is
/// `census_of`, the parse-only path `run` took before the ADR: it does not
/// see the packages, so it does not see `helper`.
#[test]
fn a_repeat_inside_two_packages_is_reported() {
    let lib_a_manifest = package_manifest("lib_a");
    let lib_b_manifest = package_manifest("lib_b");
    let (_dir, entry) = project(&[
        (
            "main.tg",
            "use lib_a::f;\nuse lib_b::g;\nfn main() -> Nat { f() + g() }\n",
        ),
        ("albite.toml", APP_MANIFEST),
        ("packages/lib_a/albite.toml", &lib_a_manifest),
        (
            "packages/lib_a/lib.tg",
            "fn helper() -> Nat { 1 }\npub fn f() -> Nat { helper() }\n",
        ),
        ("packages/lib_b/albite.toml", &lib_b_manifest),
        (
            "packages/lib_b/lib.tg",
            "fn helper() -> Nat { 2 }\npub fn g() -> Nat { helper() }\n",
        ),
    ]);

    let Outcome::Reported(report) = human_report(&entry) else {
        panic!("a mountable project reports");
    };
    assert!(report.contains("\n  `helper` [scoped]\n"), "{report}");
    assert!(report.contains("    lib_a  (private)"), "{report}");
    assert!(report.contains("    lib_b  (private)"), "{report}");
    assert!(
        report.contains(
            "  examined 3 module(s) across 2 package(s), 4 distinct value name(s) \
             across 5 definition site(s)\n"
        ),
        "{report}"
    );

    let unmounted = census_of(&entry);
    assert!(
        unmounted.collisions.iter().all(|c| c.name != "helper"),
        "the pre-ADR parse-only census cannot see the packages"
    );
    assert_eq!(unmounted.packages_mounted, 0);
}

/// ADR 15.10.26n AC2: with no manifest the output is byte-identical to the
/// pre-change render — the old line, spelled out, with no package clause.
#[test]
fn without_a_manifest_the_report_is_unchanged() {
    let (_dir, entry) = tree(&[
        ("main.tg", "mod alpha;\nmod beta;\nfn main() -> Nat { 0 }\n"),
        ("alpha.tg", "fn shared() -> Nat { 1 }\n"),
        ("beta.tg", "fn shared() -> Nat { 2 }\n"),
    ]);

    let Outcome::Reported(report) = human_report(&entry) else {
        panic!("a parsable tree reports");
    };
    assert_eq!(
        report,
        "! 1 name collision(s) (all classes):\n\
         \n  `shared` [scoped]\n\
         \x20   alpha  (private)   as `_tg_5alpha_6shared`\n\
         \x20   beta  (private)   as `_tg_4beta_6shared`\n\
         \x20   → information: each definition has its own Core name; a qualified path \
         names one, and a bare call reaching two is E0019\n\
         \x20 examined 3 module(s), 2 distinct value name(s) across 3 definition site(s)\n\
         \x20 advisory: exit is 0 even with findings — read `--json` to gate on them \
         (ADR 13.8.26c D3)\n"
    );
}

/// Assert that `entry` is bad input and that no census was rendered for it.
fn assert_bad_input_without_census(entry: &Path) {
    let outcome = human_report(entry);
    let Outcome::Unparsable(message) = &outcome else {
        panic!("expected bad input, got {outcome:?}");
    };
    assert!(message.contains("albite check"), "{message}");
    assert!(!message.contains("examined"), "no census text: {message}");
    assert_eq!(census_of_file(entry, ReexportHandling::Subtract), None);
}

/// ADR 15.10.26n AC3: a manifest that does not parse is bad input.
#[test]
fn a_malformed_manifest_is_bad_input() {
    let (_dir, entry) = project(&[
        ("main.tg", "fn main() -> Nat { 0 }\n"),
        ("albite.toml", "[package\nname = \n"),
    ]);
    assert_bad_input_without_census(&entry);
}

/// ADR 15.10.26n AC3: a dependency whose path does not exist is bad input.
#[test]
fn a_missing_dependency_is_bad_input() {
    let (_dir, entry) = project(&[
        ("main.tg", "fn main() -> Nat { 0 }\n"),
        (
            "albite.toml",
            "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"packages/lib_a\" }\n",
        ),
    ]);
    assert_bad_input_without_census(&entry);
}

/// ADR 15.10.26n AC3: two packages depending on each other is bad input.
#[test]
fn a_dependency_cycle_is_bad_input() {
    let (_dir, entry) = project(&[
        ("main.tg", "use lib_a::f;\nfn main() -> Nat { f() }\n"),
        (
            "albite.toml",
            "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"packages/lib_a\" }\n",
        ),
        (
            "packages/lib_a/albite.toml",
            "[package]\nname = \"lib_a\"\n\n[dependencies]\nlib_b = { path = \"../lib_b\" }\n",
        ),
        ("packages/lib_a/lib.tg", "pub fn f() -> Nat { 1 }\n"),
        (
            "packages/lib_b/albite.toml",
            "[package]\nname = \"lib_b\"\n\n[dependencies]\nlib_a = { path = \"../lib_a\" }\n",
        ),
        ("packages/lib_b/lib.tg", "pub fn g() -> Nat { 2 }\n"),
    ]);
    assert_bad_input_without_census(&entry);
}

/// ADR 15.10.26n AC3: a mounted package whose source does not parse is bad
/// input, though the application's own tree parses.
#[test]
fn an_unparsable_package_source_is_bad_input() {
    let lib_a_manifest = package_manifest("lib_a");
    let (_dir, entry) = project(&[
        ("main.tg", "use lib_a::f;\nfn main() -> Nat { f() }\n"),
        (
            "albite.toml",
            "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"packages/lib_a\" }\n",
        ),
        ("packages/lib_a/albite.toml", &lib_a_manifest),
        ("packages/lib_a/lib.tg", "pub fn f( -> Nat {\n"),
    ]);
    assert_bad_input_without_census(&entry);
}

/// ADR 15.10.26n D3: only a *package's* parse errors refuse the census. The
/// application's own stay tolerated on a manifest project, as without one —
/// the check exists for files the compiler rejects.
#[test]
fn an_unparsable_application_module_still_reports_beside_packages() {
    let lib_a_manifest = package_manifest("lib_a");
    let (_dir, entry) = project(&[
        (
            "main.tg",
            "mod broken;\nuse lib_a::f;\nfn main() -> Nat { f() }\n",
        ),
        (
            "albite.toml",
            "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"packages/lib_a\" }\n",
        ),
        ("broken.tg", "pub fn g( -> Nat {\n"),
        ("packages/lib_a/albite.toml", &lib_a_manifest),
        ("packages/lib_a/lib.tg", "pub fn f() -> Nat { 1 }\n"),
    ]);

    let Outcome::Reported(report) = human_report(&entry) else {
        panic!("the application's own parse errors do not refuse the census");
    };
    assert!(report.contains("across 1 package(s)"), "{report}");
}

/// ADR 15.10.26n AC5: a declared package the entry never imports. K and the
/// module count follow what `mount_packages` mounted (D6).
#[test]
fn an_unused_declaration_follows_the_planner() {
    let lib_a_manifest = package_manifest("lib_a");
    let lib_b_manifest = package_manifest("lib_b");
    let (_dir, entry) = project(&[
        ("main.tg", "use lib_a::f;\nfn main() -> Nat { f() }\n"),
        ("albite.toml", APP_MANIFEST),
        ("packages/lib_a/albite.toml", &lib_a_manifest),
        ("packages/lib_a/lib.tg", "pub fn f() -> Nat { 1 }\n"),
        ("packages/lib_b/albite.toml", &lib_b_manifest),
        ("packages/lib_b/lib.tg", "pub fn g() -> Nat { 2 }\n"),
    ]);

    let census = census_of_file(&entry, ReexportHandling::Subtract).expect("mounts");
    // The planner mounts every declared package, imported or not: `lib_b` is
    // mounted and counted although `main.tg` never names it.
    assert_eq!(census.packages_mounted, 2);
    assert_eq!(census.modules_examined, 3);
}
