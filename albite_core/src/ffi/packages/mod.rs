//! The package planner, for the self-host (ADR 6.10.26j D3): plan once, then
//! read the plan back through index accessors.
//!
//! ```text
//! h = tg_packages_plan(entry, app_modules, reserved)
//! tg_packages_has_manifest(h) / tg_packages_manifest_dir(h)
//! tg_packages_count(h) → per i: _name, _library_root, _declares_count/_declares
//! tg_packages_application_declares_count(h) / _application_declares
//! spans.rs (ADR 6.10.26p D4): tg_packages_application_name(h), per i:
//!   _manifest_path, _introduced_{file,start,end}, _dependency_keys,
//!   per nth: _dependency_key_{start,end}
//! tg_packages_problem_count(h) → per k: _tag, _file, _start, _end, _message
//! tg_packages_note_count(h) → per k: _note_text (ADR 20.10.26i D2)
//! ```
//!
//! Lists of strings cross as one newline-separated string, because no `tg_*`
//! FFI passes a list today. A problem crosses as a file and a byte range,
//! which is what a self-host `Span` is; the self-host computes line and column
//! as it does for every other record, and prints the core's message, so both
//! compilers word a refusal alike.
//!
//! An invalid handle or index answers zero counts, empty strings and
//! [`INVALID_HANDLE`] indices, never a crash. There is no `free`: a compile
//! plans once, and the registry lives for the process.

mod spans;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::sync::Mutex;

use crate::packages::{
    find_manifest, manifest_dir, parse_manifest, plan_graph, ByteRange, PlanProblem, Probes,
};

use super::INVALID_HANDLE;

/// One package of a plan, as the accessors hand it out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PackageRecord {
    pub(crate) name: String,
    pub(crate) library_root: String,
    pub(crate) declares: Vec<u64>,
    /// Its own manifest, and each `[dependencies]` key with where it is
    /// written: the library-key check's location (ADR 6.10.26p D4).
    pub(crate) manifest_path: String,
    pub(crate) dependency_keys: Vec<(String, ByteRange)>,
    /// The manifest and key that first named it: the extern collision's
    /// location (ADR 6.10.26p D4).
    pub(crate) introduced_by: (String, ByteRange),
}

/// A plan, flattened to what the accessors read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PlanRecord {
    /// Whether a manifest governs the entry — even one that was refused.
    pub(crate) has_manifest: bool,
    /// The manifest's directory, the self-host's sibling root; empty without one.
    pub(crate) manifest_dir: String,
    /// The application's `[package] name`; empty without a valid manifest.
    pub(crate) application_name: String,
    pub(crate) packages: Vec<PackageRecord>,
    pub(crate) application_declares: Vec<u64>,
    pub(crate) problems: Vec<PlanProblem>,
    /// The plan's non-fatal notes, as the text after `note: ` (ADR 20.10.26i D2).
    pub(crate) notes: Vec<String>,
}

/// The non-empty lines of a newline-separated list.
pub(crate) fn split_lines(list: &str) -> Vec<&str> {
    list.lines().filter(|line| !line.is_empty()).collect()
}

/// Plan the graph governing `entry` against the real filesystem, as the
/// bootstrap's `load_manifest` and `mount_packages` do.
pub(crate) fn plan_entry(entry: &Path, app_modules: &[&str], reserved: &[&str]) -> PlanRecord {
    let Some(path) = find_manifest(entry, Path::is_file) else {
        return PlanRecord::default();
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut record = PlanRecord {
        has_manifest: true,
        ..PlanRecord::default()
    };
    let app = match parse_manifest(&text, &path, reserved) {
        Ok(app) => app,
        Err(problems) => {
            record.problems = problems;
            return record;
        }
    };
    record.manifest_dir = manifest_dir(&app).to_string_lossy().into_owned();
    record.application_name.clone_from(&app.name);
    let modules: HashSet<String> = app_modules.iter().map(|m| (*m).to_string()).collect();
    let probes = Probes {
        canonical: &|p| std::fs::canonicalize(p).map_err(|e| e.to_string()),
        exists: &|p| p.exists(),
        read: &|p| std::fs::read_to_string(p).ok(),
        reserved,
    };
    match plan_graph(&app, &modules, &probes) {
        Err(problems) => record.problems = problems,
        Ok(plan) => {
            record.notes = plan.notes.iter().map(|n| n.message.clone()).collect();
            let indices = |list: &[usize]| list.iter().map(|&i| i as u64).collect();
            record.application_declares = indices(&plan.application_declares);
            record.packages = plan
                .packages
                .iter()
                .map(|package| PackageRecord {
                    name: package.name.clone(),
                    library_root: package.library_root.to_string_lossy().into_owned(),
                    declares: indices(&package.declares),
                    manifest_path: package.manifest.path.to_string_lossy().into_owned(),
                    dependency_keys: package
                        .manifest
                        .dependencies
                        .iter()
                        .map(|dep| (dep.key.clone(), dep.key_span))
                        .collect(),
                    introduced_by: (
                        package.introduced_by.0.to_string_lossy().into_owned(),
                        package.introduced_by.1,
                    ),
                })
                .collect();
        }
    }
    record
}

/// Every plan made this process, by handle.
static PLANS: Mutex<Vec<PlanRecord>> = Mutex::new(Vec::new());

/// Store a plan; its handle.
pub(crate) fn register(record: PlanRecord) -> u64 {
    let mut plans = PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    plans.push(record);
    (plans.len() - 1) as u64
}

/// Read plan `handle` through `read`, or `fallback` for an unknown handle.
fn with_plan<T>(handle: u64, fallback: T, read: impl FnOnce(&PlanRecord) -> T) -> T {
    let plans = PLANS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    usize::try_from(handle)
        .ok()
        .and_then(|index| plans.get(index))
        .map_or(fallback, read)
}

/// Package `index` of plan `handle`, read through `read`.
fn with_package<T>(
    handle: u64,
    index: u64,
    fallback: T,
    read: impl FnOnce(&PackageRecord) -> T,
) -> T {
    with_plan(handle, None, |plan| {
        plan.packages.get(index as usize).map(read)
    })
    .unwrap_or(fallback)
}

/// Problem `index` of plan `handle`, read through `read`.
fn with_problem<T>(
    handle: u64,
    index: u64,
    fallback: T,
    read: impl FnOnce(&PlanProblem) -> T,
) -> T {
    with_plan(handle, None, |plan| {
        plan.problems.get(index as usize).map(read)
    })
    .unwrap_or(fallback)
}

/// A leaked C string, as every string this FFI hands out is; empty on an
/// interior NUL.
fn leak(text: &str) -> *const c_char {
    CString::new(text)
        .unwrap_or_default()
        .into_raw()
        .cast_const()
}

/// A C string argument; empty for null or non-UTF-8.
///
/// # Safety
/// `ptr` must be null or a valid null-terminated C string.
unsafe fn read_arg(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_str().unwrap_or_default().to_string()
}

/// Plan the dependency graph governing `entry`. `app_modules` (the
/// application's top-level module names) and `reserved` (the caller's
/// reserved words) are newline-separated. Always returns a handle: a plan
/// with problems, or with no manifest, is still a plan.
///
/// # Safety
/// Each argument must be null or a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tg_packages_plan(
    entry: *const c_char,
    app_modules: *const c_char,
    reserved: *const c_char,
) -> u64 {
    let (entry, app_modules, reserved) =
        (read_arg(entry), read_arg(app_modules), read_arg(reserved));
    let record = plan_entry(
        Path::new(&entry),
        &split_lines(&app_modules),
        &split_lines(&reserved),
    );
    register(record)
}

/// Whether a manifest governs the plan's entry.
#[no_mangle]
pub extern "C" fn tg_packages_has_manifest(handle: u64) -> bool {
    with_plan(handle, false, |plan| plan.has_manifest)
}

/// The governing manifest's directory: the self-host's sibling root.
#[no_mangle]
pub extern "C" fn tg_packages_manifest_dir(handle: u64) -> *const c_char {
    leak(&with_plan(handle, String::new(), |plan| {
        plan.manifest_dir.clone()
    }))
}

/// How many packages the plan mounts.
#[no_mangle]
pub extern "C" fn tg_packages_count(handle: u64) -> u64 {
    with_plan(handle, 0, |plan| plan.packages.len() as u64)
}

/// Package `index`'s name, the module it mounts as.
#[no_mangle]
pub extern "C" fn tg_packages_name(handle: u64, index: u64) -> *const c_char {
    leak(&with_package(handle, index, String::new(), |p| {
        p.name.clone()
    }))
}

/// Package `index`'s `lib.tg`.
#[no_mangle]
pub extern "C" fn tg_packages_library_root(handle: u64, index: u64) -> *const c_char {
    leak(&with_package(handle, index, String::new(), |p| {
        p.library_root.clone()
    }))
}

/// How many packages package `index` declares.
#[no_mangle]
pub extern "C" fn tg_packages_declares_count(handle: u64, index: u64) -> u64 {
    with_package(handle, index, 0, |p| p.declares.len() as u64)
}

/// The `nth` package package `index` declares, as a package index.
#[no_mangle]
pub extern "C" fn tg_packages_declares(handle: u64, index: u64, nth: u64) -> u64 {
    with_package(handle, index, INVALID_HANDLE, |p| {
        p.declares
            .get(nth as usize)
            .copied()
            .unwrap_or(INVALID_HANDLE)
    })
}

/// How many packages the application declares.
#[no_mangle]
pub extern "C" fn tg_packages_application_declares_count(handle: u64) -> u64 {
    with_plan(handle, 0, |plan| plan.application_declares.len() as u64)
}

/// The `nth` package the application declares, as a package index.
#[no_mangle]
pub extern "C" fn tg_packages_application_declares(handle: u64, nth: u64) -> u64 {
    with_plan(handle, INVALID_HANDLE, |plan| {
        plan.application_declares
            .get(nth as usize)
            .copied()
            .unwrap_or(INVALID_HANDLE)
    })
}

/// How many refusals the plan holds.
#[no_mangle]
pub extern "C" fn tg_packages_problem_count(handle: u64) -> u64 {
    with_plan(handle, 0, |plan| plan.problems.len() as u64)
}

/// Problem `index`'s tag: the bootstrap's code number (120–122, 124); 0 when
/// out of range.
#[no_mangle]
pub extern "C" fn tg_packages_problem_tag(handle: u64, index: u64) -> u64 {
    with_problem(handle, index, 0, |p| p.kind.tag())
}

/// The file problem `index` spans into.
#[no_mangle]
pub extern "C" fn tg_packages_problem_file(handle: u64, index: u64) -> *const c_char {
    leak(&with_problem(handle, index, String::new(), |p| {
        p.file.to_string_lossy().into_owned()
    }))
}

/// Problem `index`'s start byte.
#[no_mangle]
pub extern "C" fn tg_packages_problem_start(handle: u64, index: u64) -> u64 {
    with_problem(handle, index, 0, |p| u64::from(p.range.start))
}

/// Problem `index`'s end byte (exclusive).
#[no_mangle]
pub extern "C" fn tg_packages_problem_end(handle: u64, index: u64) -> u64 {
    with_problem(handle, index, 0, |p| u64::from(p.range.end))
}

/// Problem `index`'s message, rendered by the core.
#[no_mangle]
pub extern "C" fn tg_packages_problem_message(handle: u64, index: u64) -> *const c_char {
    leak(&with_problem(handle, index, String::new(), |p| {
        p.message.clone()
    }))
}

/// How many non-fatal notes the plan holds (ADR 20.10.26i D2).
#[no_mangle]
pub extern "C" fn tg_packages_note_count(handle: u64) -> u64 {
    with_plan(handle, 0, |plan| plan.notes.len() as u64)
}

/// Note `index`'s text, without the `note: ` prefix; empty when out of range.
#[no_mangle]
pub extern "C" fn tg_packages_note_text(handle: u64, index: u64) -> *const c_char {
    leak(&with_plan(handle, String::new(), |plan| {
        plan.notes.get(index as usize).cloned().unwrap_or_default()
    }))
}
