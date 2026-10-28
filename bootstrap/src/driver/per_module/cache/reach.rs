//! The reach fingerprint a module's cache key carries (ADR 3.10.26m D12).
//!
//! What a module may name depends on its package's `[dependencies]`, which
//! neither the file's bytes nor the exports hash see; every key site folds
//! the owning package's declared names into the content hash through here.

use std::path::Path;

use crate::cache::elab_cache;
use crate::driver::modules::{ModuleInfo, ParsedModule};
use crate::driver::pipeline::BuildCtx;
use crate::elaborate::PackageId;

/// `content`'s hash, folded with the reach of the package owning `module`
/// and with the program's scoped set (ADR 4.10.26j D7, 9.10.26f D1): a
/// cached module output names its values by Core name, and whether a name is
/// scoped depends on modules this file never mentions. The set is global,
/// so a new clash — or a new reserved definition — anywhere re-elaborates
/// every module once: coarse, on purpose, since no cache records a module's
/// free names.
pub(in crate::driver::per_module) fn module_hash(
    module: &ParsedModule,
    build: &BuildCtx<'_>,
    content: &[u8],
) -> [u8; 32] {
    let declared = declared_dependencies(&build.module_info, &module.path);
    let reach_keyed =
        elab_cache::reach_keyed_content_hash(elab_cache::hash_file_content(content), &declared);
    scoped_keyed_hash(reach_keyed, &build.module_info.core_names())
}

/// `hash` folded with `names`' scoped set.
fn scoped_keyed_hash(hash: [u8; 32], names: &crate::elaborate::CoreNames) -> [u8; 32] {
    let scoped_set = names.scoped_set();
    crate::cache::collision_keyed_hash(hash, scoped_set.iter().map(String::as_str))
}

/// The sorted dependency names of the package owning `file`: the
/// application's when the file is not one of a mounted package's. Empty
/// without a manifest, which keeps every key as it was.
pub(in crate::driver::per_module) fn declared_dependencies(
    info: &ModuleInfo,
    file: &Path,
) -> Vec<String> {
    let package = info
        .file_to_module
        .get(file)
        .map_or(PackageId::Application, |module| {
            info.packages.package_of(module)
        });
    info.packages.declared_names(package)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::declared_dependencies;
    use crate::driver::modules::ModuleInfo;
    use crate::elaborate::{ModulePath, PackageId};

    // ADR 3.10.26m AC10 (D12): each file keys on its own package's
    // declarations — a library's, the application's, or none at all.
    #[test]
    fn each_file_takes_its_own_package_declarations() {
        let mut info = ModuleInfo::default();
        let lib_a = info
            .packages
            .add_package("lib_a", ModulePath::from_name("lib_a"));
        let lib_c = info
            .packages
            .add_package("lib_c", ModulePath::from_name("lib_c"));
        info.packages.declare(PackageId::Application, lib_a);
        info.packages.declare(PackageId::Dependency(lib_a), lib_c);
        let lib_file = PathBuf::from("/p/lib_a/lib.tg");
        let app_file = PathBuf::from("/app/main.tg");
        info.file_to_module
            .insert(lib_file.clone(), ModulePath::from_name("lib_a"));
        info.file_to_module
            .insert(app_file.clone(), ModulePath::root());

        assert_eq!(declared_dependencies(&info, &lib_file), ["lib_c"]);
        assert_eq!(declared_dependencies(&info, &app_file), ["lib_a"]);
        // A file the tree never mapped is the application's.
        assert_eq!(
            declared_dependencies(&info, &PathBuf::from("/x.tg")),
            ["lib_a"]
        );
        let lib_c_file = PathBuf::from("/p/lib_c/lib.tg");
        info.file_to_module
            .insert(lib_c_file.clone(), ModulePath::from_name("lib_c"));
        assert!(declared_dependencies(&info, &lib_c_file).is_empty());
        assert!(declared_dependencies(&ModuleInfo::default(), &app_file).is_empty());
    }

    // ADR 4.10.26j AC6 (D7): the collision fold leaves a clash-free program's
    // key alone, moves it for any clash, and keeps the names apart.
    #[test]
    fn the_collision_set_folds_into_the_key() {
        use crate::cache::collision_keyed_hash;
        let hash = [0x5A; 32];
        assert_eq!(collision_keyed_hash(hash, std::iter::empty()), hash);
        let one_clash = collision_keyed_hash(hash, ["helper"]);
        assert_ne!(one_clash, hash);
        assert_eq!(one_clash, collision_keyed_hash(hash, ["helper"]));
        assert_ne!(one_clash, collision_keyed_hash(hash, ["helper", "run"]));
        assert_ne!(
            collision_keyed_hash(hash, ["ab"]),
            collision_keyed_hash(hash, ["a", "b"])
        );
        assert_ne!(one_clash, collision_keyed_hash([0x5B; 32], ["helper"]));
    }

    // ADR 9.10.26f AC3 (D1): one reserved single definer moves the key, a
    // program that scopes nothing keeps it, and a reserved `write` and a
    // colliding `write` fold differently.
    #[test]
    fn a_reserved_single_definer_moves_the_key() {
        use super::scoped_keyed_hash;
        use crate::elaborate::CoreNames;
        let hash = [0x5A; 32];
        let def = |module: &str, name: &str| (ModulePath::from_name(module), name.to_string());
        let plain = CoreNames::from_definitions([def("x", "helper")], []);
        assert_eq!(scoped_keyed_hash(hash, &plain), hash);
        let reserved = CoreNames::from_definitions([def("x", "write")], []);
        assert_eq!(reserved.scoped_set(), ["reserved:write"]);
        assert_ne!(scoped_keyed_hash(hash, &reserved), hash);
        let colliding = CoreNames::from_definitions([def("x", "write"), def("y", "write")], []);
        assert_eq!(colliding.scoped_set(), ["write"]);
        assert_ne!(
            scoped_keyed_hash(hash, &reserved),
            scoped_keyed_hash(hash, &colliding)
        );
        let extern_only = CoreNames::from_definitions([def("x", "write")], [def("x", "write")]);
        assert_eq!(scoped_keyed_hash(hash, &extern_only), hash);
    }
}
