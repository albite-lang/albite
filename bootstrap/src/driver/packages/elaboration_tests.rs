//! Mounted packages through the real tree elaboration (ADR 29.9.26g AC 4–6):
//! `prepare_project` then `elaborate_module_tree` over on-disk fixtures in a
//! tempdir — the same two calls `check` makes, minus rendering.

use std::path::Path;

use crate::driver::output::TraceOptions;
use crate::driver::per_module::elaborate_module_tree;
use crate::driver::{pipeline, prepare_project, PipelineError};
use crate::elaborate::ElabError;

enum Outcome {
    Elaborated,
    Refused(Vec<ElabError>),
    Unprepared(PipelineError),
}

fn codes(outcome: &Outcome) -> Vec<&'static str> {
    match outcome {
        Outcome::Refused(errors) => errors.iter().map(|e| e.kind.code()).collect(),
        Outcome::Elaborated => vec![],
        Outcome::Unprepared(error) => panic!("did not prepare: {error:?}"),
    }
}

/// Write `files` (the first is the entry) under a fresh tempdir and elaborate.
fn elaborate(files: &[(&str, &str)]) -> Outcome {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }
    let entry = dir.path().join(files[0].0);
    elaborate_entry(&entry)
}

fn elaborate_entry(entry: &Path) -> Outcome {
    let prepared = match prepare_project(entry, false, None) {
        Ok(prepared) => prepared,
        Err(error) => return Outcome::Unprepared(error),
    };
    let build = pipeline::BuildCtx {
        cache: None,
        module_info: prepared.module_info,
        source_map: prepared.source_map,
    };
    let trace = TraceOptions::default();
    match elaborate_module_tree(&prepared.module_tree, entry, false, &build, &trace) {
        Ok(_) => Outcome::Elaborated,
        Err(errors) => Outcome::Refused(errors),
    }
}

const PRIVATE_MODULE: [(&str, &str); 2] = [
    ("a/mod.tg", "mod hidden;\npub fn visible() -> Nat { 1 }\n"),
    ("a/hidden.tg", "pub fn f() -> Nat { 2 }\n"),
];

// The E0009 arm of an aliased `use` (its `!` was on a line 29.9.26g D6 edited).
#[test]
fn an_aliased_use_of_a_private_module_is_refused() {
    let main = (
        "main.tg",
        "mod a;\nuse a::hidden::f as g;\nfn main() -> Nat { g() }\n",
    );
    let outcome = elaborate(&[main, PRIVATE_MODULE[0], PRIVATE_MODULE[1]]);
    assert!(codes(&outcome).contains(&"E0009"), "{:?}", codes(&outcome));
    let public = (
        "main.tg",
        "mod a;\nuse a::visible as g;\nfn main() -> Nat { g() }\n",
    );
    let outcome = elaborate(&[public, PRIVATE_MODULE[0], PRIVATE_MODULE[1]]);
    assert!(
        matches!(outcome, Outcome::Elaborated),
        "{:?}",
        codes(&outcome)
    );
}

// The E0009 arm of a glob `use` (same provenance).
#[test]
fn a_glob_use_of_a_private_module_is_refused() {
    let main = (
        "main.tg",
        "mod a;\nuse a::hidden::*;\nfn main() -> Nat { f() }\n",
    );
    let outcome = elaborate(&[main, PRIVATE_MODULE[0], PRIVATE_MODULE[1]]);
    assert!(codes(&outcome).contains(&"E0009"), "{:?}", codes(&outcome));
}

const APP_MANIFEST: &str =
    "[package]\nname = \"app\"\n\n[dependencies]\ngeometry = { path = \"packages/geometry\" }\n";
const GEOMETRY_MANIFEST: (&str, &str) = (
    "packages/geometry/albite.toml",
    "[package]\nname = \"geometry\"\n",
);
const GEOMETRY_LIB: (&str, &str) = (
    "packages/geometry/lib.tg",
    "pub mod shapes;\nuse shapes::side;\nfn hidden() -> Nat { 1 }\npub fn area() -> Nat { side() + hidden() }\n",
);
const GEOMETRY_SHAPES: (&str, &str) = (
    "packages/geometry/shapes.tg",
    "pub fn side() -> Nat { 3 }\n",
);

fn with_geometry(main: &str) -> Outcome {
    elaborate(&[
        ("main.tg", main),
        ("albite.toml", APP_MANIFEST),
        GEOMETRY_MANIFEST,
        GEOMETRY_LIB,
        GEOMETRY_SHAPES,
    ])
}

// 29.9.26g AC4: a mounted library elaborates, and its `pub` items are reachable.
#[test]
fn a_mounted_library_elaborates() {
    let outcome = with_geometry("use geometry::area;\nfn main() -> Nat { area() }\n");
    assert!(
        matches!(outcome, Outcome::Elaborated),
        "{:?}",
        codes(&outcome)
    );
}

// 29.9.26g AC5: a library's private item is E0016 from the application.
#[test]
fn a_library_private_item_is_refused() {
    let outcome = with_geometry("fn main() -> Nat { geometry::hidden() }\n");
    assert_eq!(codes(&outcome), ["E0016"]);
}

// 29.9.26g AC3: a manifest refusal stops preparation, before elaboration.
#[test]
fn a_manifest_refusal_stops_preparation() {
    let outcome = elaborate(&[
        ("main.tg", "fn main() -> Nat { 0 }\n"),
        ("albite.toml", "[package]\nname = \"my-app\"\n"),
    ]);
    assert!(matches!(
        outcome,
        Outcome::Unprepared(PipelineError::ElabFailed(_))
    ));
}

/// `app → lib_a → lib_c`, plus `lib_b`, which the application declares and
/// `lib_a` does not. `lib_a/lib.tg` is `lib_a`; the entry is `main`.
fn with_transitive(main: &str, lib_a: &str) -> Outcome {
    let app = "[package]\nname = \"app\"\n\n[dependencies]\nlib_a = { path = \"packages/lib_a\" }\nlib_b = { path = \"packages/lib_b\" }\n";
    let lib_a_manifest =
        "[package]\nname = \"lib_a\"\n\n[dependencies]\nlib_c = { path = \"../lib_c\" }\n";
    elaborate(&[
        ("main.tg", main),
        ("albite.toml", app),
        ("packages/lib_a/albite.toml", lib_a_manifest),
        ("packages/lib_a/lib.tg", lib_a),
        (
            "packages/lib_b/albite.toml",
            "[package]\nname = \"lib_b\"\n",
        ),
        (
            "packages/lib_b/lib.tg",
            "pub fn g() -> Nat { 41 }\npub type Colour = Red | Blue\npub type Boxed = Boxed(Nat)\n",
        ),
        (
            "packages/lib_c/albite.toml",
            "[package]\nname = \"lib_c\"\n",
        ),
        (
            "packages/lib_c/lib.tg",
            "pub fn double(n: Nat) -> Nat { n + n }\npub fn base() -> Nat { 10 }\n",
        ),
    ])
}

const USES_A: &str = "use lib_a::f;\nfn main() -> Nat { f() }\n";

// 3.10.26m AC5: a library reaches its declared dependency by `use` and by
// qualified path, and re-exports one of its values.
#[test]
fn a_library_reaches_its_declared_dependency() {
    let lib_a =
        "use lib_c::base;\npub use lib_c::double;\npub fn f() -> Nat { base() + lib_c::base() }\n";
    let main = "use lib_a::f;\nuse lib_a::double;\nfn main() -> Nat { double(f()) }\n";
    let outcome = with_transitive(main, lib_a);
    assert!(
        matches!(outcome, Outcome::Elaborated),
        "{:?}",
        codes(&outcome)
    );
}

// 3.10.26m AC5: the application cannot name a package it did not declare,
// by `use` or by qualified path, although it is mounted.
#[test]
fn the_application_is_refused_an_undeclared_package() {
    let lib_a = "use lib_c::base;\npub fn f() -> Nat { base() }\n";
    let by_use = with_transitive("use lib_c::base;\nfn main() -> Nat { base() }\n", lib_a);
    assert!(codes(&by_use).contains(&"E0005"), "{:?}", codes(&by_use));
    let by_path = with_transitive("fn main() -> Nat { lib_c::base() }\n", lib_a);
    assert_eq!(codes(&by_path), ["E0005"]);
    // A library cannot name its dependent's sibling either.
    let sideways = with_transitive(USES_A, "use lib_b::g;\npub fn f() -> Nat { g() }\n");
    assert!(
        codes(&sideways).contains(&"E0005"),
        "{:?}",
        codes(&sideways)
    );
}

/// The single refusal of `outcome`, which must be E0001.
fn not_found(outcome: &Outcome) -> &ElabError {
    let Outcome::Refused(errors) = outcome else {
        panic!("expected a refusal: {:?}", codes(outcome));
    };
    assert_eq!(codes(outcome), ["E0001"]);
    &errors[0]
}

// 3.10.26m AC6: a bare name defined in an undeclared package does not exist
// to the caller — E0001, with a help line naming the package and the
// missing declaration; a constructor likewise.
#[test]
fn a_bare_name_does_not_cross_into_an_undeclared_package() {
    let outcome = with_transitive(USES_A, "pub fn f() -> Nat { g() + 1 }\n");
    let error = not_found(&outcome);
    assert!(
        matches!(&error.kind, crate::elaborate::ElabErrorKind::UndefinedVariable(name) if name == "g"),
        "{:?}",
        error.kind
    );
    let help = error.help.as_deref().unwrap_or_default();
    assert_eq!(
        help,
        "`g` is defined in package `lib_b`, which `lib_a` does not declare in `[dependencies]`"
    );
    // Each constructor shape is E0001, not the type mismatch it would be if
    // the constructor resolved: the inferred one, the checked argument, and
    // the checked application (`check_app`'s own constructor shortcut).
    for body in [
        "pub fn f() -> Nat { let c = Red; 0 }\n",
        "fn pick(c: Nat) -> Nat { c }\npub fn f() -> Nat { pick(Red) }\n",
        "fn pick(c: Nat) -> Nat { c }\npub fn f() -> Nat { pick(Boxed(1)) }\n",
    ] {
        not_found(&with_transitive(USES_A, body));
    }
}

// 3.10.26m AC6: a value bound by `use` through a re-export is exempt — its
// path was reach-checked — although its defining package is not declared.
#[test]
fn a_reexported_value_bound_by_use_resolves_bare() {
    let lib_a = "pub use lib_c::double;\npub fn f() -> Nat { 1 }\n";
    let outcome = with_transitive(
        "use lib_a::double;\nfn main() -> Nat { double(2) }\n",
        lib_a,
    );
    assert!(
        matches!(outcome, Outcome::Elaborated),
        "{:?}",
        codes(&outcome)
    );
}

// 4.10.26j AC5 (D6): the application and `geometry` each defining a private
// `hidden` is no longer a collision — each package calls its own — while two
// packages declaring one extern C symbol still stop preparation (E0123).
#[test]
fn a_shared_value_name_elaborates_and_a_shared_extern_stops_preparation() {
    let outcome = with_geometry("fn hidden() -> Nat { 5 }\nfn main() -> Nat { hidden() }\n");
    assert!(
        matches!(outcome, Outcome::Elaborated),
        "{:?}",
        codes(&outcome)
    );

    let extern_lib = (
        "packages/geometry/lib.tg",
        "pub mod shapes;\npub extern \"C\" fn tg_x() -> Nat\npub fn area() -> Nat { 1 }\n",
    );
    let outcome = elaborate(&[
        (
            "main.tg",
            "extern \"C\" fn tg_x(n: Nat) -> Nat\nfn main() -> Nat { 0 }\n",
        ),
        ("albite.toml", APP_MANIFEST),
        GEOMETRY_MANIFEST,
        extern_lib,
        GEOMETRY_SHAPES,
    ]);
    assert!(matches!(
        outcome,
        Outcome::Unprepared(PipelineError::ElabFailed(_))
    ));
}
