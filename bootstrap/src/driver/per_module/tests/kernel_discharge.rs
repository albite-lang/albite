//! ADR 2.10.26c AC 6: the per-module walk carries tactic proofs to the
//! admission boundary, discharges them through the kernel, never caches
//! them, and retries cold when a signature hit unbound a citation.

use std::sync::Mutex;

use super::conversion_discharge::{file_of, run_fresh, run_tree};
use super::*;
use crate::cache::BuildCache;
use crate::driver::per_module::accumulator::ModuleTreeAccumulator;
use crate::driver::per_module::cache::stages_cache_entry;
use crate::elaborate::kernel::{KernelObligation, ObligationOrigin};
use crate::elaborate::{ElabErrorKind, ElabOutput};

const ROOT: &str = "mod k;\nmod p;\n\nfn main() -> Nat { 0 }";
/// A data `fn`, not a `theorem`: since ADR 2.10.26d a module that declares a
/// theorem is never cached (D8), and these drills need `k` cached. Since ADR
/// 13.10.26t an `Eq`-returning `fn` records an obligation too, so the proof
/// cites `two` through δ instead, which needs its body bound.
const LEMMA: &str = "pub fn two() -> Nat { 2 }";
const CITATION: &str = "use k::{two};\n\
    pub theorem by_tactic() -> Eq<Nat, two(), 2> by { refl }";

fn e0081s(errors: &[ElabError]) -> Vec<&ElabError> {
    errors.iter().filter(|e| e.kind.code() == "E0081").collect()
}

// ADR 2.10.26c AC6
#[test]
fn a_module_with_a_kernel_obligation_is_not_staged() {
    let mut output = ElabOutput::default();
    assert!(stages_cache_entry(&output));
    output.kernel_obligations.push(KernelObligation {
        def: "t".into(),
        span: crate::span::Span::new(0, 1),
        module: None,
        origin: ObligationOrigin::Tactics(tactic_trace()),
    });
    assert!(!stages_cache_entry(&output));

    // On disk: `k` is cached, the module holding the tactic proof is not.
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let files = [("main.tg", ROOT), ("k.tg", LEMMA), ("p.tg", CITATION)];
    run_tree(dir.path(), &files, Some(&cache), 1).expect("the citation is admitted");
    assert_eq!(elab_cache_entry_count(dir.path()), 2, "main and k only");
}

/// A trace the walk would record for `by { refl }`: built by elaborating one,
/// with the flag on so the obligation is left behind to take.
fn tactic_trace() -> crate::elaborate::tactics::TacticTrace {
    let (ast, _) = crate::parse("theorem t(n: Nat) -> Eq<Nat, n + 0, n> by { refl }");
    let mut ctx = albite_core::Context::new();
    let mut elaborator = crate::elaborate::Elaborator::new(&mut ctx);
    elaborator.defer_kernel = true;
    elaborator.elaborate_file(&ast).expect("a sound proof");
    let obligation = elaborator.kernel_obligations.pop().expect("one obligation");
    let ObligationOrigin::Tactics(trace) = obligation.origin else {
        panic!("a `by` block records a tactic obligation");
    };
    trace
}

/// A record and a function over it: a `refl` about `px` the elaborator
/// accepts and the kernel refuses, the record reaching it as a bare name.
/// The δ golden's `two_is_two` was this shape until ADR 2.10.26l. Since ADR
/// 3.10.26p a record reaches the kernel by its encoding, so this one is
/// self-recursive — a cycle the presentation leaves named (D3).
const POINT: &str = "type Point = { x: Nat, next: Link }\ntype Link = End | More(Point)\n\
                     fn px(p: Point) -> Nat { p.x }\n";

// ADR 2.10.26c AC6: an elaborator-accepted, kernel-refused tactic proof,
// through the real walk.
#[test]
fn the_walk_refuses_a_kernel_refused_tactic_proof() {
    let root = format!(
        "{POINT}theorem px_refl(p: Point): px(p) == px(p) by {{ refl }}\nfn main() -> Nat {{ 0 }}"
    );
    let errors = run_fresh(&[("main.tg", root.as_str())], 1)
        .err()
        .expect("the kernel refuses what the elaborator accepts");
    let refused = e0081s(&errors);
    assert_eq!(refused.len(), 1, "{errors:?}");
    // The malformed type is the theorem's own, so the refusal is the whole
    // block's; a refusal naming its step is `boundary_tests`' hand-built drill.
    assert!(matches!(
        &refused[0].kind,
        ElabErrorKind::KernelRefusedProof { def, cause, .. }
            if def == "px_refl" && cause.contains("Malformed type")
    ));
    assert_eq!(file_of(refused[0]), "main.tg");
}

/// D5: when a conversion obligation failed, the kernel discharge is skipped —
/// the run already fails, and a refusal over the same terms would be noise.
/// Each half alone is reported, so the skip is what removes the E0081.
#[test]
fn a_failed_conversion_skips_the_kernel_discharge() {
    let both = format!(
        "fn two() -> Nat {{ 2 }}\n{POINT}\
         theorem wrong(): two() == 3 {{ refl }}\n\
         theorem by_tactic(p: Point): px(p) == px(p) by {{ refl }}\n\
         fn main() -> Nat {{ 0 }}"
    );
    let errors = run_fresh(&[("main.tg", both.as_str())], 1)
        .err()
        .expect("the false δ proof fails");
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0071"),
        "{errors:?}"
    );
    assert!(e0081s(&errors).is_empty(), "{errors:?}");
}

/// A citation across modules links each obligation through its own file and
/// binds the lemma from the other — on the serial and the parallel walk.
#[test]
fn a_cross_module_tactic_proof_is_admitted() {
    for threads in [1, 4] {
        let result = run_fresh(
            &[("main.tg", ROOT), ("k.tg", LEMMA), ("p.tg", CITATION)],
            threads,
        );
        assert!(result.is_ok(), "threads={threads}: {:?}", result.err());
    }
}

// 7.10.26i AC2: a theorem stated at a closed numeral and proved by a lemma
// whose type is the closed sum (`Le<2, 2 + 3>`) passes the term-mode kernel
// gate — the kernel converts what the elaborator's nested fall-through
// accepted, so no acceptance is the elaborator's alone.
#[test]
fn a_closed_numeral_citation_is_kernel_admitted() {
    let source = "type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n\
        theorem le_add(n: Nat, m: Nat) -> Le<n, n + m> { (m, refl) }\n\
        theorem le_two_five() -> Le<2, 5> { le_add(2, 3) }\n\
        fn main() -> Nat { 0 }";
    let result = run_fresh(&[("main.tg", source)], 1);
    assert!(result.is_ok(), "{:?}", result.err());
}

/// A cited `#[partial]` lemma is refused by the kernel in the citing file.
#[test]
fn a_partial_citation_is_refused_in_the_citing_module() {
    let partial = "#[partial]\npub fn two() -> Nat { two() }";
    let errors = run_fresh(
        &[("main.tg", ROOT), ("k.tg", partial), ("p.tg", CITATION)],
        1,
    )
    .err()
    .expect("a partial citation is refused");
    assert!(
        errors
            .iter()
            .any(|e| e.kind.code() == "E0081" || e.kind.code() == "E0063"),
        "{errors:?}"
    );
}

// ADR 2.10.26c AC6 (D5): the second run serves `k` from the signature tier,
// whose definitions are absent; the kernel refusal earns the cold retry.
#[test]
fn a_warm_signature_hit_does_not_unbind_a_citation() {
    // The cold retry clears the global comparator registry, which a test
    // holding this lock relies on.
    let _registry = crate::comparator::requests::TEST_EXCLUSIVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let files = [("main.tg", ROOT), ("k.tg", LEMMA), ("p.tg", CITATION)];
    let cold = run_tree(dir.path(), &files, Some(&cache), 1).expect("the cold run is clean");
    assert!(!cold.conversion_retried);
    let warm = run_tree(dir.path(), &files, Some(&cache), 1)
        .unwrap_or_else(|errors| panic!("the warm run must pass too: {errors:?}"));
    assert!(
        warm.conversion_retried,
        "the citation was unbound until the retry"
    );
}

/// A diamond over `base`: `left` and `right` each declare it, so the walk
/// elaborates `base.tg` once per path (ADR 9.10.26p § 1.2).
fn diamond(base: &str) -> [(&'static str, String); 4] {
    let side = |name: &str| format!("mod base;\n\npub fn {name}() -> Nat {{ 0 }}");
    [
        (
            "main.tg",
            "mod left;\nmod right;\n\nfn main() -> Nat { 0 }".into(),
        ),
        ("left.tg", side("left_zero")),
        ("right.tg", side("right_zero")),
        ("base.tg", base.into()),
    ]
}

fn run_diamond(base: &str, threads: usize) -> Result<ModuleTreeOutput, Vec<ElabError>> {
    let files = diamond(base);
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(n, s)| (*n, s.as_str())).collect();
    run_fresh(&borrowed, threads)
}

/// ADR 9.10.26p AC 2. Red before D2: both copies of `base_refl` carry
/// `base.tg`, so the link found two and refused.
///
/// P0.2's finding, asserted here: the walk's two copies are equal — `ty` and
/// term both — so the kernel's verdicts cannot disagree on this shape, and the
/// two visits' obligations are `==` (one error per refusal in the twin below).
#[test]
fn a_library_reached_by_two_paths_is_checked_once() {
    let base = "pub theorem base_refl(n: Nat) -> Eq<Nat, n + 0, n> { refl }";
    for threads in [1, 4] {
        let output = run_diamond(base, threads)
            .unwrap_or_else(|errors| panic!("threads={threads}: {errors:?}"));
        let copies: Vec<_> = output
            .module_defs
            .iter()
            .flat_map(|(_, file, defs)| defs.iter().map(move |def| (file, def)))
            .filter(|(_, def)| def.name == "base_refl")
            .collect();
        assert_eq!(copies.len(), 2, "one copy per import path");
        assert_eq!(copies[0].0, copies[1].0, "both stamped with base.tg");
        assert_eq!(copies[0].1.ty, copies[1].1.ty);
        assert_eq!(
            format!("{:?}", copies[0].1.term),
            format!("{:?}", copies[1].1.term)
        );
    }
}

/// ADR 9.10.26p AC 2, the must-fail twin: a proof the elaborator accepts and
/// the kernel refuses, reached by two paths, is refused once — with the
/// kernel's cause, not the link's — and identically at 1 and 4 threads.
#[test]
fn a_kernel_refusal_through_two_paths_is_reported_once() {
    let base = "#[partial]\npub fn spin(n: Nat) -> Eq<Nat, n, n> { spin(n) }\n\
                pub theorem base_refl(n: Nat) -> Eq<Nat, n, n> by { exact spin(n) }";
    let rendered = |threads| -> Vec<String> {
        let errors = run_diamond(base, threads)
            .err()
            .expect("the kernel refuses");
        let refused = e0081s(&errors);
        assert_eq!(refused.len(), 1, "threads={threads}: {errors:?}");
        assert!(matches!(
            &refused[0].kind,
            ElabErrorKind::KernelRefusedProof { def, cause, .. }
                if def == "base_refl" && !cause.contains("could not be identified")
        ));
        assert_eq!(file_of(refused[0]), "base.tg");
        errors.iter().map(|e| format!("{:?}", e.kind)).collect()
    };
    assert_eq!(rendered(1), rendered(4));
}

/// A worker's kernel obligations reach the main accumulator.
#[test]
fn a_worker_merge_carries_kernel_obligations() {
    let obligation = KernelObligation {
        def: "t".into(),
        span: crate::span::Span::new(0, 1),
        module: None,
        origin: ObligationOrigin::Tactics(tactic_trace()),
    };
    let mut main_acc = ModuleTreeAccumulator::new();
    let mut worker = ModuleTreeAccumulator::new();
    worker.kernel_obligations.push(obligation.clone());
    main_acc.merge_worker(worker);
    let mut output = ElabOutput::default();
    output.kernel_obligations.push(obligation);
    main_acc.merge_output(output);
    assert_eq!(main_acc.kernel_obligations.len(), 2);
}
