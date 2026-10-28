//! `(x: A) -> B` → `Type::Pi(x, A, B)`, or `A → B` when `x` is unused (ADR
//! 7.10.26n D2).
//!
//! The binder is bound as a TERM of type `A`, as `exists`' is, so `x` may
//! appear where a term may appear in a type: an `Eq` argument, a family
//! argument or a predicate argument. `(T: Nat) -> T` stays an unknown type.

use crate::ast::{Ident, TypeExpr};
use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;
use crate::span::Spanned;
use albite_core::Type;

use super::exists::binds_data;

impl Elaborator<'_> {
    /// Elaborate `(binder: domain) -> codomain`.
    pub(super) fn elab_dependent_function_type(
        &mut self,
        binder: &Ident,
        domain: &TypeExpr,
        codomain: &TypeExpr,
    ) -> ElabResult<Type> {
        let domain_ty = self.elab_type(domain)?;
        let saved_depth = self.depth;
        self.env.push_scope();
        self.env
            .bind_local(binder.name.clone(), domain_ty.clone(), self.depth);
        self.depth += 1;
        let codomain_ty = self.elab_type(codomain);
        // Popped before the `?`, so a failed codomain leaks no binder (ADR
        // 6.10.26b D1's rule).
        self.env.pop_scope();
        self.depth = saved_depth;
        let codomain_ty = codomain_ty?;
        dependent_function_type(&binder.name, domain_ty, codomain_ty).map_err(|domain_ty| {
            ElabError::new(domain.span(), ElabErrorKind::ExistsBinderNotData(domain_ty))
        })
    }
}

/// D2's rule over elaborated parts: an arrow when `binder` is not free in
/// `codomain`; a `Pi` when it is and `domain` is data; else the refused
/// domain, which a binder over a proof cannot be (E0065's check).
fn dependent_function_type(binder: &str, domain: Type, codomain: Type) -> Result<Type, Type> {
    if !codomain.free_term_vars().contains(binder) {
        return Ok(Type::arrow(domain, codomain));
    }
    if !binds_data(&domain) {
        return Err(domain);
    }
    Ok(Type::pi(binder, domain, codomain))
}

#[cfg(test)]
mod tests {
    use super::dependent_function_type;
    use albite_core::{Term, Type};

    fn eq_on(name: &str) -> Type {
        Type::eq(Type::Nat, Term::var(name), Term::var(name))
    }

    // 7.10.26n AC2 (D2): an unused binder is an arrow, a used one a `Pi`.
    #[test]
    fn an_unused_binder_is_an_arrow_and_a_used_one_a_pi() {
        assert_eq!(
            dependent_function_type("n", Type::Nat, Type::Nat),
            Ok(Type::arrow(Type::Nat, Type::Nat))
        );
        assert_eq!(
            dependent_function_type("n", Type::Nat, eq_on("n")),
            Ok(Type::pi("n", Type::Nat, eq_on("n")))
        );
        // A different free name leaves the binder unused.
        assert_eq!(
            dependent_function_type("n", Type::Nat, eq_on("m")),
            Ok(Type::arrow(Type::Nat, eq_on("m")))
        );
    }

    // 7.10.26n AC2 (D2): a used binder over a proof is refused; an unused one
    // is a plain implication.
    #[test]
    fn a_used_binder_over_a_proof_is_refused() {
        let proof = Type::eq(Type::Nat, Term::Zero, Term::Zero);
        assert_eq!(
            dependent_function_type("e", proof.clone(), eq_on("e")),
            Err(proof.clone())
        );
        assert_eq!(
            dependent_function_type("e", proof.clone(), Type::Void),
            Ok(Type::arrow(proof, Type::Void))
        );
    }
}
