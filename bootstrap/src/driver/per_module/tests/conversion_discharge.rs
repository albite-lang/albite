//! ADR 28.9.26d AC 5 / AC 6: the per-module driver defers, discharges, orders
//! and caches conversion obligations as specified.

use std::sync::Mutex;

use super::super::cache::stages_cache_entry;
use super::super::{cold_retry_notice, needs_cold_retry, ModuleTreeOutput};
use super::*;
use crate::cache::BuildCache;
use crate::elaborate::conversion::ConversionObligation;
use crate::elaborate::ElabOutput;

const K_TOTAL: &str = "pub fn two() -> Nat { 2 }";
const ROOT: &str = "mod k;\nmod p;\n\nfn main() -> Nat { 0 }";

/// A δ proof as an `Eq`-returning `fn`, not a `theorem`: since ADR 2.10.26d
/// the δ-free kernel checks every theorem, and these tests are about the
/// elaborator's conversion discharge, which a `fn` still exercises (P0.3).
fn proof_module(claim: u32) -> String {
    format!("use k::{{two}};\npub fn claim() -> two() == {claim} {{ refl }}")
}

/// Run the real tree elaboration over on-disk `files` in `dir`, with `threads`
/// pinned the way every fixture here pins it (see `elaborate_tree_errors`).
/// Shared with `kernel_discharge.rs` (ADR 2.10.26c), which drives the same walk.
pub(super) fn run_tree(
    dir: &std::path::Path,
    files: &[(&str, &str)],
    cache: Option<&Mutex<BuildCache>>,
    threads: usize,
) -> Result<ModuleTreeOutput, Vec<ElabError>> {
    let _guard = super::super::cache::equivalence::ELAB_THREADS_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("ALBITE_ELAB_THREADS", threads.to_string());
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let main_path = dir.join(files[0].0);
    let prepared = prepare_project(&main_path, false, cache).expect("fixture must parse");
    let build = pipeline::BuildCtx {
        cache,
        module_info: prepared.module_info,
        source_map: prepared.source_map,
    };
    let result = elaborate_module_tree(
        &prepared.module_tree,
        &main_path,
        false,
        &build,
        &TraceOptions::default(),
    );
    std::env::remove_var("ALBITE_ELAB_THREADS");
    result
}

pub(super) fn run_fresh(
    files: &[(&str, &str)],
    threads: usize,
) -> Result<ModuleTreeOutput, Vec<ElabError>> {
    let dir = tempfile::tempdir().unwrap();
    run_tree(dir.path(), files, None, threads)
}

fn e0071s(errors: &[ElabError]) -> Vec<&ElabError> {
    errors.iter().filter(|e| e.kind.code() == "E0071").collect()
}

pub(super) fn file_of(error: &ElabError) -> String {
    let path = error.file_path.as_ref().expect("the error is file-stamped");
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Defs, warnings and errors rendered, so allocation order cannot differ.
fn rendered(output: &ModuleTreeOutput) -> Vec<String> {
    let mut lines: Vec<String> = output
        .elab
        .defs
        .iter()
        .map(|d| format!("{} : {} = {}", d.name, d.ty, d.term.term))
        .collect();
    lines.sort();
    lines.extend(output.elab.warnings.iter().map(|w| w.message.clone()));
    lines
}

#[test]
fn a_cross_module_proof_over_a_total_function_is_accepted() {
    let proof = proof_module(2);
    let output = run_fresh(&[("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", &proof)], 1)
        .unwrap_or_else(|errors| panic!("δ over a total global must discharge: {errors:?}"));
    assert!(
        !output.conversion_retried,
        "a cold run has nothing to retry"
    );
}

#[test]
fn the_same_proof_over_a_partial_function_fails_in_the_proving_module() {
    let partial = "#[partial]\npub fn two() -> Nat { 2 }";
    let proof = proof_module(2);
    let errors = run_fresh(&[("main.tg", ROOT), ("k.tg", partial), ("p.tg", &proof)], 1)
        .err()
        .expect("a partial global stays folded");
    let refl = e0071s(&errors);
    assert_eq!(refl.len(), 1, "{errors:?}");
    assert_eq!(file_of(refl[0]), "p.tg");
}

// 30.9.26e AC6: an imported lemma cited at a wrong instance goes through D5's
// `Eq` retry and is still refused with its cross-file note and trace frames.
#[test]
fn a_cross_module_wrong_citation_keeps_its_cross_file_note() {
    let lemma = "pub fn add_zero(n: Nat) -> Eq<Nat, n + 0, n> { refl }";
    let cite = "use k::{add_zero};\n\
                pub fn at(n: Nat) -> Eq<Nat, n + 1, n> { add_zero(n) }";
    let errors = run_fresh(&[("main.tg", ROOT), ("k.tg", lemma), ("p.tg", cite)], 1)
        .err()
        .expect("`n + 0` does not convert to `n + 1`");
    let mismatch = errors
        .iter()
        .find(|e| e.kind.code() == "E0010")
        .unwrap_or_else(|| panic!("no E0010: {errors:?}"));
    assert!(mismatch
        .notes
        .iter()
        .any(|n| n.message == "return type declared in `add_zero`" && n.file_path.is_some()));
    let frames: Vec<&str> = mismatch.trace.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert!(
        frames[0] == "add_zero call (expected Eq Nat (n + (succ zero)) n)",
        "{frames:?}"
    );
    assert_eq!(frames[1], "return type declared in `add_zero`");
}

#[test]
fn two_failing_obligations_report_in_source_order() {
    let proofs = "use k::{two};\n\
                  pub theorem first(): two() == 3 { refl }\n\
                  pub theorem second(): two() == 4 { refl }";
    let errors = run_fresh(&[("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", proofs)], 1)
        .err()
        .expect("both proofs are false");
    let refl = e0071s(&errors);
    assert_eq!(refl.len(), 2);
    assert!(refl[0].span.start < refl[1].span.start, "source order");
}

// ADR 4.10.26e AC5 (D6): a false δ proof is reported beside an elaboration
// error — in its own module, and in a clean module beside a failing one —
// on both walks, while a proof citing the broken definition adds nothing.
#[test]
fn a_deferred_refusal_is_reported_beside_an_elaboration_error() {
    let broken = "fn broken() -> Nat { true }";
    let same_module = format!("{}\n{broken}", proof_module(3));
    for threads in [1, 4] {
        for files in [
            vec![
                ("main.tg", ROOT),
                ("k.tg", K_TOTAL),
                ("p.tg", same_module.as_str()),
            ],
            vec![
                (
                    "main.tg",
                    "mod k;\nmod p;\nmod q;\n\nfn main() -> Nat { 0 }",
                ),
                ("k.tg", K_TOTAL),
                ("p.tg", &proof_module(3)),
                ("q.tg", broken),
            ],
        ] {
            let errors = run_fresh(&files, threads)
                .err()
                .expect("both faults fail the run");
            let codes: Vec<&str> = errors.iter().map(|e| e.kind.code()).collect();
            assert_eq!(codes, ["E0010", "E0071"], "{threads}: {errors:?}");
            assert_eq!(file_of(&errors[1]), "p.tg");
        }
    }
    // The cascade filter: a proof whose δ needs the definition that failed
    // is not reported a second time.
    let cites_broken = "pub fn broken() -> Nat { true }\n\
                        pub fn claim() -> broken() == 2 { refl }";
    let errors = run_fresh(
        &[("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", cites_broken)],
        1,
    )
    .err()
    .expect("the broken definition fails");
    let codes: Vec<&str> = errors.iter().map(|e| e.kind.code()).collect();
    assert_eq!(codes, ["E0010"], "{errors:?}");
}

#[test]
fn nothing_is_lost_on_the_parallel_walk() {
    // D15: the discharge's debug assertion checks the count; the E0071 proves
    // the obligation itself survived the worker merge.
    let proof = proof_module(3);
    let errors = run_fresh(&[("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", &proof)], 4)
        .err()
        .expect("a false δ proof in a non-root module still fails");
    let refl = e0071s(&errors);
    assert_eq!(refl.len(), 1, "{errors:?}");
    assert_eq!(file_of(refl[0]), "p.tg");
}

#[test]
fn only_an_output_without_obligations_is_staged() {
    let mut output = ElabOutput::default();
    assert!(stages_cache_entry(&output));
    output.conversion_obligations.push(ConversionObligation {
        left: albite_core::Term::Global("two".into()),
        right: albite_core::Term::Zero,
        ty: Type::Nat,
        error: ElabError::invalid_refl(
            crate::span::Span::new(0, 0),
            albite_core::Term::Zero,
            albite_core::Term::Zero,
        ),
    });
    assert!(!stages_cache_entry(&output));
}

#[test]
fn the_cold_retry_needs_a_failure_and_a_hit() {
    assert!(needs_cold_retry(1, 1));
    assert!(!needs_cold_retry(0, 1));
    assert!(!needs_cold_retry(1, 0));
    assert!(!needs_cold_retry(0, 0));
    assert_eq!(
        cold_retry_notice(2, 3),
        "conversion: retrying without signature-cache hits (2 failed obligation(s), 3 hit module(s))"
    );
}

#[test]
fn a_warm_run_sees_an_edited_body() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let proof = proof_module(2);
    let cold = run_tree(
        dir.path(),
        &[("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", &proof)],
        Some(&cache),
        1,
    );
    assert!(cold.is_ok(), "{:?}", cold.err());

    // Same signature, different body: the proof is now false.
    let edited = "pub fn two() -> Nat { 3 }";
    let warm = run_tree(
        dir.path(),
        &[("main.tg", ROOT), ("k.tg", edited), ("p.tg", &proof)],
        Some(&cache),
        1,
    );
    let errors = warm
        .err()
        .expect("the warm run must not serve a stale discharge");
    assert_eq!(file_of(e0071s(&errors)[0]), "p.tg");
}

#[test]
fn a_warm_run_retries_cold_and_matches_a_cold_run() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let proof = proof_module(2);
    let files = [
        ("main.tg", ROOT),
        ("k.tg", K_TOTAL),
        ("p.tg", proof.as_str()),
    ];
    let cold = run_tree(dir.path(), &files, Some(&cache), 1).expect("cold run is clean");
    assert!(
        elab_cache_entry_count(dir.path()) > 0,
        "the cold run seeded the cache"
    );

    // `k` is now a signature hit, so `two`'s body is absent from the first pass.
    let warm = run_tree(dir.path(), &files, Some(&cache), 1)
        .unwrap_or_else(|errors| panic!("the retry must accept the proof: {errors:?}"));
    assert!(warm.conversion_retried);
    let cold_rendered = rendered(&cold);
    assert!(
        cold_rendered.iter().any(|line| line.starts_with("two : ")),
        "the comparison must see the defs, not two empty lists: {cold_rendered:?}"
    );
    assert_eq!(rendered(&warm), cold_rendered);
}

#[test]
fn a_module_with_an_obligation_is_never_cached() {
    for claim in [2, 3] {
        let dir = tempfile::tempdir().unwrap();
        let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
        let proof = proof_module(claim);
        let _ = run_tree(
            dir.path(),
            &[("main.tg", ROOT), ("k.tg", K_TOTAL), ("p.tg", &proof)],
            Some(&cache),
            1,
        );
        assert_eq!(
            elab_cache_entry_count(dir.path()),
            2,
            "main and k are cached, p (claim {claim}) is not"
        );
    }
}

#[test]
fn expect_error_still_observes_an_undischarged_refl() {
    // D13: inside `expect_error` conversion is δ-free, so the operand's E0071
    // is observed rather than deferred into "elaborated successfully".
    let root = "fn two() -> Nat { 2 }\n\
                fn probe() -> Unit { expect_error((refl : Eq<Nat, two(), 3>), \"E0071\") }\n\
                fn main() -> Nat { 0 }";
    let result = run_fresh(&[("main.tg", root)], 1);
    assert!(result.is_ok(), "{:?}", result.err());
}

/// D15: a worker's obligations, their count and its signature hits all add
/// into the main accumulator — nothing a worker deferred is lost at the merge.
#[test]
fn a_worker_merge_carries_obligations_and_both_counters() {
    use super::super::accumulator::ModuleTreeAccumulator;
    let obligation = |claim| ConversionObligation {
        left: albite_core::Term::Global("two".into()),
        right: albite_core::Term::nat(claim),
        ty: Type::Nat,
        error: ElabError::invalid_refl(
            crate::span::Span::new(0, 0),
            albite_core::Term::Zero,
            albite_core::Term::Zero,
        ),
    };
    let mut main_acc = ModuleTreeAccumulator::new();
    main_acc.conversion_obligations.push(obligation(3));
    main_acc.conversion_deferred = 1;
    main_acc.signature_hit_modules = 2;
    let mut worker = ModuleTreeAccumulator::new();
    worker.conversion_obligations.push(obligation(4));
    worker.conversion_obligations.push(obligation(5));
    worker.conversion_deferred = 2;
    worker.signature_hit_modules = 3;
    main_acc.merge_worker(worker);
    assert_eq!(main_acc.conversion_obligations.len(), 3);
    assert_eq!(main_acc.conversion_deferred, 3);
    assert_eq!(main_acc.signature_hit_modules, 5);
}
