//! Conversion with δ, discharged after admission (ADR 28.9.26d).
//!
//! Conversion runs during Body Elaboration, but whether a constant may be
//! unfolded is decided by termination admission, which runs once, after every
//! module has elaborated (§1.2). So the elaboration-time check stays δ-free —
//! [`Elaborator::terms_definitionally_equal`] is the fast path — and a failure
//! that δ could change becomes a [`ConversionObligation`] instead of an error.
//! The driver discharges every obligation at the trusted boundary, immediately
//! after admission, with an environment holding only definitions admitted
//! `Total` whose bodies perform no effect (`delta`).
//!
//! Only the per-module Body Elaboration walk opts in (D3): every other entry
//! point leaves `Elaborator::defer_conversion` off, and there a δ proof is
//! refused with today's E0071 — conservative, never unsound, because an entry
//! with no admission step could never discharge it.
//!
//! Phase 2a (ADR 29.9.26c) adds a call-by-name WHNF comparison that runs only
//! where call-by-value has already failed: over the same admitted environment
//! in the discharge, and δ-free in the fast path. Every free variable is
//! renamed apart first, so neither evaluator can capture one. That engine
//! lives in `albite_core::conversion` (ADR 30.9.26g), where the self-host's
//! `tg_terms_convertible` reaches it too. The δ discharge and the deferral
//! table followed it (ADR 7.10.26u, `albite_core::conversion::delta`); what
//! stays here is the elaborator's side — recording, rendering, ordering.

#[cfg(test)]
mod deferral_tests;
#[cfg(test)]
mod delta_tests;
mod discharge;
#[cfg(test)]
mod discharge_tests;
mod nested;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod whnf_tests;

/// The paths the bootstrap's δ tests import, kept when the engine moved to the
/// core (ADR 7.10.26u AC 1), so the tests pass through it unchanged.
#[cfg(test)]
mod delta {
    pub(super) use albite_core::conversion::delta::settle_order;
}
#[cfg(test)]
mod numerals {
    pub(super) use albite_core::conversion::delta::canonicalize_numerals;
}

use albite_core::terms::termination::AdmissionState;
use albite_core::{Term, Type};

use crate::elaborate::{CoreDef, ElabError, ElabResult, Elaborator};
use crate::span::Spanned;

/// The deferral table and its predicates moved to the core with the δ engine
/// (ADR 7.10.26u D3), so the self-host defers by the same rule.
pub(crate) use albite_core::conversion::delta::{
    deferral, is_admitted_total, mentions_global, performs_effect, DeltaEnv, Verdict,
};
/// Moved to the core with the engine (ADR 30.9.26g), which the FFI shares.
pub(crate) use albite_core::conversion::CONVERSION_STEP_BUDGET;
pub(crate) use discharge::{delta_globals, discharge};

/// A conversion the elaboration-time check could not decide without δ,
/// deferred to the post-admission discharge (§2.1).
///
/// `error` is the diagnostic the site would have raised, built and
/// file-stamped at deferral time (D9), so a failed discharge reports exactly
/// what an undeferred failure would have.
#[derive(Debug, Clone)]
pub struct ConversionObligation {
    pub left: Term,
    pub right: Term,
    pub ty: Type,
    pub error: ElabError,
}

/// Whether a case expression is a bare `refl`, seen through parentheses.
fn is_refl(case: &crate::ast::Expr) -> bool {
    match case {
        crate::ast::Expr::Refl(_) => true,
        crate::ast::Expr::Paren(inner, _) => is_refl(inner),
        _ => false,
    }
}

/// Whether a failed check-mode comparison may retry through
/// [`Elaborator::check_eq_against`]: only when **both** sides are a bare `Eq`
/// (ADR 30.9.26e D5). An `Eq` nested in another type stays structural until
/// full Phase 2b.
pub(crate) fn eq_fallback(inferred: &Type, expected: &Type) -> bool {
    matches!(inferred, Type::Eq(..)) && matches!(expected, Type::Eq(..))
}

impl Elaborator<'_> {
    /// Check `left ≡ right`: `Ok(())` if equal now or deferred as an
    /// obligation, `Err(error)` if the failure is final (§2.1).
    pub(crate) fn convert_or_defer(
        &mut self,
        left: &Term,
        right: &Term,
        ty: &Type,
        error: ElabError,
    ) -> ElabResult<()> {
        let fast_equal = self.terms_definitionally_equal(left, right, ty);
        // The normal forms are only needed on the (rare) failure path.
        let normal_forms = if fast_equal {
            (Term::Unit, Term::Unit)
        } else {
            (
                albite_core::eval::eval(left),
                albite_core::eval::eval(right),
            )
        };
        match deferral(
            fast_equal,
            self.defer_conversion,
            (&normal_forms.0, &normal_forms.1),
            (left, right),
        ) {
            Verdict::Equal => Ok(()),
            Verdict::Reject => Err(error),
            Verdict::Defer => {
                self.conversion_obligations.push(ConversionObligation {
                    left: left.clone(),
                    right: right.clone(),
                    ty: ty.clone(),
                    error: self.error_with_file(error),
                });
                self.conversion_deferred += 1;
                Ok(())
            }
        }
    }

    /// Check a `natind` case against its motive instance (ADR 29.9.26d D5).
    ///
    /// A goal that is not an `Eq`, and a `refl` case, go through `check` —
    /// `refl` already converts and defers there. Any other case is inferred:
    /// its type must be an `Eq` over the goal's base type, and each endpoint
    /// pair meets the goal's through `convert_or_defer`, so a proof such as
    /// `cong(|x: Nat| x + 1, ih)`, whose endpoints hold an unreduced β-redex,
    /// is accepted where `types_equal`'s structural comparison would refuse
    /// it. Every refusal is E0010 with the instance as the expected type; a
    /// case whose own type already failed (`Type::Error`) passes through, as
    /// poison does everywhere (ADR 15.8.26d), so the fault is reported once.
    pub(crate) fn check_case_against(
        &mut self,
        case: &crate::ast::Expr,
        goal: &Type,
    ) -> ElabResult<Term> {
        if !matches!(goal, Type::Eq(..)) || is_refl(case) {
            return self.check(case, goal);
        }
        // A `Type::Error` case passes through inside `check_eq_against`.
        let (term, found) = self.infer(case)?;
        let mismatch =
            |_: &Self| ElabError::type_mismatch(case.span(), goal.clone(), found.clone());
        self.check_eq_against(&found, goal, mismatch)?;
        Ok(term)
    }

    /// An inferred `Eq` meets an expected `Eq` when the base types are equal
    /// and each endpoint pair converts, now or after admission (ADR 30.9.26e
    /// D5). Shared by `check_case_against` and check mode's two
    /// infer-then-compare fall-throughs; every refusal is the caller's own
    /// `mismatch`, so only acceptances differ from a structural comparison.
    pub(crate) fn check_eq_against(
        &mut self,
        found: &Type,
        goal: &Type,
        mismatch: impl Fn(&Self) -> ElabError,
    ) -> ElabResult<()> {
        let (found_base, found_left, found_right, goal_base, goal_left, goal_right) =
            match (found, goal) {
                (Type::Eq(fb, fl, fr), Type::Eq(gb, gl, gr)) => (fb, fl, fr, gb, gl, gr),
                // Poison passes through (ADR 15.8.26d): its fault is already reported.
                (Type::Error, _) | (_, Type::Error) => return Ok(()),
                _ => return Err(mismatch(self)),
            };
        if !self.types_equal(found_base, goal_base) {
            return Err(mismatch(self));
        }
        let left_error = mismatch(self);
        self.convert_or_defer(found_left, goal_left, goal_base, left_error)?;
        let right_error = mismatch(self);
        self.convert_or_defer(found_right, goal_right, goal_base, right_error)
    }

    /// Run `f` with deferral off, restoring the previous setting afterwards
    /// (D13). `expect_type` / `expect_error` observe their operand's errors
    /// directly, so an obligation recorded inside one would read as
    /// "elaborated successfully" and then leak into the module's list.
    pub(crate) fn without_conversion_deferral<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let previous = std::mem::replace(&mut self.defer_conversion, false);
        let result = f(self);
        self.defer_conversion = previous;
        result
    }
}

impl crate::elaborate::CollectedElaborator<'_> {
    /// Defer conversion failures δ could change instead of raising them
    /// (D3). Only the per-module Body Elaboration walk calls this.
    pub fn set_defer_conversion(&mut self, enabled: bool) {
        self.elaborator.defer_conversion = enabled;
    }
}

/// Deduplicate on `(file_path, span, left, right)`, then order by
/// `(file_path, span)` so the parallel walk's merge order cannot reorder the
/// errors (D10).
pub(crate) fn dedup_and_sort(obligations: Vec<ConversionObligation>) -> Vec<ConversionObligation> {
    let mut unique: Vec<ConversionObligation> = Vec::with_capacity(obligations.len());
    for obligation in obligations {
        let seen = unique.iter().any(|kept| {
            kept.error.file_path == obligation.error.file_path
                && kept.error.span == obligation.error.span
                && kept.left == obligation.left
                && kept.right == obligation.right
        });
        if !seen {
            unique.push(obligation);
        }
    }
    unique.sort_by(|a, b| {
        (&a.error.file_path, a.error.span.start, a.error.span.end).cmp(&(
            &b.error.file_path,
            b.error.span.start,
            b.error.span.end,
        ))
    });
    unique
}

/// The verbose line, printed even when nothing was deferred, so its absence
/// is itself a failure (AC 8).
pub(crate) fn discharge_summary(obligations: usize, discharged: usize) -> String {
    format!("conversion: {obligations} obligation(s), {discharged} discharged")
}

/// Discharge every obligation against the admitted environment (§2.2), and
/// return the errors of those that failed.
///
/// `deferred` is the count every `convert_or_defer` deferral incremented
/// (D15); a mismatch means an obligation was lost between recording and here,
/// which would accept a proof nobody checked.
pub(crate) fn discharge_after_admission(
    defs: &[CoreDef],
    admission: &std::collections::BTreeMap<String, AdmissionState>,
    obligations: Vec<ConversionObligation>,
    deferred: usize,
    verbose: bool,
) -> Vec<ElabError> {
    debug_assert_eq!(
        obligations.len(),
        deferred,
        "a deferred conversion obligation was lost before discharge (ADR 28.9.26d D15)"
    );
    let obligations = dedup_and_sort(obligations);
    let failures = if obligations.is_empty() {
        Vec::new()
    } else {
        let candidates = crate::comparator::eval::defs_as_globals(defs);
        let env = delta_globals(&candidates, admission, &obligations, CONVERSION_STEP_BUDGET);
        discharge(&obligations, &env, CONVERSION_STEP_BUDGET)
    };
    if verbose {
        eprintln!(
            "{}",
            discharge_summary(obligations.len(), obligations.len() - failures.len())
        );
    }
    failures
}
