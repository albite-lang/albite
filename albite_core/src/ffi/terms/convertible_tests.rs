//! ADR 30.9.26g AC 4: `tg_terms_convertible`, the self-host's `refl`/`trans`
//! check, over terms built with the FFI constructors.

use std::ffi::CString;
use std::os::raw::c_char;

use super::core::{tg_term_app, tg_term_global, tg_term_var_named};
use super::core_data::{tg_term_nat_lit, tg_term_true, tg_term_zero};
use super::ext::{tg_terms_convertible, tg_terms_equal};
use super::primitives::tg_term_nat_add;
use crate::ffi::{tg_init, TermHandle, INVALID_HANDLE};

/// Build a named term — a variable or a global — through its FFI constructor.
fn named(constructor: unsafe extern "C" fn(*const c_char) -> TermHandle, name: &str) -> TermHandle {
    let name = CString::new(name).expect("no interior NUL");
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call.
    unsafe { constructor(name.as_ptr()) }
}

fn var(name: &str) -> TermHandle {
    named(tg_term_var_named, name)
}

// 30.9.26g AC4: `k + 0` computes to `k`; `0 + k` needs induction.
#[test]
fn convertible_ffi_accepts_a_right_zero() {
    tg_init();
    let right_zero = tg_term_nat_add(var("k"), tg_term_nat_lit(0));
    assert!(tg_terms_convertible(right_zero, var("k")));
    assert!(
        !tg_terms_equal(right_zero, var("k")),
        "structurally they differ"
    );
    let left_zero = tg_term_nat_add(tg_term_zero(), var("k"));
    assert!(!tg_terms_convertible(left_zero, var("k")));
}

// 30.9.26g AC4 / D4: an invalid handle on either side is `false`, as for
// `tg_terms_equal`.
#[test]
fn convertible_ffi_refuses_invalid_handles() {
    tg_init();
    let zero = tg_term_zero();
    assert!(tg_terms_convertible(zero, zero));
    assert!(!tg_terms_convertible(zero, INVALID_HANDLE));
    assert!(!tg_terms_convertible(INVALID_HANDLE, zero));
    assert!(!tg_terms_convertible(INVALID_HANDLE, INVALID_HANDLE));
}

// 30.9.26g AC4 / D5: a `Global` stays neutral — no δ before admission.
#[test]
fn convertible_ffi_leaves_a_global_neutral() {
    tg_init();
    // Two globals differ by name alone, and neither is ever unfolded.
    let same = || named(tg_term_global, "same");
    assert!(!tg_terms_convertible(
        same(),
        named(tg_term_global, "other")
    ));
    let call = tg_term_app(same(), var("k"));
    assert!(!tg_terms_convertible(call, var("k")));
    assert!(tg_terms_convertible(call, tg_term_app(same(), var("k"))));
}

// 30.9.26g AC4 / D4: monotone over `tg_terms_equal`. The pairs are copied
// from `ffi_equality_entry_points_report_both_verdicts`
// (`ffi/term_surface_tests.rs`), plus the variable and application pairs
// this module builds; every pair `tg_terms_equal` accepts is accepted here.
#[test]
fn convertible_ffi_accepts_whatever_equal_accepts() {
    tg_init();
    let pairs = [
        (tg_term_zero(), tg_term_zero()),
        (tg_term_zero(), tg_term_true()),
        (tg_term_zero(), INVALID_HANDLE),
        (var("k"), var("k")),
        (var("k"), var("j")),
        (
            tg_term_app(named(tg_term_global, "same"), var("k")),
            tg_term_app(named(tg_term_global, "same"), var("k")),
        ),
    ];
    let mut equal = 0;
    for (left, right) in pairs {
        if tg_terms_equal(left, right) {
            equal += 1;
            assert!(tg_terms_convertible(left, right));
        }
    }
    assert_eq!(equal, 3, "the table must exercise the accepting arm");
    assert!(!tg_terms_convertible(tg_term_zero(), tg_term_true()));
    assert!(!tg_terms_convertible(var("k"), var("j")));
}
