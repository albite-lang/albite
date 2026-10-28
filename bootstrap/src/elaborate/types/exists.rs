//! `exists x: A. B` → `Type::Sigma(x, A, B)` (ADR 4.10.26h D5).
//!
//! The body is elaborated with `x` bound as a TERM of type `A` — the binding
//! 30.9.26d gives a signature parameter — so `x` may appear inside an `Eq`
//! argument and nowhere else: `exists k: Nat. k` stays an unknown type.

use crate::ast::{Ident, TypeExpr};
use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;
use crate::span::Spanned;
use albite_core::types::Sort;
use albite_core::Type;

impl Elaborator<'_> {
    /// Elaborate `exists binder: binder_ty. body`.
    pub(super) fn elab_exists_type(
        &mut self,
        binder: &Ident,
        binder_ty: &TypeExpr,
        body: &TypeExpr,
    ) -> ElabResult<Type> {
        let domain = self.elab_type(binder_ty)?;
        if !binds_data(&domain) {
            return Err(ElabError::new(
                binder_ty.span(),
                ElabErrorKind::ExistsBinderNotData(domain),
            ));
        }
        let saved_depth = self.depth;
        self.env.push_scope();
        self.env
            .bind_local(binder.name.clone(), domain.clone(), self.depth);
        self.depth += 1;
        let body_ty = self.elab_type(body);
        self.env.pop_scope();
        self.depth = saved_depth;
        Ok(Type::sigma(binder.name.clone(), domain, body_ty?))
    }
}

/// Q4's sort check, the same predicate the kernel applies (ADR 4.10.26i D1,
/// replacing 4.10.26h D3's stand-in): a binder ranges over data, never a
/// type that sorts `Prop`. A poison has no sort and passes, as transit. A
/// dependent function's used binder shares it (ADR 7.10.26n D2).
pub(super) fn binds_data(domain: &Type) -> bool {
    domain.sort_of() != Some(Sort::Prop)
}

#[cfg(test)]
mod tests {
    use super::binds_data;
    use albite_core::{Term, Type};

    // 4.10.26h AC3 (D3): the elaborator's binder check agrees with the kernel's.
    #[test]
    fn a_binder_binds_data_only() {
        assert!(binds_data(&Type::Nat));
        assert!(binds_data(&Type::product(Type::Nat, Type::Bool)));
        assert!(!binds_data(&Type::Void));
        assert!(!binds_data(&Type::eq(Type::Nat, Term::Zero, Term::Zero)));
        // 4.10.26i AC1 (D1): a conjunction, which `is_proposition` misses.
        let zero_eq = Type::eq(Type::Nat, Term::Zero, Term::Zero);
        assert!(!binds_data(&Type::product(zero_eq.clone(), zero_eq)));
        assert!(binds_data(&Type::Error));
    }
}
