//! The elaboration-time half of the δ rule (ADR 28.9.26d §2.1, D2, D4, D5):
//! whether a failed δ-free conversion is deferred, and what the discharge may
//! unfold. Moved from the bootstrap with the engine (ADR 7.10.26u D3), so the
//! self-host defers by the same table.

use std::collections::BTreeMap;

use crate::eval::extern_registry::{entry_for, ExternKind};
use crate::terms::termination::AdmissionState;
use crate::Term;

/// What the elaboration-time check does with a conversion (§2.1's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Equal without δ.
    Equal,
    /// Unequal without δ, but δ could change the answer: record an obligation.
    Defer,
    /// Final: raise the error now, exactly as before δ existed.
    Reject,
}

/// Decide a conversion's fate (§2.1). `normal_forms` are the env-free normal
/// forms of the two sides; `sides` are the sides as written.
///
/// No global in either normal form means δ cannot change the answer (D2); an
/// effectful extern call in either side means the obligation could not be
/// discharged without performing the effect (D5).
#[must_use]
pub fn deferral(
    fast_equal: bool,
    defer_enabled: bool,
    normal_forms: (&Term, &Term),
    sides: (&Term, &Term),
) -> Verdict {
    if fast_equal {
        return Verdict::Equal;
    }
    if !defer_enabled || !(mentions_global(normal_forms.0) || mentions_global(normal_forms.1)) {
        return Verdict::Reject;
    }
    if performs_effect(sides.0) || performs_effect(sides.1) {
        return Verdict::Reject;
    }
    Verdict::Defer
}

/// Whether `term` references any global constant.
#[must_use]
pub fn mentions_global(term: &Term) -> bool {
    if matches!(term, Term::Global(_)) {
        return true;
    }
    let mut found = false;
    term.for_each_subterm(|child| found = found || mentions_global(child));
    found
}

/// Whether `term` calls an extern the evaluator would execute as an effect
/// (D4): console output, arena or builder mutation, a test assertion.
///
/// An extern the evaluator cannot execute at all (`entry_for` is `None`) is not
/// an effect here — its call stays Stuck and is compared syntactically.
#[must_use]
pub fn performs_effect(term: &Term) -> bool {
    if let Term::ExternCall(name, _) = term {
        if entry_for(name).is_some_and(|entry| entry.kind != ExternKind::Pure) {
            return true;
        }
    }
    let mut found = false;
    term.for_each_subterm(|child| found = found || performs_effect(child));
    found
}

/// Whether the discharge may unfold `name`: admitted `Total` (D4). A name the
/// report does not know is not admitted — the same answer `state_of` gives.
#[must_use]
pub fn is_admitted_total(admission: &BTreeMap<String, AdmissionState>, name: &str) -> bool {
    admission
        .get(name)
        .copied()
        .unwrap_or(AdmissionState::Rejected)
        .is_delta_reducible()
}
