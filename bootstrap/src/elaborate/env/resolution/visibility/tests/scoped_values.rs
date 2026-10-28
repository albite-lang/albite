//! Two modules sharing a value name (ADR 4.10.26j): a qualified path answers
//! from its module (D2), a bare name the caller may see twice is ambiguous
//! (D3), and a type stub belongs to its definer (D5).

use std::sync::Arc;

use super::*;

use crate::driver::modules::ModuleInfo;
use crate::elaborate::env::{
    CoreNames, ImportRequest, ModuleContents, PathResolutionError, ResolvedValue, ValueDef,
};
use crate::span::Span;
use albite_core::Type;

fn path(segments: &[&str]) -> ModulePath {
    ModulePath::new(segments.iter().map(|s| (*s).to_string()).collect())
}

fn simple(name: &str) -> crate::ast::Path {
    crate::ast::Path::simple(crate::ast::Ident::new(name, Span::new(0, 1)))
}

fn qualified(module: &str, name: &str) -> crate::ast::Path {
    crate::ast::Path {
        segments: vec![
            crate::ast::Ident::new(module, Span::new(0, 1)),
            crate::ast::Ident::new(name, Span::new(3, 4)),
        ],
        span: Span::new(0, 4),
    }
}

/// `helper` defined in each `(module, visibility, type)`, plus the modules
/// themselves and the root, with the Core-name table built from them.
fn env_defining_helper(definitions: &[(&str, Visibility, Type)]) -> Env {
    let mut env = Env::new();
    env.register_module(ModulePath::root());
    env.core_names = Arc::new(CoreNames::from_definitions(
        definitions
            .iter()
            .map(|(module, _, _)| (path(&[module]), "helper".to_string())),
        [],
    ));
    for (module, visibility, ty) in definitions {
        env.register_module(path(&[module]));
        let def = ValueDef {
            name: "helper".to_string(),
            ty: ty.clone(),
            visibility: *visibility,
            span: Span::new(0, 6),
        };
        env.define_value_in_module(def, path(&[module]));
    }
    env
}

fn nat_to_nat() -> Type {
    Type::arrow(Type::Nat, Type::Nat)
}

fn bool_to_bool() -> Type {
    Type::arrow(Type::Bool, Type::Bool)
}

fn resolved(env: &Env, name: &crate::ast::Path, from: &ModulePath) -> (String, Type) {
    match env.resolve_value_path(name, 0, from) {
        Ok(Some(ResolvedValue::Global(core_name, ty))) => (core_name, ty),
        other => panic!("expected a global, got {other:?}"),
    }
}

// 4.10.26j AC2: `x::helper` is `x`'s definition, type and Core name, even
// when `y`'s `helper` has a different type; `use x::helper` binds `x`'s.
#[test]
fn a_qualified_path_resolves_to_its_own_module() {
    let mut env = env_defining_helper(&[
        ("x", Visibility::Public, nat_to_nat()),
        ("y", Visibility::Public, bool_to_bool()),
    ]);
    let root = ModulePath::root();
    assert_eq!(
        resolved(&env, &qualified("x", "helper"), &root),
        ("_tg_1x_6helper".to_string(), nat_to_nat())
    );
    assert_eq!(
        resolved(&env, &qualified("y", "helper"), &root),
        ("_tg_1y_6helper".to_string(), bool_to_bool())
    );
    assert_eq!(env.value_module("_tg_1x_6helper"), Some(&path(&["x"])));
    assert_eq!(env.value_source_name("_tg_1y_6helper"), "helper");

    env.add_value_import(
        &root,
        ImportRequest {
            local_name: "helper".to_string(),
            source_module: path(&["x"]),
            original_name: "helper".to_string(),
            span: Span::new(0, 6),
            is_reexport: false,
            reexport_visibility: None,
        },
    );
    assert_eq!(
        resolved(&env, &simple("helper"), &root),
        ("_tg_1x_6helper".to_string(), nat_to_nat())
    );
}

// 4.10.26j AC3: a bare name two visible definitions match is refused, naming
// both; the defining module's own bare call is not ambiguous.
#[test]
fn a_bare_name_matching_two_visible_definitions_is_ambiguous() {
    let env = env_defining_helper(&[
        ("x", Visibility::Public, nat_to_nat()),
        ("y", Visibility::Public, bool_to_bool()),
    ]);
    match env.resolve_value_path(&simple("helper"), 0, &ModulePath::root()) {
        Err(PathResolutionError::Ambiguous { item, candidates }) => {
            assert_eq!(item, "helper");
            assert_eq!(candidates, ["x::helper", "y::helper"]);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
    assert_eq!(
        resolved(&env, &simple("helper"), &path(&["y"])).0,
        "_tg_1y_6helper",
        "a module's own definition comes first"
    );
}

// 4.10.26j AC3: a private `helper` in a sibling the caller cannot see is no
// candidate, so the one visible definition resolves; with none visible, a
// private one is returned for the caller to report as E0016.
#[test]
fn an_invisible_definition_is_not_a_candidate() {
    let env = env_defining_helper(&[
        ("x", Visibility::Public, nat_to_nat()),
        ("z", Visibility::Private, bool_to_bool()),
    ]);
    assert_eq!(
        resolved(&env, &simple("helper"), &ModulePath::root()),
        ("_tg_1x_6helper".to_string(), nat_to_nat())
    );

    let all_private = env_defining_helper(&[
        ("x", Visibility::Private, nat_to_nat()),
        ("z", Visibility::Private, bool_to_bool()),
    ]);
    let (core_name, _) = resolved(&all_private, &simple("helper"), &ModulePath::root());
    assert_eq!(core_name, "_tg_1x_6helper");
    assert!(!all_private.is_item_accessible(
        Visibility::Private,
        all_private.value_module(&core_name).expect("module"),
        &ModulePath::root()
    ));
}

// 4.10.26j AC3: a local of the same name shadows a colliding global.
#[test]
fn a_local_shadows_a_colliding_value() {
    let mut env = env_defining_helper(&[
        ("x", Visibility::Public, nat_to_nat()),
        ("y", Visibility::Public, bool_to_bool()),
    ]);
    env.push_scope();
    env.bind_local("helper".to_string(), Type::Nat, 0);
    assert!(matches!(
        env.resolve_value_path(&simple("helper"), 1, &ModulePath::root()),
        Ok(Some(ResolvedValue::Local(0, Type::Nat)))
    ));
}

// 4.10.26j AC4 (D5): a re-exporter whose path sorts first does not own the
// type it copied; the stub's defining module is the definer.
#[test]
fn a_type_stub_is_owned_by_its_definer_not_a_reexporter() {
    let mut info = ModuleInfo::default();
    let mut reexporter = ModuleContents::default();
    reexporter.types.push("Colour".to_string());
    info.modules.insert(path(&["a_reexporter"]), reexporter);
    let mut definer = ModuleContents::default();
    definer.types.push("Colour".to_string());
    definer.defined_types.insert("Colour".to_string());
    info.modules.insert(path(&["z_definer"]), definer);

    let mut env = Env::new();
    env.populate_module_info(info);
    let stub = env.lookup_type("Colour").expect("stub registered");
    assert_eq!(stub.defining_module, Some(path(&["z_definer"])));
    assert_eq!(env.get_item_module("Colour"), Some(&path(&["z_definer"])));
}
