//! Phase 1: ADT Resolution
//!
//! Resolves the ADT type from constructor patterns in match arms.

use crate::ast::{self, Pattern};
use crate::span::Span;
use albite_core::Type;

use super::context::AdtMatchContext;
use super::ownership;
use crate::elaborate::env::{self as elab_env, ModulePath, PathResolutionError};
use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::{ElabResult, Elaborator};

impl<'a> Elaborator<'a> {
    /// Resolve ADT match context from constructor patterns.
    ///
    /// Finds the first constructor pattern in the arms, resolves its path,
    /// and looks up the ADT type definition. `None` is the poison verdict
    /// (ADR 15.8.26d): the ADT's own body failed, its stub has no
    /// constructor list, and the fault is already reported at its span —
    /// the match is transit, not a second complaint.
    ///
    /// A scrutinee that is not the resolved type's encoding is E0022 at the
    /// first constructor pattern (ADR 4.10.26d D1 b, D2).
    pub(super) fn resolve_adt_match_context(
        &self,
        arms: &[ast::MatchArm],
        scrutinee_ty: &Type,
        span: Span,
    ) -> ElabResult<Option<AdtMatchContext>> {
        // Find the first arm naming a constructor (may not be arms[0] due to
        // catch-alls). Defensive: callers route a match here only after
        // finding such an arm — uncoded by design (ADR 15.8.26b).
        let ctor_path = &self
            .find_first_owner_arm(arms, scrutinee_ty)
            .ok_or_else(|| {
                ElabError::new(
                    span,
                    ElabErrorKind::Other("expected at least one constructor pattern".to_string()),
                )
            })?;

        // Resolve the constructor and get its ADT info
        let ctor_info = self.resolve_constructor_for_match(ctor_path)?;

        // Look up the type definition
        let type_def = self
            .env
            .lookup_type(&ctor_info.type_name)
            .cloned()
            .ok_or_else(|| {
                ElabError::internal(span, format!("type `{}` not found", ctor_info.type_name))
            })?;

        if type_def.is_poison() {
            return Ok(None);
        }

        // Extract constructors from ADT
        // An undeclared sibling's constructor is the user's error (ADR 2.10.26g D4).
        if let Some(refusal) = elab_env::sibling_constructor_refusal(
            &ctor_path.item_name().name,
            &type_def,
            &self.env.names_only,
            span,
        ) {
            return Err(refusal);
        }

        let constructors = match &type_def.kind {
            elab_env::TypeDefKind::ADT(ctors) => ctors.clone(),
            _ => {
                // A resolved constructor implies its owning type is an ADT —
                // same invariant `constructors/context.rs` asserts.
                return Err(ElabError::internal(
                    span,
                    format!("`{}` is not an ADT", ctor_info.type_name),
                ));
            }
        };

        // D1(b) of ADR 4.10.26d: the scrutinee must be this type's encoding,
        // or every arm reads its payload at another type's index. A type
        // parameter is never one (ADR 13.10.26i D1).
        let normalized = self.normalize_for_comparison(scrutinee_ty);
        if self.scrutinee_is_type_parameter(&normalized)
            || !ownership::scrutinee_admits(&constructors, &normalized)
        {
            return Err(ElabError::new(
                ctor_path.span,
                ElabErrorKind::MatchScrutineeNotAdt {
                    adt_name: Some(ctor_info.type_name.clone()),
                    found: scrutinee_ty.clone(),
                },
            ));
        }

        // Check if it's recursive (self-recursive or in a mutual recursion group)
        let is_recursive = self.adt_is_recursive(&ctor_info.type_name, &constructors);

        Ok(Some(AdtMatchContext {
            type_def,
            owner: ctor_info.type_name,
            constructors,
            is_recursive,
        }))
    }

    /// Whether the (normalized) scrutinee type is a type parameter of the
    /// enclosing definition, which no constructor pattern may match (ADR
    /// 13.10.26i D1). `scrutinee_admits` admits every `TyVar` unread (4.10.26d
    /// D7) because a collection-time `@Name`, a μ-marker or an unresolved
    /// import cannot be read; a parameter in scope can, and the caller chooses
    /// it, so it is never the encoding of a particular type. Shared by the
    /// top-level and nested resolvers so the two cannot drift apart.
    pub(in crate::elaborate) fn scrutinee_is_type_parameter(&self, normalized: &Type) -> bool {
        matches!(normalized, Type::TyVar(name) if self.env.has_type_var(name))
    }

    /// The path of the first arm that names a constructor: a constructor
    /// pattern, or a bare name the routing rule reads as one (ADR 6.10.26k
    /// D2 a), as the one-segment path its `()` spelling would carry.
    pub(super) fn find_first_owner_arm(
        &self,
        arms: &[ast::MatchArm],
        scrutinee_ty: &Type,
    ) -> Option<ast::Path> {
        arms.iter().find_map(|arm| match &arm.pattern {
            Pattern::Constructor(path, _, _) => Some(path.clone()),
            Pattern::Var(ident)
                if self.bare_name_routes_to_constructor_matcher(ident, scrutinee_ty) =>
            {
                Some(ast::Path::simple(ident.clone()))
            }
            _ => None,
        })
    }

    /// Resolve a constructor path for pattern matching.
    ///
    /// Checks module visibility and resolves the constructor info.
    pub(super) fn resolve_constructor_for_match(
        &self,
        path: &ast::Path,
    ) -> ElabResult<elab_env::ConstructorInfo> {
        let name = path.item_name();

        // Check module visibility for qualified paths
        if !path.is_simple() {
            let module_path = ModulePath::new(
                path.module_segments()
                    .iter()
                    .map(|s| s.name.clone())
                    .collect(),
            );
            if !self
                .env
                .is_module_accessible(&module_path, &self.current_module)
            {
                return Err(ElabError::private_module(
                    path.span,
                    module_path.to_string(),
                    self.current_module.to_string(),
                ));
            }
        }

        match self
            .env
            .resolve_constructor_path(path, &self.current_module)
        {
            Ok(Some(info)) => {
                let info = info.clone();
                // Check constructor visibility (ADR 14.5.26c AC3)
                if !self
                    .env
                    .is_constructor_accessible(&info, &self.current_module)
                {
                    if let Some(item_module) = self.env.get_item_module(&info.type_name) {
                        return Err(ElabError::private_item(
                            path.span,
                            &name.name,
                            "constructor",
                            item_module.to_string(),
                            self.current_module.to_string(),
                        ));
                    }
                }
                Ok(info)
            }
            Ok(None) => Err(self.undefined_constructor_error(name.span, &name.name)),
            Err(PathResolutionError::ModuleNotFound(module)) => {
                Err(ElabError::module_not_found(path.span, module.to_string()))
            }
            Err(PathResolutionError::ItemNotFound { module, item }) => Err(
                ElabError::item_not_in_module(path.span, module.to_string(), item),
            ),
            Err(PathResolutionError::Ambiguous { item, candidates }) => {
                Err(ElabError::ambiguous_value(path.span, item, candidates))
            }
        }
    }
}
