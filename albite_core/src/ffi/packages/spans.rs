//! Where the self-host locates the two post-mount refusals (ADR 6.10.26p D4):
//! the application's name, each package's own manifest and dependency keys,
//! and the manifest key that introduced it. The refusals themselves are
//! decided in `.tg` (`driver/packages/collisions.tg`), over the mounted trees
//! the planner never sees; these accessors only read the plan back.

use std::ffi::c_char;

use super::{leak, with_package, with_plan};

/// The application's `[package] name`; empty without a valid manifest.
#[no_mangle]
pub extern "C" fn tg_packages_application_name(handle: u64) -> *const c_char {
    leak(&with_plan(handle, String::new(), |plan| {
        plan.application_name.clone()
    }))
}

/// Package `index`'s own `albite.toml`.
#[no_mangle]
pub extern "C" fn tg_packages_manifest_path(handle: u64, index: u64) -> *const c_char {
    leak(&with_package(handle, index, String::new(), |p| {
        p.manifest_path.clone()
    }))
}

/// The manifest whose key first named package `index`.
#[no_mangle]
pub extern "C" fn tg_packages_introduced_file(handle: u64, index: u64) -> *const c_char {
    leak(&with_package(handle, index, String::new(), |p| {
        p.introduced_by.0.clone()
    }))
}

/// That key's start byte.
#[no_mangle]
pub extern "C" fn tg_packages_introduced_start(handle: u64, index: u64) -> u64 {
    with_package(handle, index, 0, |p| u64::from(p.introduced_by.1.start))
}

/// That key's end byte (exclusive).
#[no_mangle]
pub extern "C" fn tg_packages_introduced_end(handle: u64, index: u64) -> u64 {
    with_package(handle, index, 0, |p| u64::from(p.introduced_by.1.end))
}

/// Package `index`'s `[dependencies]` keys, one per line, in manifest order.
#[no_mangle]
pub extern "C" fn tg_packages_dependency_keys(handle: u64, index: u64) -> *const c_char {
    leak(&with_package(handle, index, String::new(), |p| {
        let keys: Vec<&str> = p
            .dependency_keys
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        keys.join("\n")
    }))
}

/// Where package `index`'s `nth` dependency key starts in its manifest.
#[no_mangle]
pub extern "C" fn tg_packages_dependency_key_start(handle: u64, index: u64, nth: u64) -> u64 {
    with_package(handle, index, 0, |p| {
        p.dependency_keys
            .get(nth as usize)
            .map_or(0, |(_, range)| u64::from(range.start))
    })
}

/// Where it ends (exclusive).
#[no_mangle]
pub extern "C" fn tg_packages_dependency_key_end(handle: u64, index: u64, nth: u64) -> u64 {
    with_package(handle, index, 0, |p| {
        p.dependency_keys
            .get(nth as usize)
            .map_or(0, |(_, range)| u64::from(range.end))
    })
}
