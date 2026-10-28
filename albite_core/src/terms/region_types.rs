//! The two types a region's copy-out needs (ADR 13.10.26q D2).
//!
//! `elab_region` already holds both where E0094 checks them: the region's
//! value type, and the innermost return type when an exit inside the body
//! crosses the region. Carrying them on `Term::Region` means the code
//! generator infers nothing — `infer_term_type` types a `Return` as `Void`
//! and a `Var` only through the context its caller passes.

use serde::{Deserialize, Serialize};

use crate::types::Type;

/// What leaves a region: its value, and what a crossing `return` or `?`
/// carries out of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionTypes {
    /// The type of `region { e }` itself.
    pub value: Type,
    /// The innermost return type, when the body holds an exit crossing the
    /// region; `None` otherwise.
    pub exit: Option<Type>,
}

impl RegionTypes {
    /// The types of a region with no crossing exit.
    #[must_use]
    pub fn value_only(value: Type) -> Self {
        Self { value, exit: None }
    }

    /// Apply `rewrite` to both types: substitution and span-stripping
    /// reach the region's types as they reach `Inl`'s.
    #[must_use]
    pub fn map_types(&self, mut rewrite: impl FnMut(&Type) -> Type) -> Self {
        Self {
            value: rewrite(&self.value),
            exit: self.exit.as_ref().map(rewrite),
        }
    }

    /// Both types, the value first.
    pub fn types(&self) -> impl Iterator<Item = &Type> {
        std::iter::once(&self.value).chain(self.exit.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_types_rewrites_value_and_exit() {
        let types = RegionTypes {
            value: Type::TyVar("a".into()),
            exit: Some(Type::TyVar("a".into())),
        };
        let mapped = types.map_types(|ty| ty.substitute("a", &Type::String));
        assert_eq!(mapped.value, Type::String);
        assert_eq!(mapped.exit, Some(Type::String));
    }

    #[test]
    fn value_only_has_no_exit_and_one_type() {
        let types = RegionTypes::value_only(Type::Nat);
        assert_eq!(types.exit, None);
        assert_eq!(types.types().collect::<Vec<_>>(), vec![&Type::Nat]);
        let crossing = RegionTypes {
            value: Type::Nat,
            exit: Some(Type::String),
        };
        assert_eq!(
            crossing.types().collect::<Vec<_>>(),
            vec![&Type::Nat, &Type::String]
        );
    }
}
