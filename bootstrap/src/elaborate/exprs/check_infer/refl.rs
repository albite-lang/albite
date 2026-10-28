//! `refl` checking against equality types (ADR 21.5.26d).

use albite_core::{Term, Type};

use crate::elaborate::error::ElabError;
use crate::elaborate::{ElabResult, Elaborator};

impl Elaborator<'_> {
    /// Check `refl` against an expected type.
    ///
    /// Succeeds only when `expected` is `Eq(τ, t1, t2)` and `t1` and `t2` are
    /// definitionally equal (normalize to the same term) — or, on the
    /// per-module path, when the failure is deferred for δ to decide after
    /// admission (ADR 28.9.26d §2.1).
    pub(crate) fn check_refl(
        &mut self,
        span: crate::span::Span,
        expected: &Type,
    ) -> ElabResult<Term> {
        match expected {
            Type::Eq(ty, t1, t2) => {
                let error = ElabError::invalid_refl(span, (**t1).clone(), (**t2).clone());
                self.convert_or_defer(t1, t2, ty, error)?;
                Ok(Term::refl((**ty).clone(), (**t1).clone()))
            }
            _ => Err(ElabError::refl_expected_equality(span, expected.clone())),
        }
    }
}
