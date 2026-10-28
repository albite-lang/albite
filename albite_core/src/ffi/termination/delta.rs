//! The self-host's δ deferral and discharge (ADR 7.10.26u D5).
//!
//! The rule is 28.9.26d's, unchanged: conversion is δ-free while bodies
//! elaborate; a `refl` failure whose sides mention a global, and perform no
//! effect, is recorded here instead of refused; once termination admission has
//! run over the whole program, each obligation is discharged with only the
//! admitted `Total` definitions unfoldable, and a failure is reported where the
//! `refl` was written.
//!
//! Four externs: `defer` (record or refuse), `deferral_set` (the bootstrap's
//! D13 — `expect_error`/`expect_type` observe their operand's errors directly,
//! so an obligation inside one would read as success), `discharge` (count the
//! failures) and `failure_render` (read one back). Deferral is **off** until
//! [`super::tg_termination_reset`] opens a program and off again after its
//! discharge, so an entry point that never discharges refuses a δ proof, as
//! the bootstrap's does — conservative, never unsound.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::{c_char, CString};

use crate::conversion::convertible_without_delta;
use crate::conversion::delta::{deferral, global_names, DischargeFailure, Sides, Verdict};
use crate::terms::Term;

use crate::ffi::terms::nodes::materialize_term;
use crate::ffi::{with_arena_ref, TermHandle};

use super::{read_cstr, with_registry};

/// One recorded obligation: the two sides, and where its `refl` was written.
#[derive(Debug, Clone)]
struct RecordedObligation {
    left: Term,
    right: Term,
    start: u64,
    end: u64,
    file: String,
}

/// The obligations one program recorded, and what their discharge found.
#[derive(Debug, Default)]
pub(crate) struct DeltaLedger {
    /// Whether a `refl` may defer now.
    enabled: bool,
    obligations: Vec<RecordedObligation>,
    failures: Vec<DischargeFailure>,
}

impl DeltaLedger {
    /// Open a fresh program: nothing recorded, deferral on.
    pub(crate) fn open(&mut self) {
        *self = DeltaLedger {
            enabled: true,
            ..DeltaLedger::default()
        };
    }

    /// Set whether deferral is on, returning the previous setting so a caller
    /// can restore it.
    fn set_enabled(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.enabled, enabled)
    }

    /// Record `left ≡ right` if the deferral table says so (§2.1). Returns
    /// whether the obligation was recorded; `false` means the caller refuses
    /// now, exactly as before δ existed.
    fn defer(&mut self, left: Term, right: Term, start: u64, end: u64, file: String) -> bool {
        let fast_equal = convertible_without_delta(&left, &right);
        let normal_forms = (crate::eval::eval(&left), crate::eval::eval(&right));
        match deferral(
            fast_equal,
            self.enabled,
            (&normal_forms.0, &normal_forms.1),
            (&left, &right),
        ) {
            Verdict::Defer => {
                self.obligations.push(RecordedObligation {
                    left,
                    right,
                    start,
                    end,
                    file,
                });
                true
            }
            Verdict::Equal | Verdict::Reject => false,
        }
    }

    /// Every global the recorded obligations mention: the discharge closure's
    /// seeds.
    pub(crate) fn seeds(&self) -> BTreeSet<String> {
        self.obligations
            .iter()
            .flat_map(|o| {
                global_names(&o.left)
                    .into_iter()
                    .chain(global_names(&o.right))
            })
            .collect()
    }

    /// Discharge every obligation with `run` and close the program: deferral
    /// is off until the next one opens. Returns the failure count.
    fn discharge(&mut self, run: impl FnOnce(&[Sides<'_>]) -> Vec<DischargeFailure>) -> usize {
        let sides: Vec<Sides<'_>> = self
            .obligations
            .iter()
            .map(|o| (&o.left, &o.right))
            .collect();
        let failures = if sides.is_empty() {
            Vec::new()
        } else {
            run(&sides)
        };
        self.failures = failures;
        self.enabled = false;
        self.failures.len()
    }

    /// The `index`th failure as `"<start>\n<end>\n<file>\n<note>"`; the note
    /// is empty when the engine gave none.
    fn render(&self, index: usize) -> Option<String> {
        let failure = self.failures.get(index)?;
        let obligation = self.obligations.get(failure.index)?;
        Some(format!(
            "{}\n{}\n{}\n{}",
            obligation.start,
            obligation.end,
            obligation.file,
            failure.note.as_deref().unwrap_or("")
        ))
    }
}

thread_local! {
    /// The program's obligations. Thread-local like the registry, for the same
    /// reason: the FFI is single-threaded by contract.
    pub(crate) static LEDGER: RefCell<DeltaLedger> = RefCell::new(DeltaLedger::default());
}

/// Access the ledger mutably.
pub(crate) fn with_ledger<T>(body: impl FnOnce(&mut DeltaLedger) -> T) -> T {
    LEDGER.with(|cell| body(&mut cell.borrow_mut()))
}

/// Defer `lhs ≡ rhs`, which failed δ-free, if δ could still decide it.
///
/// Returns `true` when the obligation was recorded — the caller accepts the
/// `refl` for now — and `false` when the failure is final: no global on
/// either side, an effect, deferral off, or an unreadable handle or file.
/// `start`/`end` are the `refl`'s byte span in `file`.
///
/// # Safety
/// `file` must be null or a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tg_conversion_defer(
    lhs: TermHandle,
    rhs: TermHandle,
    start: u64,
    end: u64,
    file: *const c_char,
) -> bool {
    let sides =
        with_arena_ref!(|arena| (materialize_term(arena, lhs), materialize_term(arena, rhs)));
    let (Some(left), Some(right), Some(file)) = (sides.0, sides.1, read_cstr(file)) else {
        return false;
    };
    with_ledger(|ledger| ledger.defer(left, right, start, end, file))
}

/// Turn deferral on or off, returning the previous setting.
#[no_mangle]
pub extern "C" fn tg_conversion_deferral_set(enabled: bool) -> bool {
    with_ledger(|ledger| ledger.set_enabled(enabled))
}

/// Discharge every recorded obligation against the last termination check's
/// admission, and return how many failed. Call after `tg_termination_check`.
#[no_mangle]
pub extern "C" fn tg_conversion_discharge() -> u64 {
    with_ledger(|ledger| {
        ledger.discharge(|sides| with_registry(|registry| registry.discharge_deltas(sides)))
    }) as u64
}

/// The `index`th failed obligation, as `"<start>\n<end>\n<file>\n<note>"`;
/// null when out of range. Leaked, like every C string this FFI hands out.
#[no_mangle]
pub extern "C" fn tg_conversion_failure_render(index: u64) -> *const c_char {
    let rendered = with_ledger(|ledger| ledger.render(index as usize));
    match rendered.and_then(|text| CString::new(text).ok()) {
        Some(cstring) => cstring.into_raw().cast_const(),
        None => std::ptr::null(),
    }
}
