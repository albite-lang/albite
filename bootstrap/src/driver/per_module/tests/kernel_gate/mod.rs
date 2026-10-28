//! ADR 2.10.26d AC 4 / AC 5: the term-mode kernel gate through the shared
//! path — injected-register drills, the census window, the single-file
//! entry, and warm runs that must hide nothing.
//!
//! A folder of its own because `per_module/tests/` holds its eight-file cap.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::conversion_discharge::{run_fresh, run_tree};
use crate::cache::BuildCache;
use crate::driver::modules::{ModuleInfo, SourceMap};
use crate::driver::output::TraceOptions;
use crate::driver::per_module::ModuleTreeOutput;
use crate::elaborate::kernel::{register_key, repository_root, Register};
use crate::elaborate::{ElabError, ElabErrorKind};

const TWO: &str = "fn two() -> Nat { 2 }\nfn main() -> Nat { 0 }\n";

/// A tempdir under this repository's root, so a register row can name a
/// fixture in it (only files inside the root match a row, D4).
///
/// `out/`, not `target/`: the devcontainer mounts the workspace read-only
/// apart from an allowlist, and `target/` is deliberately off it because the
/// host executes the guardrail binary from there (ADR 16.10.26p D1). `out/`
/// is the allowlisted output directory that `initialize.sh` always creates.
fn in_repository() -> tempfile::TempDir {
    let parent = repository_root().join("out");
    std::fs::create_dir_all(&parent).expect("out/ exists or is created");
    tempfile::Builder::new()
        .prefix("kernel-gate-drill-")
        .tempdir_in(parent)
        .expect("a tempdir under the repository")
}

/// `dir/name`'s register key.
fn key_of(dir: &Path, name: &str) -> String {
    let file = std::fs::canonicalize(dir).unwrap().join(name);
    let root = std::fs::canonicalize(repository_root()).unwrap();
    register_key(&file, &root).expect("the drill sits under the root")
}

/// A register over `main.tg` in `dir`: one row per `(theorem, cause, cites)`.
fn register_for(dir: &Path, rows: &[(&str, &str, &[&str])]) -> Register {
    let file = key_of(dir, "main.tg");
    let mut source = String::from("schema = 1\n");
    for (theorem, cause, cites) in rows {
        source.push_str(&format!(
            "[[exemption]]\nfile = \"{file}\"\ntheorem = \"{theorem}\"\ncause = \"{cause}\"\nremoved_by = \"drill\"\n"
        ));
        if !cites.is_empty() {
            let list: Vec<String> = cites.iter().map(|c| format!("\"{c}\"")).collect();
            source.push_str(&format!("cites = [{}]\n", list.join(", ")));
        }
    }
    Register::parse(&source).expect("a well-formed drill register")
}

/// Elaborate `main.tg` = `TWO` + `theorems` under `register`.
fn gated(
    dir: &Path,
    theorems: &str,
    register: Register,
) -> Result<ModuleTreeOutput, Vec<ElabError>> {
    let source = format!("{TWO}{theorems}");
    Register::inject(register, || run_tree(dir, &[("main.tg", &source)], None, 1))
}

/// The single E0084, which must name `theorem`.
fn violation_for(errors: &[ElabError], theorem: &str) -> String {
    let violations: Vec<&ElabError> = errors.iter().filter(|e| e.kind.code() == "E0084").collect();
    assert_eq!(violations.len(), 1, "{errors:?}");
    match &violations[0].kind {
        ElabErrorKind::KernelExemptionViolation { def, why } => {
            assert_eq!(def, theorem, "the violation names the theorem");
            why.clone()
        }
        other => panic!("expected E0084, got {other:?}"),
    }
}

/// The single E0081's theorem and cause.
fn refusal_of(errors: &[ElabError]) -> (String, String) {
    let refused: Vec<&ElabError> = errors.iter().filter(|e| e.kind.code() == "E0081").collect();
    assert_eq!(refused.len(), 1, "{errors:?}");
    match &refused[0].kind {
        ElabErrorKind::KernelRefusedProof { def, cause, .. } => (def.clone(), cause.clone()),
        other => panic!("expected E0081, got {other:?}"),
    }
}

// 2.10.26d AC4: a needs-delta row for a theorem the kernel admits.
#[test]
fn a_stale_exemption_fails_the_build() {
    let dir = in_repository();
    let register = register_for(dir.path(), &[("sound", "needs-delta", &[])]);
    let errors = gated(dir.path(), "theorem sound(): 2 == 2 { refl }\n", register)
        .err()
        .expect("a stale row fails the build");
    assert!(violation_for(&errors, "sound").contains("stale"));
}

// 2.10.26d AC4's drift and citation drills needed a real `needs-delta`
// refusal to exempt; ADR 2.10.26l removed the last one (the kernel unfolds
// `two`), so they were retired. `decide`'s drift and citation rules keep
// their pure tests in `elaborate/kernel/tests/verdict_tests.rs`.

// ADR 2.10.26l AC 4: δ's standing example is admitted by the build with no
// row, and its old `needs-delta` row is stale.
#[test]
fn the_delta_example_passes_the_build_with_no_row() {
    let dir = in_repository();
    let theorem = "theorem base(): two() == 2 { refl }\n";
    assert!(gated(dir.path(), theorem, Register::EMPTY).is_ok());
    let register = register_for(dir.path(), &[("base", "needs-delta", &[])]);
    let errors = gated(dir.path(), theorem, register)
        .err()
        .expect("the old row is stale");
    assert!(violation_for(&errors, "base").contains("stale"));
}

fn error_golden() -> PathBuf {
    repository_root().join("tests/golden/error/kernel_refuses_an_unregistered_theorem.tg")
}

// 2.10.26d AC4: the census still reaches a file whose theorem the gate refuses.
#[test]
fn the_census_window_reports_instead_of_gating() {
    use crate::doctor::checks::check_kernel_census::{run_census, Cause};
    let refused = crate::driver::elaborate_project(&error_golden(), false, 20, None);
    assert!(refused.is_err(), "outside a window the gate refuses");
    let window = crate::elaborate::termination::ReportingOnly::begin();
    let project = crate::driver::elaborate_project(&error_golden(), false, 20, None)
        .expect("inside the census's window the gate reports nothing");
    drop(window);
    let (_, rows) = run_census(&project, false);
    let row = rows
        .iter()
        .find(|row| row.name == "px_refl")
        .expect("the census prints its row");
    assert_eq!(row.primary, Cause::NamedTypeReference);
}

/// The single-file pipeline over `source` at `path`, with `cache` if given
/// (already holding `source`'s AST entry).
fn single_file(
    path: &Path,
    source: &str,
    cache: Option<&Mutex<BuildCache>>,
) -> Result<crate::elaborate::ElabOutput, Vec<ElabError>> {
    std::fs::write(path, source).unwrap();
    let (ast, errors) = crate::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    let build = crate::driver::pipeline::BuildCtx {
        cache,
        module_info: ModuleInfo::default(),
        source_map: SourceMap::single(path.to_path_buf(), source.to_string()),
    };
    crate::driver::pipeline::ir_cached_elab::elaborate_with_ir_cache(
        &ast,
        path,
        false,
        &build,
        &TraceOptions::default(),
    )
}

// 2.10.26d AC4: S1's local discharge gates a term-mode theorem on the
// single-file path — cold, and where an IR-cache entry for the same path and
// types hash exists, which must not be served (D1). That path defers no
// conversion, so the refusal is a cited `#[partial]` global, not δ.
#[test]
fn the_library_entry_gates_a_theorem() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.tg");
    let partial = "#[partial]\nfn spin() -> Nat { spin() }";
    let refused = format!("{partial}\ntheorem t(): spin() == spin() {{ refl }}");
    let cold = single_file(&path, &refused, None)
        .err()
        .expect("refused cold");
    let (theorem, cause) = refusal_of(&cold);
    assert_eq!(theorem, "t");
    assert!(cause.starts_with("global-not-admitted — "), "{cause}");

    // An entry written before this ADR: the same source, path and types hash,
    // its defs and no obligation. Elaboration never writes one now.
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let (ast, _) = crate::parse(&refused);
    let mut ctx = albite_core::Context::new();
    let collected = crate::elaborate::collect_definitions(&ast, &mut ctx).expect("collects");
    let types_hash = BuildCache::compute_types_hash(&collected.types_for_hash());
    let mut seeded = cache.lock().unwrap();
    seeded.put(&path, &refused, &ast).unwrap();
    seeded.put_ir(&path, types_hash, &[]).unwrap();
    assert!(
        seeded.get_ir(&path, &types_hash).is_some(),
        "the entry would hit"
    );
    drop(seeded);
    let warm = single_file(&path, &refused, Some(&cache))
        .err()
        .expect("an IR-cache entry does not hide the theorem");
    assert_eq!(refusal_of(&warm), refusal_of(&cold));
}

// 2.10.26d AC5: the same refusal on a second run over one cache directory.
#[test]
fn a_warm_check_reports_the_same_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let source = format!("{RECORD}\nfn main() -> Nat {{ 0 }}");
    let files = [("main.tg", source.as_str())];
    let first = run_tree(dir.path(), &files, Some(&cache), 1)
        .err()
        .expect("refused");
    let second = run_tree(dir.path(), &files, Some(&cache), 1)
        .err()
        .expect("refused again");
    assert_eq!(refusal_of(&first), refusal_of(&second));
    assert!(refusal_of(&first).1.contains("Malformed type"));
}

/// A record theorem the elaborator accepts and the kernel refuses: since ADR
/// 2.10.26l the kernel unfolds `two`, so these drills' refusal is a record.
/// Since 3.10.26p a record reaches the kernel by its encoding, so this one is
/// self-recursive — a cycle the presentation leaves named (D3).
const RECORD: &str = "type Point = { x: Nat, next: Link }\ntype Link = End | More(Point)\n\
                      fn px(p: Point) -> Nat { p.x }\n\
                      theorem t(p: Point): px(p) == px(p) { refl }";

const ROOT: &str = "mod k;\nmod p;\n\nfn main() -> Nat { 0 }";
const K_TOTAL: &str = "pub fn two() -> Nat { 2 }";

// 2.10.26d AC5 (D8): `k` declares no theorem, so the second run serves it
// from the signature tier and `two` is absent from the kernel's context; the
// spurious refusal earns the cold retry and the theorem is still admitted.
// With a refused theorem, both runs report the same class.
#[test]
fn a_warm_check_binds_a_global_from_a_cached_non_theorem_module() {
    let _registry = crate::comparator::requests::TEST_EXCLUSIVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let admitted = "use k::{two};\npub theorem t(): two() == two() { refl }";
    let files = [("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", admitted)];
    let cold = run_tree(dir.path(), &files, Some(&cache), 1).expect("admitted cold");
    assert!(!cold.conversion_retried);
    let warm = run_tree(dir.path(), &files, Some(&cache), 1).expect("admitted warm");
    assert!(warm.conversion_retried, "the signature hit unbound `two`");

    let refused = format!("use k::{{two}};\npub theorem u(): two() == 2 {{ refl }}\n{RECORD}");
    let files = [("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", &*refused)];
    let first = run_tree(dir.path(), &files, Some(&cache), 1)
        .err()
        .expect("refused");
    let second = run_tree(dir.path(), &files, Some(&cache), 1)
        .err()
        .expect("refused");
    assert_eq!(refusal_of(&first), refusal_of(&second));
}

/// One term-mode refusal per theorem: a theorem the termination gate already
/// rejected is not reported by the kernel too (D1).
#[test]
fn a_termination_rejection_is_not_reported_twice() {
    let source = "#[partial]\nfn wrapper() -> Nat { wrapper() }\n\
                  theorem wrapped(): wrapper() == wrapper() { refl }\n\
                  fn main() -> Nat { 0 }";
    let errors = run_fresh(&[("main.tg", source)], 1)
        .err()
        .expect("rejected");
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0063"),
        "{errors:?}"
    );
    assert!(
        errors.iter().all(|e| e.kind.code() != "E0081"),
        "{errors:?}"
    );
}

/// ADR 6.10.26d D1: a partial callee in a family argument sits, once the
/// family expands, in the statement's `Eq`, so the taint rule refuses it.
#[test]
fn a_partial_callee_in_a_family_argument_taints_the_statement() {
    let source = "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n\
                  #[partial]\nfn spin(n: Nat) -> Nat { spin(n) }\n\
                  pub theorem t(n: Nat, h: Le<spin(n), n>) -> Le<spin(n), n> { h }\n\
                  fn main() -> Nat { 0 }";
    let errors = run_fresh(&[("main.tg", source)], 1)
        .err()
        .expect("rejected");
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0063"),
        "{errors:?}"
    );
}
