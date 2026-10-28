//! Local path dependencies through the binary (ADR 29.9.26g AC 4, 6, 7).
//!
//! What only a spawned process shows: that `prepare_project` finds the
//! manifest, mounts the library and runs it; that a library checks on its own
//! from the same sources; which file a manifest error names; and that the
//! package-rooted `use` warns about nothing. Fixtures live in a tempdir and
//! every run is `--no-cache`.

use std::path::{Path, PathBuf};
use std::process::Command;

const APP_MANIFEST: &str = "\
[package]
name = \"app\"

[dependencies]
geometry = { path = \"packages/geometry\" }
";
const GEOMETRY_MANIFEST: &str = "[package]\nname = \"geometry\"\n";
const GEOMETRY_LIB: &str = "\
pub mod shapes;
use shapes::{Shape, Square};
fn side(s: Shape) -> Nat { match s { Square(n) => n } }
pub fn perimeter(s: Shape) -> Nat { side(s) + side(s) + side(s) + side(s) }
";
const GEOMETRY_SHAPES: &str = "pub type Shape = Square(Nat)\n";
const APP_MAIN: &str = "\
use geometry::perimeter;
use geometry::shapes::Square;
fn main() -> Nat { perimeter(Square(5)) }
";

struct Run {
    code: i32,
    output: String,
}

fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create tempdir");
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write fixture");
    }
    dir
}

fn tungsten(cmd: &str, entry: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_albite"))
        .args([cmd, &entry.to_string_lossy(), "--no-cache"])
        .env("NO_COLOR", "1")
        .output()
        .expect("spawn tungsten");
    Run {
        code: output.status.code().unwrap_or(-1),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ),
    }
}

fn library_project() -> (tempfile::TempDir, PathBuf) {
    let dir = fixture(&[
        ("albite.toml", APP_MANIFEST),
        ("main.tg", APP_MAIN),
        ("packages/geometry/albite.toml", GEOMETRY_MANIFEST),
        ("packages/geometry/lib.tg", GEOMETRY_LIB),
        ("packages/geometry/shapes.tg", GEOMETRY_SHAPES),
    ]);
    let entry = dir.path().join("main.tg");
    (dir, entry)
}

// 29.9.26g AC4 and AC6: the application runs against its mounted library, and
// the library's own `use shapes::…` is package-rooted, so nothing warns.
#[test]
fn application_runs_with_a_path_dependency() {
    let (_dir, entry) = library_project();
    let run = tungsten("run", &entry);
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(run.output.trim_end().ends_with("20"), "{}", run.output);
    assert!(!run.output.contains("relative import"), "{}", run.output);
}

// 29.9.26g AC4: the library checks standalone from the same sources, reading
// its own manifest (the application's names a dependency this entry lacks).
#[test]
fn library_checks_on_its_own() {
    let (dir, _) = library_project();
    let lib = dir.path().join("packages/geometry/lib.tg");
    let run = tungsten("check", &lib);
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(!run.output.contains("relative import"), "{}", run.output);
}

// 29.9.26g AC6: a library cannot name an application module.
#[test]
fn library_cannot_reach_its_dependent() {
    let (dir, entry) = library_project();
    std::fs::write(dir.path().join("util.tg"), "pub fn u() -> Nat { 1 }\n").expect("write");
    std::fs::write(&entry, format!("mod util;\n{APP_MAIN}")).expect("write");
    let lib = dir.path().join("packages/geometry/lib.tg");
    std::fs::write(&lib, format!("use util::u;\n{GEOMETRY_LIB}")).expect("write");
    let run = tungsten("check", &entry);
    assert_eq!(run.code, 1, "{}", run.output);
    assert!(run.output.contains("[E0005]"), "{}", run.output);
}

// 29.9.26g AC7: a manifest error is rendered against `albite.toml`, with a
// line and column, through the ordinary exit-1 path.
#[test]
fn manifest_errors_name_the_manifest() {
    let (dir, entry) = library_project();
    std::fs::write(
        dir.path().join("albite.toml"),
        "[package]\nname = \"app\"\nedition = 1\n",
    )
    .expect("write");
    let run = tungsten("check", &entry);
    assert_eq!(run.code, 1, "{}", run.output);
    assert!(run.output.contains("[E0120]"), "{}", run.output);
    assert!(run.output.contains("albite.toml:3:1"), "{}", run.output);
}

const LIB_A_MANIFEST: &str =
    "[package]\nname = \"lib_a\"\n\n[dependencies]\nlib_c = { path = \"../lib_c\" }\n";

/// The diamond `app → {lib_a, lib_b} → lib_c` (ADR 3.10.26m §2.1). `lib_a`
/// names `lib_c` only inside a body, by qualified path: Signature Collection,
/// which runs warm too, never resolves it, so only the cache key can see a
/// dropped declaration (D12).
fn diamond_project() -> (tempfile::TempDir, PathBuf) {
    let lib_b_manifest =
        "[package]\nname = \"lib_b\"\n\n[dependencies]\nlib_c = { path = \"../lib_c\" }\n";
    let dir = fixture(&[
        (
            "albite.toml",
            "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"packages/lib_a\" }\nlib_b = { path = \"packages/lib_b\" }\n",
        ),
        ("main.tg", "use lib_a::a;\nuse lib_b::b;\nfn main() -> Nat { a() + b() }\n"),
        ("packages/lib_a/albite.toml", LIB_A_MANIFEST),
        ("packages/lib_a/lib.tg", "pub fn a() -> Nat { lib_c::base() + 1 }\n"),
        ("packages/lib_b/albite.toml", lib_b_manifest),
        ("packages/lib_b/lib.tg", "use lib_c::base;\npub fn b() -> Nat { base() + 2 }\n"),
        ("packages/lib_c/albite.toml", "[package]\nname = \"lib_c\"\n"),
        ("packages/lib_c/lib.tg", "pub fn base() -> Nat { 10 }\n"),
    ]);
    let entry = dir.path().join("main.tg");
    (dir, entry)
}

// 3.10.26m AC4: transitive dependencies run, and a library with dependencies
// checks on its own, reading its manifest and mounting its dependency.
#[test]
fn transitive_dependencies_run_and_a_library_checks_alone() {
    let (dir, entry) = diamond_project();
    let run = tungsten("run", &entry);
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(run.output.trim_end().ends_with("23"), "{}", run.output);
    let lib_a = dir.path().join("packages/lib_a/lib.tg");
    let run = tungsten("check", &lib_a);
    assert_eq!(run.code, 0, "{}", run.output);
}

// 3.10.26m AC7: the cycle error renders through the ordinary path, naming
// the manifest that closes it with a line and column.
#[test]
fn a_cycle_names_the_manifest_that_closes_it() {
    let (dir, entry) = diamond_project();
    let lib_c_manifest =
        "[package]\nname = \"lib_c\"\n\n[dependencies]\nlib_a = { path = \"../lib_a\" }\n";
    std::fs::write(
        dir.path().join("packages/lib_c/albite.toml"),
        lib_c_manifest,
    )
    .expect("write");
    let run = tungsten("check", &entry);
    assert_eq!(run.code, 1, "{}", run.output);
    assert!(run.output.contains("[E0124]"), "{}", run.output);
    assert!(
        run.output
            .contains("dependency cycle: app → lib_a → lib_c → lib_a"),
        "{}",
        run.output
    );
    assert!(
        run.output.contains("lib_c/albite.toml:5:1"),
        "{}",
        run.output
    );
}

/// `check` with the elaboration cache on, both tiers, and `--verbose`.
fn warm_check(entry: &Path) -> Run {
    warm("check", entry)
}

/// `cmd` with the elaboration cache on, both tiers, and `--verbose`.
fn warm(cmd: &str, entry: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_albite"))
        .args([cmd, &entry.to_string_lossy(), "--verbose"])
        .env("NO_COLOR", "1")
        .env("ALBITE_ELAB_CACHE_FULL", "1")
        .output()
        .expect("spawn tungsten");
    Run {
        code: output.status.code().unwrap_or(-1),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ),
    }
}

// 3.10.26m AC10 (D12): drop `lib_c` from `lib_a`'s manifest without touching
// a `.tg` file — `lib_c` stays mounted through `lib_b`, so the exports hash
// is unchanged — and a warm run must not replay `lib_a`'s cached success.
#[test]
fn a_warm_cache_cannot_replay_what_reach_forbids() {
    let (dir, entry) = diamond_project();
    let cold = warm_check(&entry);
    assert_eq!(cold.code, 0, "{}", cold.output);
    let warm = warm_check(&entry);
    assert_eq!(warm.code, 0, "{}", warm.output);
    let lib_a_hit = |run: &Run| {
        run.output
            .lines()
            .any(|l| l.contains("[elab-cache-full] hit for") && l.contains("lib_a"))
    };
    assert!(lib_a_hit(&warm), "the control must hit: {}", warm.output);
    std::fs::write(
        dir.path().join("packages/lib_a/albite.toml"),
        "[package]\nname = \"lib_a\"\n",
    )
    .expect("write");
    let dropped = warm_check(&entry);
    assert_eq!(dropped.code, 1, "{}", dropped.output);
    assert!(dropped.output.contains("[E0005]"), "{}", dropped.output);
    assert!(dropped.output.contains("lib_c"), "{}", dropped.output);
    assert!(!lib_a_hit(&dropped), "{}", dropped.output);
}

// 4.10.26j AC6 (D7): `z` imports `x::helper` and never changes. A module
// added later that also defines `helper` scopes `x`'s Core name, so a warm
// run must re-elaborate `z` rather than replay a cached `helper` reference.
#[test]
fn a_warm_cache_cannot_replay_a_name_a_new_clash_scoped() {
    let dir = fixture(&[
        ("main.tg", "mod x;\nmod z;\nfn main() -> Nat { z::run() }\n"),
        ("x.tg", "pub fn helper(n: Nat) -> Nat { n + 10 }\n"),
        (
            "z.tg",
            "use x::helper;\npub fn run() -> Nat { helper(1) }\n",
        ),
    ]);
    let entry = dir.path().join("main.tg");
    let z_hit = |run: &Run| {
        run.output
            .lines()
            .any(|l| l.contains("[elab-cache-full] hit for") && l.contains("z.tg"))
    };
    assert_eq!(warm_check(&entry).code, 0);
    let control = warm_check(&entry);
    assert!(z_hit(&control), "the control must hit: {}", control.output);

    std::fs::write(
        dir.path().join("y.tg"),
        "pub fn helper(n: Nat) -> Nat { n + 100 }\n",
    )
    .expect("write");
    std::fs::write(
        &entry,
        "mod x;\nmod y;\nmod z;\nfn main() -> Nat { z::run() + y::helper(1) }\n",
    )
    .expect("write");
    let clashed = warm_check(&entry);
    assert_eq!(clashed.code, 0, "{}", clashed.output);
    assert!(!z_hit(&clashed), "{}", clashed.output);
    let run = warm("run", &entry);
    assert_eq!(run.code, 0, "{}", run.output);
    assert!(run.output.contains("112 : Nat"), "{}", run.output);
}

// 29.9.26g AC8: without a manifest, the same tree is not mounted — the
// library's name resolves to nothing.
#[test]
fn no_manifest_mounts_nothing() {
    let (dir, entry) = library_project();
    std::fs::remove_file(dir.path().join("albite.toml")).expect("remove");
    let run = tungsten("check", &entry);
    assert_eq!(run.code, 1, "{}", run.output);
    assert!(run.output.contains("geometry"), "{}", run.output);
}
