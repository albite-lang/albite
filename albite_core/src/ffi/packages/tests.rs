//! The planner's FFI over tempdir fixtures (ADR 6.10.26j AC 2): every
//! accessor is driven through the externs the self-host calls.

use std::ffi::{c_char, CStr, CString};
use std::fs;
use std::path::Path;

use super::spans::*;
use super::*;

/// A file under `root`, its directories created.
fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// A package directory: its manifest and a `lib.tg`.
fn package(root: &Path, dir: &str, name: &str, deps: &[(&str, &str)]) {
    let mut manifest = format!("[package]\nname = \"{name}\"\n");
    if !deps.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for (key, path) in deps {
            manifest.push_str(&format!("{key} = {{ path = \"{path}\" }}\n"));
        }
    }
    write(root, &format!("{dir}/albite.toml"), &manifest);
    write(root, &format!("{dir}/lib.tg"), "pub fn f() -> Nat { 1 }\n");
}

/// Plan `entry` through the extern, as the self-host does.
fn plan(entry: &Path, app_modules: &str, reserved: &str) -> u64 {
    let entry = CString::new(entry.to_string_lossy().as_bytes()).expect("entry");
    let modules = CString::new(app_modules).expect("modules");
    let reserved = CString::new(reserved).expect("reserved");
    unsafe { tg_packages_plan(entry.as_ptr(), modules.as_ptr(), reserved.as_ptr()) }
}

/// A string the FFI handed out.
fn text(ptr: *const c_char) -> String {
    assert!(!ptr.is_null(), "the FFI never hands out null");
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("utf-8")
        .to_string()
}

/// A problem as the self-host reads it back.
struct ProblemSeen {
    tag: u64,
    file: String,
    /// The text its byte range covers in `file`.
    covered: String,
    message: String,
}

/// Problem `k`, read through every per-problem accessor.
fn problem(handle: u64, k: u64) -> ProblemSeen {
    let file = text(tg_packages_problem_file(handle, k));
    let source = fs::read_to_string(&file).expect("the problem's file");
    let (start, end) = (
        tg_packages_problem_start(handle, k) as usize,
        tg_packages_problem_end(handle, k) as usize,
    );
    ProblemSeen {
        tag: tg_packages_problem_tag(handle, k),
        covered: source[start..end].to_string(),
        message: text(tg_packages_problem_message(handle, k)),
        file,
    }
}

/// A tempdir holding an application whose manifest names `deps`.
fn application(deps: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    package(dir.path(), "app", "app", deps);
    write(dir.path(), "app/main.tg", "fn main() -> Nat { 0 }\n");
    dir
}

// 6.10.26j AC2: a diamond plans three packages with their declarations, and
// the manifest's directory is the sibling root.
#[test]
fn a_diamond_plans_three_packages_with_their_declarations() {
    let dir = application(&[("a", "../a"), ("b", "../b")]);
    let root = dir.path();
    package(root, "a", "a", &[("c", "../c")]);
    package(root, "b", "b", &[("c", "../c")]);
    package(root, "c", "c", &[]);
    let h = plan(&root.join("app/main.tg"), "util\n", "");
    assert!(tg_packages_has_manifest(h));
    assert_eq!(tg_packages_problem_count(h), 0);
    assert_eq!(
        text(tg_packages_manifest_dir(h)),
        root.join("app").to_string_lossy()
    );
    assert_eq!(tg_packages_count(h), 3);
    let names: Vec<String> = (0..3).map(|i| text(tg_packages_name(h, i))).collect();
    assert_eq!(names, ["a", "c", "b"]);
    assert_eq!(
        text(tg_packages_library_root(h, 1)),
        root.join("c/lib.tg").to_string_lossy()
    );
    assert_eq!(tg_packages_declares_count(h, 0), 1);
    assert_eq!(tg_packages_declares(h, 0, 0), 1);
    assert_eq!(tg_packages_declares(h, 2, 0), 1);
    assert_eq!(tg_packages_declares_count(h, 1), 0);
    assert_eq!(tg_packages_declares(h, 0, 1), INVALID_HANDLE);
    assert_eq!(tg_packages_application_declares_count(h), 2);
    assert_eq!(tg_packages_application_declares(h, 0), 0);
    assert_eq!(tg_packages_application_declares(h, 1), 2);
    assert_eq!(tg_packages_application_declares(h, 2), INVALID_HANDLE);
}

// 6.10.26j AC2: an invalid manifest is one E0120 on its key.
#[test]
fn an_invalid_manifest_is_one_problem_on_its_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "albite.toml",
        "[package]\nname = \"my-project\"\n",
    );
    let h = plan(&dir.path().join("main.tg"), "", "");
    assert!(tg_packages_has_manifest(h));
    assert_eq!(text(tg_packages_manifest_dir(h)), "");
    assert_eq!(tg_packages_problem_count(h), 1);
    let ProblemSeen {
        tag,
        file,
        covered,
        message,
    } = problem(h, 0);
    assert_eq!((tag, covered.as_str()), (120, "name"));
    assert!(file.ends_with("albite.toml"), "{file}");
    assert!(
        message.starts_with("invalid `albite.toml`: package name"),
        "{message}"
    );
}

// 6.10.26j AC2: a missing directory is E0121 and a key unequal to its
// package's name E0122, each on the key that declared it.
#[test]
fn a_missing_directory_and_a_misnamed_key_are_located() {
    let dir = application(&[("gone", "../gone"), ("lib_a", "../a")]);
    package(dir.path(), "a", "other", &[]);
    let h = plan(&dir.path().join("app/main.tg"), "", "");
    assert_eq!(tg_packages_problem_count(h), 2);
    let ProblemSeen {
        tag,
        file,
        covered,
        message,
    } = problem(h, 0);
    assert_eq!((tag, covered.as_str()), (121, "gone"));
    assert!(file.ends_with("app/albite.toml"), "{file}");
    assert!(message.contains("does not exist"), "{message}");
    let ProblemSeen {
        tag,
        covered,
        message,
        ..
    } = problem(h, 1);
    assert_eq!((tag, covered.as_str()), (122, "lib_a"));
    assert!(message.contains("named `other`"), "{message}");
    assert_eq!(tg_packages_count(h), 0);
}

// 6.10.26j AC2: a cycle is E0124 on the closing key, in the manifest holding it.
#[test]
fn a_cycle_is_one_problem_on_its_closing_key() {
    let dir = application(&[("a", "../a")]);
    package(dir.path(), "a", "a", &[("b", "../b")]);
    package(dir.path(), "b", "b", &[("a", "../a")]);
    let h = plan(&dir.path().join("app/main.tg"), "", "");
    assert_eq!(tg_packages_problem_count(h), 1);
    let ProblemSeen {
        tag,
        file,
        covered,
        message,
    } = problem(h, 0);
    assert_eq!((tag, covered.as_str()), (124, "a"));
    assert!(file.ends_with("b/albite.toml"), "{file}");
    assert_eq!(message, "dependency cycle: app → a → b → a");
}

// 6.10.26j AC2: the caller's reserved words and application modules are
// read from their newline-separated lists.
#[test]
fn the_callers_lists_are_read_line_by_line() {
    let dir = application(&[("util", "../util")]);
    package(dir.path(), "util", "util", &[]);
    let entry = dir.path().join("app/main.tg");
    let clear = plan(&entry, "", "");
    assert_eq!(tg_packages_problem_count(clear), 0);
    let clash = plan(&entry, "other\nutil\n", "");
    assert_eq!(tg_packages_problem_count(clash), 1);
    assert_eq!(problem(clash, 0).tag, 122);
    let reserved = plan(&entry, "", "fn\nutil");
    assert_eq!(tg_packages_problem_count(reserved), 1);
    assert!(problem(reserved, 0).message.contains("reserved word"));
    assert_eq!(split_lines("a\n\nb\n"), ["a", "b"]);
}

// 6.10.26j AC2: no manifest is no plan and no problem.
#[test]
fn no_manifest_is_an_empty_plan() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "main.tg", "fn main() -> Nat { 0 }\n");
    let h = plan(&dir.path().join("main.tg"), "", "");
    assert!(!tg_packages_has_manifest(h));
    assert_eq!(tg_packages_problem_count(h), 0);
    assert_eq!(tg_packages_count(h), 0);
    assert_eq!(text(tg_packages_manifest_dir(h)), "");
}

// 6.10.26j AC2: an invalid handle, or index, answers zeros and empty strings.
#[test]
fn an_invalid_handle_answers_zeros_and_empty_strings() {
    let bad = u64::MAX;
    assert!(!tg_packages_has_manifest(bad));
    assert_eq!(text(tg_packages_manifest_dir(bad)), "");
    assert_eq!(tg_packages_count(bad), 0);
    assert_eq!(text(tg_packages_name(bad, 0)), "");
    assert_eq!(text(tg_packages_library_root(bad, 0)), "");
    assert_eq!(tg_packages_declares_count(bad, 0), 0);
    assert_eq!(tg_packages_declares(bad, 0, 0), INVALID_HANDLE);
    assert_eq!(tg_packages_application_declares_count(bad), 0);
    assert_eq!(tg_packages_application_declares(bad, 0), INVALID_HANDLE);
    assert_eq!(tg_packages_problem_count(bad), 0);
    assert_eq!(tg_packages_problem_tag(bad, 0), 0);
    assert_eq!(text(tg_packages_problem_file(bad, 0)), "");
    assert_eq!(tg_packages_problem_start(bad, 0), 0);
    assert_eq!(tg_packages_problem_end(bad, 0), 0);
    assert_eq!(text(tg_packages_problem_message(bad, 0)), "");
    // A null argument reads as empty: no entry, so no manifest.
    let h = unsafe { tg_packages_plan(std::ptr::null(), std::ptr::null(), std::ptr::null()) };
    assert!(!tg_packages_has_manifest(h));
    // An out-of-range index on a real plan is the same.
    let dir = application(&[]);
    let real = plan(&dir.path().join("app/main.tg"), "", "");
    assert!(tg_packages_has_manifest(real));
    assert_eq!(text(tg_packages_name(real, 7)), "");
    assert_eq!(tg_packages_problem_tag(real, 7), 0);
}

/// The text `[start, end)` covers in `file`.
fn covered(file: &str, start: u64, end: u64) -> String {
    let source = fs::read_to_string(file).expect("the span's file");
    source[start as usize..end as usize].to_string()
}

// 6.10.26p D4: the post-mount refusals' locations read back — the
// application's name, each package's manifest and keys, and the key that
// introduced it.
#[test]
fn the_post_mount_refusals_locations_read_back() {
    let dir = application(&[("a", "../a"), ("b", "../b")]);
    let root = dir.path();
    package(root, "a", "a", &[("c", "../c")]);
    package(root, "b", "b", &[("c", "../c")]);
    package(root, "c", "c", &[]);
    let h = plan(&root.join("app/main.tg"), "", "");
    assert_eq!(text(tg_packages_application_name(h)), "app");
    // Package 0 is `a`: its own manifest, its one key `c`.
    let a_manifest = text(tg_packages_manifest_path(h, 0));
    assert!(a_manifest.ends_with("a/albite.toml"), "{a_manifest}");
    assert_eq!(text(tg_packages_dependency_keys(h, 0)), "c");
    let (start, end) = (
        tg_packages_dependency_key_start(h, 0, 0),
        tg_packages_dependency_key_end(h, 0, 0),
    );
    assert_eq!(covered(&a_manifest, start, end), "c");
    assert_eq!(tg_packages_dependency_key_start(h, 0, 1), 0);
    assert_eq!(tg_packages_dependency_key_end(h, 0, 1), 0);
    // `c` (index 1) was first named by `a`'s key; `b` (index 2) by the app's.
    let c_file = text(tg_packages_introduced_file(h, 1));
    assert_eq!(c_file, a_manifest);
    let c_key = covered(
        &c_file,
        tg_packages_introduced_start(h, 1),
        tg_packages_introduced_end(h, 1),
    );
    assert_eq!(c_key, "c");
    let b_file = text(tg_packages_introduced_file(h, 2));
    assert!(b_file.ends_with("app/albite.toml"), "{b_file}");
    let b_key = covered(
        &b_file,
        tg_packages_introduced_start(h, 2),
        tg_packages_introduced_end(h, 2),
    );
    assert_eq!(b_key, "b");
    // A package with no keys, and an out-of-range index, answer "".
    assert_eq!(text(tg_packages_dependency_keys(h, 1)), "");
    assert_eq!(text(tg_packages_dependency_keys(h, 7)), "");
}

// 6.10.26p D4: two keys cross as two lines, and an invalid handle answers
// empty strings and zeros.
#[test]
fn the_span_accessors_answer_empty_for_a_bad_handle() {
    let dir = application(&[]);
    let root = dir.path();
    write(
        root,
        "app/albite.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\nx = { path = \"../x\" }\n",
    );
    package(root, "x", "x", &[("y", "../y"), ("z", "../z")]);
    package(root, "y", "y", &[]);
    package(root, "z", "z", &[]);
    let h = plan(&root.join("app/main.tg"), "", "");
    assert_eq!(text(tg_packages_dependency_keys(h, 0)), "y\nz");
    let bad = u64::MAX;
    assert_eq!(text(tg_packages_application_name(bad)), "");
    assert_eq!(text(tg_packages_manifest_path(bad, 0)), "");
    assert_eq!(text(tg_packages_introduced_file(bad, 0)), "");
    assert_eq!(tg_packages_introduced_start(bad, 0), 0);
    assert_eq!(tg_packages_introduced_end(bad, 0), 0);
    assert_eq!(text(tg_packages_dependency_keys(bad, 0)), "");
    assert_eq!(tg_packages_dependency_key_start(bad, 0, 0), 0);
    assert_eq!(tg_packages_dependency_key_end(bad, 0, 0), 0);
}

// 20.10.26i AC2: a diamond's legacy-named package crosses as one note, which
// the self-host prints after `note: `; a bad handle or index answers empty.
#[test]
fn a_legacy_manifest_crosses_as_one_note() {
    let dir = application(&[("a", "../a"), ("b", "../b")]);
    let root = dir.path();
    package(root, "a", "a", &[("c", "../c")]);
    package(root, "b", "b", &[("c", "../c")]);
    write(root, "c/tungsten.toml", "[package]\nname = \"c\"\n");
    write(root, "c/lib.tg", "pub fn f() -> Nat { 1 }\n");
    let h = plan(&root.join("app/main.tg"), "", "");
    assert_eq!(tg_packages_problem_count(h), 0);
    assert_eq!(tg_packages_note_count(h), 1);
    let note = text(tg_packages_note_text(h, 0));
    assert!(note.ends_with("uses the old manifest name; rename it to `albite.toml`"));
    assert!(note.contains("c/tungsten.toml"), "{note}");
    assert_eq!(text(tg_packages_note_text(h, 1)), "");
    assert_eq!(tg_packages_note_count(u64::MAX), 0);
    assert_eq!(text(tg_packages_note_text(u64::MAX, 0)), "");
}
