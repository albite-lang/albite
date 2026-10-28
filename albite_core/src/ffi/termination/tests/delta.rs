//! The self-host's δ deferral and discharge, driven through the externs (ADR
//! 7.10.26u AC 2).
//!
//! Each test runs one program the way `termination_check_program` and
//! `check_refl` do: reset, defer from a `refl`, declare and stream the
//! definitions, plan, stream again, check, discharge. The admission is real —
//! the shared engine's — and the terms are built in the arena.

use std::ffi::CStr;

use super::*;
use crate::ffi::termination::delta::{
    tg_conversion_defer, tg_conversion_deferral_set, tg_conversion_discharge,
    tg_conversion_failure_render,
};
use crate::ffi::terms::core_data::tg_term_nat_lit;
use crate::ffi::types::constructors::tg_type_nat;

/// `global:<name>`, as the self-host elaborates a nullary call.
fn global(name: &str) -> TermHandle {
    unsafe { tg_term_global(cstr(name)) }
}

/// Defer `<name>() == <expected>` from a `refl` at bytes 10..14 of `a.tg`.
fn defer_against(name: &str, expected: u64) -> bool {
    unsafe {
        tg_conversion_defer(
            global(name),
            tg_term_nat_lit(expected),
            10,
            14,
            cstr("a.tg"),
        )
    }
}

/// Stream `defs` (name, body) through both phases and check; returns what
/// `plan` said it would retain. `partial` names the `#[partial]` ones.
fn admit(defs: &[(&str, TermHandle)], partial: &[&str]) -> u64 {
    unsafe {
        for name in partial {
            tg_termination_note_item(cstr(name), true, std::ptr::null(), false);
        }
        for (name, _) in defs {
            tg_termination_declare(cstr(name));
        }
        for (name, body) in defs {
            assert!(tg_termination_add_def(cstr(name), tg_type_nat(), *body));
        }
    }
    let planned = tg_termination_plan(true);
    for (name, body) in defs {
        assert!(unsafe { tg_termination_add_def(cstr(name), tg_type_nat(), *body) });
    }
    tg_termination_check();
    planned
}

/// The `index`th discharge failure's wire fields.
fn failure(index: u64) -> Vec<String> {
    let ptr = tg_conversion_failure_render(index);
    assert!(!ptr.is_null(), "failure {index} did not render");
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().expect("utf-8");
    text.splitn(4, '\n').map(str::to_string).collect()
}

/// 7.10.26u AC 2: `two() == 2` with `two` admitted `Total` and its body in the
/// closure is discharged.
#[test]
fn a_total_nullary_unfolds_and_discharges() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("two", 2), "a global side defers");
    let planned = admit(&[("two", unsafe { tg_term_nat_lit(2) })], &[]);
    assert_eq!(
        planned, 1,
        "the closure adds `two` to an empty descent plan"
    );
    assert_eq!(tg_conversion_discharge(), 0);
    assert!(tg_conversion_failure_render(0).is_null());
}

/// 7.10.26u AC 2 must-fail twin: with `two` `Partial`, the same obligation fails.
#[test]
fn a_partial_nullary_stays_folded_and_fails() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("two", 2));
    admit(&[("two", unsafe { tg_term_nat_lit(2) })], &["two"]);
    assert_eq!(tg_conversion_discharge(), 1);
    let fields = failure(0);
    assert_eq!(
        fields[..3],
        ["10", "14", "a.tg"],
        "located where `refl` was written"
    );
}

/// 7.10.26u AC 2 must-fail twin: `two() == 3` fails, and the note names what
/// `two` unfolded to.
#[test]
fn a_false_equation_fails_with_the_unfolded_sides() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("two", 3));
    admit(&[("two", unsafe { tg_term_nat_lit(2) })], &[]);
    assert_eq!(tg_conversion_discharge(), 1);
    let note = &failure(0)[3];
    assert!(
        note.contains("after unfolding total definitions") && note.contains("(succ (succ zero))"),
        "{note}"
    );
}

/// 7.10.26u AC 2, route (a): a definition outside every obligation's closure
/// is not retained, and one reached through a closure member is.
#[test]
fn only_the_obligations_closure_is_retained() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("outer", 2));
    let planned = admit(
        &[
            ("outer", global("two")),
            ("two", unsafe { tg_term_nat_lit(2) }),
            ("unrelated", unsafe { tg_term_nat_lit(7) }),
        ],
        &[],
    );
    assert_eq!(planned, 2);
    let kept = with_registry(|registry| registry.delta_bodies_for_test());
    assert_eq!(kept, ["outer", "two"].map(String::from).into());
    assert_eq!(
        tg_conversion_discharge(),
        0,
        "`outer` unfolds through `two`"
    );
}

/// With no obligation, the closure is empty: nothing more is retained.
#[test]
fn no_obligation_retains_nothing_more() {
    tg_init();
    tg_termination_reset();
    let planned = admit(&[("two", unsafe { tg_term_nat_lit(2) })], &[]);
    assert_eq!(planned, 0);
    assert!(with_registry(|registry| registry.delta_bodies_for_test()).is_empty());
    assert_eq!(tg_conversion_discharge(), 0);
}

/// §2.1: no global on either side means δ cannot change the answer, so the
/// failure is final; and an equal pair is not an obligation.
#[test]
fn only_a_global_side_defers() {
    tg_init();
    tg_termination_reset();
    let closed =
        unsafe { tg_conversion_defer(tg_term_nat_lit(2), tg_term_nat_lit(3), 0, 1, cstr("a.tg")) };
    assert!(!closed, "no global: refused now");
    let equal =
        unsafe { tg_conversion_defer(tg_term_nat_lit(2), tg_term_nat_lit(2), 0, 1, cstr("a.tg")) };
    assert!(!equal, "already equal: nothing to defer");
    let null_file =
        unsafe { tg_conversion_defer(global("two"), tg_term_nat_lit(2), 0, 1, std::ptr::null()) };
    assert!(!null_file, "an unreadable file refuses rather than records");
}

/// D13 mirror and D3: deferral is off before a program opens, while switched
/// off, and after the program's discharge — each refuses rather than records.
#[test]
fn deferral_is_off_outside_an_open_program() {
    tg_init();
    // Open a program first, so closing it is what makes the next defer refuse.
    tg_termination_reset();
    with_ledger_closed();
    assert!(!defer_against("two", 2), "no program open");
    tg_termination_reset();
    assert!(
        tg_conversion_deferral_set(false),
        "reset turned deferral on"
    );
    assert!(
        !defer_against("two", 2),
        "switched off, as inside expect_error"
    );
    assert!(
        !tg_conversion_deferral_set(true),
        "and the previous setting comes back"
    );
    assert!(defer_against("two", 2));
    tg_conversion_discharge();
    assert!(!defer_against("two", 2), "the discharge closes the program");
}

/// A program whose streams never reached `check` admitted nothing, so an
/// obligation that needs δ fails rather than passing unchecked.
#[test]
fn an_unchecked_program_admits_nothing() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("two", 2));
    assert_eq!(tg_conversion_discharge(), 1);
}

/// 13.10.26m AC6: `x::two` is 2 and `y::two` is 3, both elaborated as `two`.
/// The last writer's body would make `two() == 3` hold for `x::two`, so a
/// name offered twice keeps no body, and the obligation fails naming why.
#[test]
fn a_name_offered_twice_is_never_unfolded() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("two", 3));
    admit(
        &[
            ("two", unsafe { tg_term_nat_lit(2) }),
            ("two", unsafe { tg_term_nat_lit(3) }),
        ],
        &[],
    );
    assert!(
        with_registry(|registry| registry.delta_bodies_for_test()).is_empty(),
        "no body is kept for a repeated name"
    );
    assert_eq!(tg_conversion_discharge(), 1, "the false proof is refused");
    let note = &failure(0)[3];
    assert_eq!(
        note,
        "`two` is defined in more than one module; tungsten1 cannot yet unfold it"
    );
}

/// 13.10.26m AC6: a repeated name reached only through another definition's
/// body is opaque too, and the note names it rather than the obligation's own.
#[test]
fn a_repeated_name_behind_a_unique_one_is_never_unfolded() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("outer", 3));
    admit(
        &[
            ("outer", global("two")),
            ("two", unsafe { tg_term_nat_lit(2) }),
            ("two", unsafe { tg_term_nat_lit(3) }),
        ],
        &[],
    );
    assert_eq!(
        with_registry(|registry| registry.delta_bodies_for_test()),
        ["outer"].map(String::from).into()
    );
    assert_eq!(tg_conversion_discharge(), 1);
    assert!(failure(0)[3].starts_with("`two` is defined in more than one module"));
}

/// 13.10.26m AC6: a name offered once still discharges beside a repeated one,
/// and a failure that reaches no repeated name keeps its own note.
#[test]
fn a_unique_name_still_unfolds_beside_a_repeated_one() {
    tg_init();
    tg_termination_reset();
    assert!(defer_against("three", 3));
    assert!(defer_against("four", 5));
    admit(
        &[
            ("two", unsafe { tg_term_nat_lit(2) }),
            ("two", unsafe { tg_term_nat_lit(3) }),
            ("three", unsafe { tg_term_nat_lit(3) }),
            ("four", unsafe { tg_term_nat_lit(4) }),
        ],
        &[],
    );
    assert_eq!(tg_conversion_discharge(), 1, "only `four() == 5` fails");
    let fields = failure(0);
    assert!(
        fields[3].contains("after unfolding total definitions"),
        "{}",
        fields[3]
    );
}

/// 13.10.26m AC6: `reset` forgets the previous program's repeats.
#[test]
fn reset_clears_the_repeated_names() {
    tg_init();
    tg_termination_reset();
    admit(
        &[
            ("two", unsafe { tg_term_nat_lit(2) }),
            ("two", unsafe { tg_term_nat_lit(3) }),
        ],
        &[],
    );
    assert_eq!(
        with_registry(|registry| registry.delta_repeated_for_test()),
        ["two"].map(String::from).into()
    );
    tg_termination_reset();
    assert!(with_registry(|registry| registry.delta_repeated_for_test()).is_empty());
    assert!(defer_against("two", 2));
    admit(&[("two", unsafe { tg_term_nat_lit(2) })], &[]);
    assert_eq!(
        tg_conversion_discharge(),
        0,
        "a fresh program unfolds `two` again"
    );
}

/// Stream a `#[partial]` and a total `f` in `order`, then a theorem citing
/// `f`; returns the termination rejection count.
fn theorem_over_repeated_f(partial_first: bool) -> u64 {
    tg_init();
    tg_termination_reset();
    let looping = || unsafe { tg_term_global(cstr("f")) };
    let total = || unsafe { tg_term_nat_lit(1) };
    let bodies = if partial_first {
        [(true, looping()), (false, total())]
    } else {
        [(false, total()), (true, looping())]
    };
    unsafe {
        tg_termination_declare(cstr("f"));
        tg_termination_declare(cstr("thm"));
        for (partial, _) in &bodies {
            tg_termination_note_item(cstr("f"), *partial, std::ptr::null(), false);
        }
        tg_termination_note_item(cstr("thm"), false, std::ptr::null(), true);
    }
    let stream = || {
        for (_, body) in &bodies {
            assert!(unsafe { tg_termination_add_def(cstr("f"), tg_type_nat(), *body) });
        }
        assert!(unsafe { tg_termination_add_def(cstr("thm"), tg_type_nat(), global("f")) });
    };
    stream();
    tg_termination_plan(true);
    stream();
    tg_termination_check()
}

/// 13.10.26m P0.5: `notes` is last-writer-wins, so a total `y::f` noted after
/// a `#[partial]` `x::f` cleared the taint, and a theorem citing `x::f` was
/// admitted. A repeated name now carries taint in both orders.
#[test]
fn a_repeated_name_taints_a_proof_in_either_order() {
    assert_eq!(theorem_over_repeated_f(true), 1, "partial noted first");
    assert_eq!(theorem_over_repeated_f(false), 1, "partial noted last");
}

/// Close the ledger as a thread that never opened a program would see it.
fn with_ledger_closed() {
    crate::ffi::termination::delta::with_ledger(|ledger| *ledger = Default::default());
}
