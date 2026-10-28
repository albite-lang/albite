//! Type constructors.
//!
//! Reconstruction helpers are in `reconstruction.rs`.

use crate::types::Type;

impl Type {
    /// Construct a function type τ₁ → τ₂
    #[must_use]
    pub fn arrow(t1: Type, t2: Type) -> Type {
        Type::Arrow(Box::new(t1), Box::new(t2))
    }

    /// Construct a dependent function type Π x:τ₁. τ₂ unconditionally. Prefer
    /// [`Type::pi_or_arrow`], which keeps a non-dependent type an `Arrow`.
    pub fn pi(var: impl Into<String>, t1: Type, t2: Type) -> Type {
        Type::Pi(var.into(), Box::new(t1), Box::new(t2))
    }

    /// `Π var:t1. t2` when `var` is free in `t2`, else `t1 → t2` (ADR 30.9.26d
    /// D1): every non-dependent signature keeps its `Arrow` byte for byte.
    pub fn pi_or_arrow(var: impl Into<String>, t1: Type, t2: Type) -> Type {
        let var = var.into();
        if t2.free_term_vars().contains(&var) {
            Type::pi(var, t1, t2)
        } else {
            Type::arrow(t1, t2)
        }
    }

    /// Construct a dependent pair type Σ x:τ₁. τ₂ (ADR 4.10.26h D1). There is
    /// deliberately no `sigma_or_product`: a `Sigma` whose binder is unused
    /// stays a `Sigma`, so the binder check is never skipped.
    #[must_use]
    pub fn sigma(var: impl Into<String>, t1: Type, t2: Type) -> Type {
        Type::Sigma(var.into(), Box::new(t1), Box::new(t2))
    }

    /// Construct a product type τ₁ × τ₂
    #[must_use]
    pub fn product(t1: Type, t2: Type) -> Type {
        Type::Product(Box::new(t1), Box::new(t2))
    }

    /// Construct a sum type τ₁ + τ₂
    #[must_use]
    pub fn sum(t1: Type, t2: Type) -> Type {
        Type::Sum(Box::new(t1), Box::new(t2))
    }

    /// Construct a forall type ∀α. τ
    pub fn forall(var: impl Into<String>, ty: Type) -> Type {
        Type::Forall(var.into(), Box::new(ty))
    }

    /// Construct an equality type Eq τ t₁ t₂
    #[must_use]
    pub fn eq(ty: Type, t1: crate::terms::Term, t2: crate::terms::Term) -> Type {
        Type::Eq(Box::new(ty), Box::new(t1), Box::new(t2))
    }

    /// Construct a recursive type μα. τ
    pub fn mu(var: impl Into<String>, ty: Type) -> Type {
        Type::Mu(var.into(), Box::new(ty))
    }

    /// Construct a pointer type *τ
    #[must_use]
    pub fn ptr(ty: Type) -> Type {
        Type::Ptr(Box::new(ty))
    }

    /// Construct a ref type Ref<τ>
    #[must_use]
    pub fn ref_ty(ty: Type) -> Type {
        Type::Ref(Box::new(ty))
    }

    /// Construct a deferred type application
    pub fn app(name: impl Into<String>, args: Vec<Type>) -> Type {
        Type::App(name.into(), args)
    }

    /// Construct an ADT type (flat enum)
    ///
    /// # Arguments
    /// - `name`: The ADT name
    /// - `type_args`: Type parameters  
    /// - `variants`: List of (`constructor_name`, `payload_type`) pairs
    pub fn adt(
        name: impl Into<String>,
        type_args: Vec<Type>,
        variants: Vec<(String, Type)>,
    ) -> Type {
        Type::Adt(name.into(), type_args, variants)
    }
}
