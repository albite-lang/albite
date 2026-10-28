//! The pure halves of `use` resolution (ADR 29.9.26g D5): which candidate a
//! path lands on, whether it warns, and when `super` would leave a package —
//! and where a declared dependency's absolute candidate lands (3.10.26m D5).

use super::*;

fn path(segments: &[&str]) -> ModulePath {
    ModulePath::new(segments.iter().map(|s| (*s).to_string()).collect())
}

fn relative(module: &[&str], via: &'static str) -> Option<UseResolution> {
    Some(UseResolution {
        module: path(module),
        relative_via: Some(via),
    })
}

fn absolute(module: &[&str]) -> Option<UseResolution> {
    Some(UseResolution {
        module: path(module),
        relative_via: None,
    })
}

// 29.9.26g AC6: a library root's `use shapes::…` — child and package-rooted
// absolute are one path — resolves as absolute, with no warning.
#[test]
fn a_child_equal_to_the_absolute_path_does_not_warn() {
    let shapes = path(&["geometry", "shapes"]);
    let chosen = choose_use_candidate(
        (&shapes, true),
        Some((&path(&["shapes"]), false)),
        (&shapes, true),
    );
    assert_eq!(chosen, absolute(&["geometry", "shapes"]));
}

// A child that differs from the absolute path is a relative resolution.
#[test]
fn a_distinct_child_wins_and_warns() {
    let child = path(&["outer", "inner"]);
    let chosen = choose_use_candidate((&child, true), None, (&path(&["inner"]), true));
    assert_eq!(chosen, relative(&["outer", "inner"], "child"));
}

// 29.9.26g AC6: a library submodule's `use core::x` reaches `lib::core` as
// its sibling, which is also the package-rooted absolute path: no warning.
#[test]
fn a_sibling_equal_to_the_absolute_path_does_not_warn() {
    let core = path(&["lib_a", "core"]);
    let chosen = choose_use_candidate(
        (&path(&["lib_a", "api", "core"]), false),
        Some((&core, true)),
        (&core, true),
    );
    assert_eq!(chosen, absolute(&["lib_a", "core"]));
}

#[test]
fn a_distinct_sibling_wins_over_absolute_and_warns() {
    let sibling = path(&["a", "b"]);
    let chosen = choose_use_candidate(
        (&path(&["a", "c", "b"]), false),
        Some((&sibling, true)),
        (&path(&["b"]), true),
    );
    assert_eq!(chosen, relative(&["a", "b"], "sibling"));
}

// 29.9.26g AC6: an inadmissible sibling (another package's module) is
// skipped, so the package-rooted absolute path is tried and, here, missing.
#[test]
fn an_inadmissible_sibling_falls_through() {
    let chosen = choose_use_candidate(
        (&path(&["geometry", "app_util"]), false),
        Some((&path(&["app_util"]), false)),
        (&path(&["geometry", "app_util"]), false),
    );
    assert_eq!(chosen, None);
    let chosen = choose_use_candidate(
        (&path(&["x", "y"]), false),
        Some((&path(&["y"]), false)),
        (&path(&["y"]), true),
    );
    assert_eq!(chosen, absolute(&["y"]));
}

// 29.9.26g AC6: `super` leaves a package only from a dependency's root.
#[test]
fn super_escapes_only_from_a_dependency_root() {
    let geometry = path(&["geometry"]);
    let dependency = PackageId::Dependency(0);
    assert!(super_escapes_package(dependency, &geometry, &geometry));
    assert!(!super_escapes_package(
        dependency,
        &path(&["geometry", "shapes"]),
        &geometry
    ));
    assert!(!super_escapes_package(
        PackageId::Application,
        &ModulePath::root(),
        &ModulePath::root()
    ));
    assert!(!super_escapes_package(
        PackageId::Application,
        &path(&["a"]),
        &ModulePath::root()
    ));
}

/// `lib_a` and `lib_c` mounted at the root, `lib_a` declaring `lib_c`.
fn library_with_a_dependency() -> crate::elaborate::env::Env {
    let mut env = crate::elaborate::env::Env::new();
    let lib_a = env.packages.add_package("lib_a", path(&["lib_a"]));
    let lib_c = env.packages.add_package("lib_c", path(&["lib_c"]));
    env.packages.declare(PackageId::Application, lib_a);
    env.packages.declare(PackageId::Dependency(lib_a), lib_c);
    env
}

// 3.10.26m AC5 (D5): a first segment naming a declared dependency makes the
// absolute candidate root-level, where the dependency is mounted; from a
// library's root its sibling is the same path, so it warns about nothing.
#[test]
fn a_declared_dependency_is_the_absolute_candidate() {
    let env = library_with_a_dependency();
    let raw = path(&["lib_c"]);
    let from_root = env.absolute_candidate(&path(&["lib_a"]), &raw);
    assert_eq!(from_root, path(&["lib_c"]));
    let chosen = choose_use_candidate(
        (&path(&["lib_a", "lib_c"]), false),
        Some((&path(&["lib_c"]), true)),
        (&from_root, true),
    );
    assert_eq!(chosen, absolute(&["lib_c"]));
    // From a submodule there is no sibling to find it: only the candidate does.
    let nested = env.absolute_candidate(&path(&["lib_a", "shapes"]), &raw);
    assert_eq!(nested, path(&["lib_c"]));
    // Any other first segment stays rooted at the library.
    let own = env.absolute_candidate(&path(&["lib_a", "shapes"]), &path(&["util"]));
    assert_eq!(own, path(&["lib_a", "util"]));
    // The application did not declare `lib_c`, so its candidate is its own root's.
    assert_eq!(
        env.absolute_candidate(&ModulePath::root(), &raw),
        path(&["lib_c"])
    );
    assert!(!env.package_may_reach(&ModulePath::root(), &path(&["lib_c"])));
}
