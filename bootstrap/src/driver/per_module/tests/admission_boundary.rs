//! ADR 11.8.26b §2.3: synthesized comparators are inside the trusted boundary.
//!
//! The property under test is an *ordering*, and ordering is what a test can
//! actually pin down here: `synthesize_comparators` must put its output into the
//! same `defs` that `admit_or_reject` then checks. Before 11.8.26b the
//! synthesizer ran in `driver::run_project_pipeline`, after the gate had already
//! returned, so a comparator could never be rejected however it was built.

use crate::comparator::{mangling::comparator_symbol, requests};
use crate::driver::per_module::accumulator::ModuleTreeAccumulator;
use crate::driver::per_module::synthesis_notice;
use crate::elaborate::kernel::KernelMode;
use crate::elaborate::{CoreDef, DefTerminationMeta};
use crate::span::Span;
use albite_core::terms::SpannedTerm;
use albite_core::{Term, Type};

/// A definition whose body calls `compare_T`, as `__compare` lowering emits.
fn caller_of(name: &str, ty: &Type) -> CoreDef {
    CoreDef {
        name: name.to_string(),
        ty: Type::Unit,
        term: SpannedTerm::generated(Term::App(
            Box::new(Term::Global(comparator_symbol(ty))),
            Box::new(Term::Unit),
        )),
        span: Span::new(0, 0),
    }
}

/// An accumulator holding one caller that references `Nat`'s comparator.
fn accumulator_needing_a_comparator() -> ModuleTreeAccumulator {
    requests::clear();
    requests::register(comparator_symbol(&Type::Nat), Type::Nat);
    let mut acc = ModuleTreeAccumulator::new();
    acc.defs.push(caller_of("uses_compare", &Type::Nat));
    acc
}

#[test]
fn synthesized_comparators_are_in_the_set_the_gate_checks() {
    let _guard = requests::TEST_EXCLUSIVE.lock().unwrap();
    let mut acc = accumulator_needing_a_comparator();
    let wanted = comparator_symbol(&Type::Nat);

    // Both polarities: absent before, present after. Asserting only the second
    // would pass against an accumulator that had been pre-populated some other
    // way, which is precisely the pre-11.8.26b arrangement.
    assert!(
        !acc.defs.iter().any(|def| def.name == wanted),
        "nothing defines `{wanted}` yet"
    );
    let count = acc.synthesize_comparators();
    assert_eq!(count, 1, "one comparator was referenced and undefined");
    assert!(
        acc.defs.iter().any(|def| def.name == wanted),
        "`{wanted}` must be in `defs` before `admit_or_reject` reads it"
    );

    // And the gate does read it: it certifies the whole set without complaint,
    // which it could not do if the synthesized def were malformed or absent.
    acc.admit_or_reject(false, KernelMode::Gate)
        .expect("synthesized comparators are structurally recursive and admit");
}

/// An accumulator whose synthesized `Nat` comparator is annotated `#[partial]`
/// in a proof role — a set the gate must reject at the default level.
fn accumulator_with_a_partial_proof_comparator() -> ModuleTreeAccumulator {
    let mut acc = accumulator_needing_a_comparator();
    acc.synthesize_comparators();
    let wanted = comparator_symbol(&Type::Nat);
    acc.termination_meta.insert(
        wanted,
        DefTerminationMeta {
            attrs: crate::ast::TerminationAttrs {
                partial: true,
                decreasing: None,
            },
            is_proof: true,
        },
    );
    acc
}

#[test]
fn a_synthesized_comparator_is_subject_to_the_gate_like_any_other_definition() {
    let _guard = requests::TEST_EXCLUSIVE.lock().unwrap();
    // Annotating the synthesized name `#[partial]` has to change the verdict.
    // If it does, the gate is genuinely reading that definition; if the gate
    // still reported a clean set, the def would be decoration.
    let mut acc = accumulator_with_a_partial_proof_comparator();
    assert!(
        acc.admit_or_reject(false, KernelMode::Gate).is_err(),
        "a `#[partial]` proof-role comparator must be rejected, proving the gate sees it"
    );
}

/// ADR 2.10.26b AC2: the parked 1.10.26e race, forced rather than hoped for.
///
/// Another thread holds a `ReportingOnly` window — as a census test does —
/// while this one runs the gate. Two barriers pin the gate's read inside the
/// window; the holder closes it only after the second.
#[test]
fn the_gate_rejects_while_another_thread_holds_a_reporting_window() {
    use std::sync::{Arc, Barrier};
    let _guard = requests::TEST_EXCLUSIVE.lock().unwrap();
    let mut acc = accumulator_with_a_partial_proof_comparator();
    let window_open = Arc::new(Barrier::new(2));
    let gate_done = Arc::new(Barrier::new(2));
    let holder = {
        let (window_open, gate_done) = (Arc::clone(&window_open), Arc::clone(&gate_done));
        std::thread::spawn(move || {
            let window = crate::elaborate::termination::ReportingOnly::begin();
            window_open.wait();
            gate_done.wait();
            drop(window);
        })
    };
    window_open.wait();
    let verdict = acc.admit_or_reject(false, KernelMode::Gate);
    gate_done.wait();
    holder.join().expect("the window holder does not panic");
    assert!(
        verdict.is_err(),
        "another thread's reporting window must not weaken this thread's gate"
    );
}

/// The verbose notice speaks only when there is both a listener and something
/// to say — all four quadrants, because the guard is a conjunction and a test
/// of one quadrant leaves the other operand free.
#[test]
fn the_synthesis_notice_needs_both_verbosity_and_a_nonzero_count() {
    assert_eq!(
        synthesis_notice(true, 2).as_deref(),
        Some("Synthesized 2 comparator(s)")
    );
    assert_eq!(synthesis_notice(true, 0), None, "nothing to report");
    assert_eq!(synthesis_notice(false, 2), None, "nobody listening");
    assert_eq!(synthesis_notice(false, 0), None);
}

/// The boundary between "nothing to say" and "something to say" is at 1, not 0
/// or 2 — pins `>` against `>=` and `==`, which a 0-vs-2 test cannot separate.
#[test]
fn the_synthesis_notice_reports_a_single_comparator() {
    assert_eq!(
        synthesis_notice(true, 1).as_deref(),
        Some("Synthesized 1 comparator(s)")
    );
}
