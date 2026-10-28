//! The package boundary in visibility and in the package table itself
//! (ADR 29.9.26g D5/D6).

use super::*;

use crate::elaborate::env::{PackageId, PackageTable};

fn path(segments: &[&str]) -> ModulePath {
    ModulePath::new(segments.iter().map(|s| (*s).to_string()).collect())
}

// 29.9.26g AC5: every arm of `is_item_accessible`, within and across a package.
#[test]
fn item_arms_within_and_across_a_package() {
    let env = cross_package(&Env::new(), &path(&["geometry"]));
    let lib_root = path(&["geometry"]);
    let lib_shapes = path(&["geometry", "shapes"]);
    let app_root = ModulePath::root();
    let app_mod = path(&["app_mod"]);

    // Public crosses both ways.
    assert!(env.is_item_accessible(Visibility::Public, &lib_shapes, &app_mod));
    assert!(env.is_item_accessible(Visibility::Public, &app_root, &lib_root));
    // Crate stays inside its package.
    assert!(env.is_item_accessible(Visibility::Crate, &lib_shapes, &lib_root));
    assert!(!env.is_item_accessible(Visibility::Crate, &lib_shapes, &app_mod));
    assert!(env.is_item_accessible(Visibility::Crate, &app_mod, &app_root));
    // Private: today's module-family rule, and the same package.
    assert!(env.is_item_accessible(Visibility::Private, &lib_root, &lib_shapes));
    assert!(!env.is_item_accessible(Visibility::Private, &lib_shapes, &lib_root));
    assert!(!env.is_item_accessible(Visibility::Private, &lib_root, &app_root));
}

// 29.9.26g AC5: the bare-name leak — every path starts with the root, so the
// application's private root-level item was reachable from a library.
#[test]
fn application_private_root_item_is_not_visible_from_a_library() {
    let env = cross_package(&Env::new(), &path(&["geometry"]));
    let app_root = ModulePath::root();
    assert!(!env.is_item_accessible(Visibility::Private, &app_root, &path(&["geometry"])));
    assert!(env.is_item_accessible(Visibility::Private, &app_root, &path(&["app_mod"])));
}

// 29.9.26g AC5: `is_module_accessible` across a package.
#[test]
fn module_arms_across_a_package() {
    let mut env = Env::new();
    let root = ModulePath::root();
    let lib = path(&["geometry"]);
    let hidden = path(&["geometry", "hidden"]);
    let crate_only = path(&["geometry", "crate_only"]);
    env.register_module_with_visibility(lib.clone(), Visibility::Public, Some(root.clone()));
    env.register_module_with_visibility(hidden.clone(), Visibility::Private, Some(lib.clone()));
    env.register_module_with_visibility(crate_only.clone(), Visibility::Crate, Some(lib.clone()));
    let env = cross_package(&env, &lib);

    assert!(env.is_module_accessible(&lib, &root));
    assert!(!env.is_module_accessible(&hidden, &root));
    assert!(env.is_module_accessible(&hidden, &lib));
    assert!(!env.is_module_accessible(&crate_only, &root));
    assert!(env.is_module_accessible(&crate_only, &hidden));
}

// 29.9.26g AC5: without a manifest the table is empty and nothing changes.
#[test]
fn empty_table_is_one_package() {
    let env = Env::new();
    let a = path(&["a"]);
    let b = path(&["b"]);
    assert!(env.is_item_accessible(Visibility::Crate, &a, &b));
    assert!(env.package_may_reach(&a, &b));
    assert_eq!(env.package_of(&a), PackageId::Application);
}

// 29.9.26g AC6: a qualified path's module is the child, else the sibling,
// else the path rooted at the current package — skipping a candidate that
// does not exist, and one in another package.
#[test]
fn qualified_paths_are_package_rooted() {
    let mut env = Env::new();
    for module in [
        path(&["geometry"]),
        path(&["geometry", "shapes"]),
        path(&["geometry", "util"]),
        path(&["app_util"]),
    ] {
        env.register_module(module);
    }
    let env = cross_package(&env, &path(&["geometry"]));
    let from_lib_root = path(&["geometry"]);
    let from_shapes = path(&["geometry", "shapes"]);

    // Child.
    assert_eq!(
        env.resolve_qualifier(&path(&["shapes"]), &from_lib_root),
        path(&["geometry", "shapes"])
    );
    // No child `geometry::shapes::util`, so the sibling.
    assert_eq!(
        env.resolve_qualifier(&path(&["util"]), &from_shapes),
        path(&["geometry", "util"])
    );
    // The sibling `app_util` is the application's: skipped, and the
    // package-rooted absolute path (which does not exist) is returned.
    assert_eq!(
        env.resolve_qualifier(&path(&["app_util"]), &from_lib_root),
        path(&["geometry", "app_util"])
    );
    // The application reaches its dependency by name.
    assert_eq!(
        env.resolve_qualifier(&path(&["geometry", "shapes"]), &path(&["app_util"])),
        path(&["geometry", "shapes"])
    );
}

// 29.9.26g AC6: package_of takes the longest mount prefix; package_root and
// package_may_reach follow from it.
#[test]
fn package_table_queries() {
    let mut table = PackageTable::default();
    assert!(table.is_empty());
    table.add_dependency("geometry", path(&["geometry"]));
    table.add_dependency("inner", path(&["geometry", "inner"]));
    assert!(!table.is_empty());

    assert_eq!(
        table.package_of(&ModulePath::root()),
        PackageId::Application
    );
    assert_eq!(table.package_of(&path(&["app"])), PackageId::Application);
    assert_eq!(
        table.package_of(&path(&["geometry"])),
        PackageId::Dependency(0)
    );
    assert_eq!(
        table.package_of(&path(&["geometry", "shapes"])),
        PackageId::Dependency(0)
    );
    assert_eq!(
        table.package_of(&path(&["geometry", "inner", "x"])),
        PackageId::Dependency(1)
    );
    assert_eq!(
        table.package_root(PackageId::Dependency(1)),
        path(&["geometry", "inner"])
    );
    assert_eq!(
        table.package_root(PackageId::Application),
        ModulePath::root()
    );
    assert_eq!(
        table.package_name(PackageId::Dependency(0)),
        Some("geometry")
    );
    assert_eq!(table.package_name(PackageId::Application), None);

    let env = cross_package(&Env::new(), &path(&["geometry"]));
    // The application may reach into a dependency; a dependency may not reach out.
    assert!(env.package_may_reach(&ModulePath::root(), &path(&["geometry", "shapes"])));
    assert!(!env.package_may_reach(&path(&["geometry"]), &path(&["app_mod"])));
    assert!(env.package_may_reach(&path(&["geometry"]), &path(&["geometry", "shapes"])));
}

/// The diamond `app → {a, b} → c`, each package mounted at the root.
fn diamond() -> PackageTable {
    let mut table = PackageTable::default();
    let [a, b, c] = ["a", "b", "c"].map(|name| table.add_package(name, path(&[name])));
    table.declare(PackageId::Application, a);
    table.declare(PackageId::Application, b);
    table.declare(PackageId::Dependency(a), c);
    table.declare(PackageId::Dependency(b), c);
    table.declare(PackageId::Dependency(b), c); // declared twice, recorded once
    table
}

// 3.10.26m AC5: a package reaches itself and what it declared, and nothing
// else — the application not its transitive package, no library the
// application, and no library its sibling.
#[test]
fn may_reach_follows_each_package_declarations() {
    let table = diamond();
    let [app, a, b, c] = [
        PackageId::Application,
        PackageId::Dependency(0),
        PackageId::Dependency(1),
        PackageId::Dependency(2),
    ];
    for (from, to, reach) in [
        (app, app, true),
        (app, a, true),
        (app, b, true),
        (app, c, false),
        (a, a, true),
        (a, c, true),
        (b, c, true),
        (a, b, false),
        (c, a, false),
        (a, app, false),
        (c, app, false),
    ] {
        assert_eq!(table.may_reach(from, to), reach, "{from:?} → {to:?}");
    }
    assert_eq!(table.declared_names(b), ["c"]);
    assert_eq!(table.declared_names(app), ["a", "b"]);
    assert!(table.declared_names(c).is_empty());
}

// 3.10.26m AC5: an absolute path starts at the root only for a first segment
// the package declared; otherwise at the package's own root.
#[test]
fn absolute_root_is_the_root_only_for_a_declared_dependency() {
    let table = diamond();
    let a = PackageId::Dependency(0);
    assert_eq!(table.absolute_root(a, "c"), ModulePath::root());
    assert_eq!(table.absolute_root(a, "b"), path(&["a"]));
    assert_eq!(table.absolute_root(a, "shapes"), path(&["a"]));
    assert_eq!(
        table.absolute_root(PackageId::Application, "c"),
        ModulePath::root()
    );
    assert_eq!(
        table.absolute_root(PackageId::Application, "a"),
        ModulePath::root()
    );
}

// 3.10.26m AC5: with no manifest the table is empty — everything is the
// application, which reaches itself, and absolute paths start at the root.
#[test]
fn an_empty_table_answers_as_before() {
    let table = PackageTable::default();
    let app = table.package_of(&path(&["anything"]));
    assert!(table.may_reach(app, table.package_of(&path(&["other"]))));
    assert_eq!(table.absolute_root(app, "anything"), ModulePath::root());
    assert!(table.declared_names(app).is_empty());
    assert_eq!(table.package_name(app), None);
}

// 3.10.26m D5: the bare-name refusal's helpers — a value and a constructor in
// an undeclared package are unreachable, a builtin (no recorded module) is
// not, and the help names the package and the missing declaration.
#[test]
fn unreachable_bare_names_and_their_help() {
    let mut env = Env::new();
    env.packages = diamond();
    env.set_item_module("in_b", path(&["b"]));
    env.set_item_module("in_c", path(&["c"]));
    let from_a = path(&["a", "inner"]);
    let b = PackageId::Dependency(1);
    assert_eq!(env.unreachable_value(&from_a, "in_b"), Some(b));
    assert_eq!(env.unreachable_value(&from_a, "in_c"), None);
    assert_eq!(env.unreachable_value(&from_a, "builtin"), None);
    let ctor = |module: &[&str]| crate::elaborate::env::ConstructorInfo {
        type_name: "T".into(),
        index: 0,
        arity: 0,
        visibility: None,
        defining_module: Some(path(module)),
    };
    assert_eq!(env.unreachable_constructor(&from_a, &ctor(&["b"])), Some(b));
    assert_eq!(
        env.unreachable_constructor(&from_a, &ctor(&["c", "x"])),
        None
    );
    assert_eq!(
        env.undeclared_package_help(&from_a, "in_b", b),
        "`in_b` is defined in package `b`, which `a` does not declare in `[dependencies]`"
    );
    assert_eq!(
        env.undeclared_package_help(&from_a, "main", PackageId::Application),
        "`main` is defined in the application, which package `a` cannot name"
    );
    // Without a manifest nothing is ever unreachable.
    let mut flat = Env::new();
    flat.set_item_module("in_b", path(&["b"]));
    assert_eq!(flat.unreachable_value(&from_a, "in_b"), None);
}
