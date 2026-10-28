//! The holes seam's tests (ADR 16.10.26g AC3).
//!
//! These drive the **externs**: §1.1's fixtures are rebuilt through the FFI the
//! self-host calls — C strings, arena handles, the kind codes — and the three
//! readers must give the counts the bootstrap's `check` line prints for each.
//! Each test resets the arena and the registry inline (ADR 18.8.26b: libtest
//! gives each test its own thread, and a `fresh()` helper is an extraction no
//! assertion can tell a mutation of).

use std::ffi::{c_char, CString};

use super::*;
use crate::ffi::terms::core::{tg_term_app, tg_term_global};
use crate::ffi::terms::ext::tg_term_sorry;
use crate::ffi::types::constructors::tg_type_nat;
use crate::ffi::{tg_init, INVALID_HANDLE};
use crate::terms::analysis::Construct;

const PROGRAM: u64 = 0;
const PROOF: u64 = 1;
const AXIOM: u64 = 2;

/// A leaked null-terminated C string, as every caller of this seam supplies.
fn cstr(text: &str) -> *const c_char {
    CString::new(text)
        .expect("no interior nul")
        .into_raw()
        .cast_const()
}

/// The arena's last error message, as the caller reads it.
fn last_error() -> String {
    crate::ffi::ARENA.with(|cell| cell.borrow().last_error.clone())
}

/// An axiom's self-host body: a bare `Sorry`, no span.
fn bare_sorry() -> TermHandle {
    tg_term_sorry(tg_type_nat())
}

/// A body that calls `callee` with a dummy argument.
fn calls(callee: &str) -> TermHandle {
    unsafe { tg_term_app(tg_term_global(cstr(callee)), bare_unit()) }
}

fn bare_unit() -> TermHandle {
    crate::ffi::terms::core_data::tg_term_zero()
}

fn note(name: &str, kind: u64) {
    assert!(
        unsafe { tg_holes_note_item(cstr(name), kind) },
        "note {name}"
    );
}

fn add(name: &str, term: TermHandle) {
    assert!(unsafe { tg_holes_add_def(cstr(name), term) }, "add {name}");
}

/// The three numbers the `check` line prints.
fn counts() -> (u64, u64, u64) {
    (
        tg_holes_authored(),
        tg_holes_not_authored(),
        tg_holes_resting(),
    )
}

// AC3: `ax.tg` — `axiom ax(): Void`, `theorem bad2(): Void { ax() }`.
#[test]
fn one_hop_through_an_axiom_rests() {
    tg_init();
    tg_holes_reset();
    note("ax", AXIOM);
    note("bad2", PROOF);
    add("ax", bare_sorry());
    add("bad2", calls("ax"));
    assert_eq!(counts(), (1, 0, 1));
}

// AC3: `both.tg` — two hops, through a program definition, and a clean pair.
#[test]
fn both_counts_two_hops_and_through_a_function() {
    tg_init();
    tg_holes_reset();
    for (name, kind) in [
        ("ax", AXIOM),
        ("bad2", PROOF),
        ("bad3", PROOF),
        ("via_fn", PROGRAM),
        ("bad4", PROOF),
        ("fine", PROOF),
        ("uses_fine", PROOF),
    ] {
        note(name, kind);
    }
    add("ax", bare_sorry());
    add("bad2", calls("ax"));
    add("bad3", calls("bad2"));
    add("via_fn", calls("ax"));
    add("bad4", calls("via_fn"));
    add("fine", bare_unit());
    add("uses_fine", calls("fine"));
    assert_eq!(counts(), (1, 0, 3));
}

// AC3: a citation added before the axiom it names is still followed.
#[test]
fn a_name_added_before_its_callee_still_rests() {
    tg_init();
    tg_holes_reset();
    note("bad2", PROOF);
    add("bad2", calls("ax"));
    note("ax", AXIOM);
    add("ax", bare_sorry());
    assert_eq!(counts(), (1, 0, 1));
}

// AC3: `unused_axiom.tg` — the hole counts, nothing rests.
#[test]
fn an_unused_axiom_counts_its_hole_and_nothing_rests() {
    tg_init();
    tg_holes_reset();
    note("lonely", AXIOM);
    note("main", PROGRAM);
    add("lonely", bare_sorry());
    add("main", bare_unit());
    assert_eq!(counts(), (1, 0, 0));
}

// AC3: the clean control.
#[test]
fn a_clean_program_counts_nothing() {
    tg_init();
    tg_holes_reset();
    note("fine", PROOF);
    note("uses_fine", PROOF);
    add("fine", bare_unit());
    add("uses_fine", calls("fine"));
    assert_eq!(counts(), (0, 0, 0));
}

// AC3: a name never noted is a program definition — its bare hole is not
// authored, and citing it from an unnoted name is not a resting theorem.
#[test]
fn an_unnoted_name_is_a_program_definition() {
    tg_init();
    tg_holes_reset();
    add("hole", bare_sorry());
    add("user", calls("hole"));
    assert_eq!(counts(), (0, 1, 0));
    // A noted proof citing it does rest: an unclassified hole is a root.
    note("thm", PROOF);
    add("thm", calls("hole"));
    assert_eq!(counts(), (0, 1, 1));
}

// AC3: an invalid handle is refused with a message, never dropped silently.
#[test]
fn an_invalid_handle_returns_false_with_a_message() {
    tg_init();
    tg_holes_reset();
    assert!(!unsafe { tg_holes_add_def(cstr("lost"), INVALID_HANDLE) });
    assert!(last_error().contains("lost"), "{}", last_error());
    assert!(!unsafe { tg_holes_add_def(std::ptr::null(), bare_unit()) });
    assert!(last_error().contains("unreadable name"), "{}", last_error());
}

// The kind codes are closed: anything past 2 is refused, by name.
#[test]
fn an_unknown_kind_is_refused_with_a_message() {
    tg_init();
    tg_holes_reset();
    assert!(!unsafe { tg_holes_note_item(cstr("odd"), 3) });
    assert!(last_error().contains("odd"), "{}", last_error());
    assert!(!unsafe { tg_holes_note_item(std::ptr::null(), PROOF) });
    assert!(last_error().contains("unreadable name"), "{}", last_error());
}

// AC3: `reset` discards the last program's kinds, counts and references.
#[test]
fn reset_discards_the_last_program() {
    tg_init();
    tg_holes_reset();
    note("ax", AXIOM);
    note("bad2", PROOF);
    add("ax", bare_sorry());
    add("bad2", calls("ax"));
    assert_eq!(counts(), (1, 0, 1));
    tg_holes_reset();
    assert_eq!(counts(), (0, 0, 0));
    // The old kind is gone: `ax` is now a program definition.
    add("ax", bare_sorry());
    assert_eq!(counts(), (0, 1, 0));
}

// AC3: a name added twice counts both bodies' holes and keeps the last kind,
// class and references (13.10.26m D5's repeated bare name).
#[test]
fn a_name_added_twice_sums_holes_and_keeps_the_last_entry() {
    tg_init();
    tg_holes_reset();
    note("dup", AXIOM);
    add("dup", bare_sorry());
    note("thm", PROOF);
    add("thm", calls("dup"));
    assert_eq!(counts(), (1, 0, 1));
    // The second `dup` is a clean program definition: its hole count adds
    // nothing, and it is no longer a root, so `thm` no longer rests.
    note("dup", PROGRAM);
    add("dup", bare_unit());
    assert_eq!(counts(), (1, 0, 0));
    // A second body with a hole adds to the sum.
    add("dup", bare_sorry());
    assert_eq!(counts(), (1, 1, 1));
}

// The resting population is theorems and lemmas: a program definition
// reaching an axiom is not counted, and an axiom citing one is not either.
#[test]
fn only_proofs_are_counted_as_resting() {
    tg_init();
    tg_holes_reset();
    note("ax", AXIOM);
    note("ax2", AXIOM);
    note("helper", PROGRAM);
    add("ax", bare_sorry());
    add("ax2", calls("ax"));
    add("helper", calls("ax"));
    assert_eq!(counts(), (1, 0, 0));
}

// The registry classifies a non-axiom body by its term: an authored hole is
// authored, a synthesised one is not and makes no root.
#[test]
fn a_non_axiom_body_is_classified_by_its_term() {
    let mut registry = HolesRegistry::default();
    registry.note_item("thm", HoleItemKind::Proof);
    let synthesised = Term::adt_match(
        Term::Unit,
        vec![
            (0, "x".to_string(), Box::new(Term::Zero)),
            (1, "__unreachable_1".to_string(), Box::new(Term::Sorry)),
        ],
    );
    registry.add_def("synth", &synthesised);
    registry.add_def("thm", &Term::Global("synth".to_string()));
    assert_eq!(registry.counts.synthesised, 1);
    assert_eq!(registry.resting(), 0, "a synthesised hole is not a root");
    let authored = Term::spanned(Term::Sorry, crate::terms::TermSpan::new(3, 8));
    registry.add_def("synth", &authored);
    assert_eq!(registry.counts.authored, 1);
    assert_eq!(registry.resting(), 1);
    assert_eq!(
        synthesised.sorry_sites(),
        vec![SorrySite::Synthesised(Construct::UnreachablePatternArm)]
    );
}
