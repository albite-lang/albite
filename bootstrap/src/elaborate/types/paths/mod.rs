//! Type Path and Name Resolution: resolving type paths (both simple and
//! qualified) to their corresponding type definitions. Type application is
//! in `application.rs` (ADR 6.10.26b D5).

mod application;

#[cfg(test)]
pub(in crate::elaborate::types) use application::{
    classify_self_reference_arguments, SelfReferenceArguments,
};

use crate::elaborate::env::{ModulePath, PathResolutionError, TypeDefKind};
use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;
use crate::span::Spanned;
use albite_core::Type;

impl<'a> Elaborator<'a> {
    /// Elaborate a type path (simple or qualified).
    pub(super) fn elab_type_path(&mut self, path: &crate::ast::Path) -> ElabResult<Type> {
        let name = &path.item_name().name;
        let span = path.span;

        // Check for built-in types first (only for simple paths)
        if path.is_simple() {
            if let Some(ty) = Self::builtin_type(name) {
                return Ok(ty);
            }

            // A predicate variable is never a type: used bare, it is missing
            // its arguments (ADR 6.10.26h D3).
            if let Some(domains) = self.env.type_var_kind(name) {
                return Err(
                    ElabError::arity_mismatch(span, domains.len(), 0).with_note(format!(
                        "`{name}` is a predicate parameter: apply it, `{name}<…>`"
                    )),
                );
            }
            // A type variable, or the bare name of the type being defined
            // (ADR 6.10.26b D1): both stay the `TyVar` they always were.
            if self.env.has_type_var(name) || self.env.self_reference_params(name).is_some() {
                return Ok(Type::TyVar(name.to_string()));
            }
        }

        // Resolve path to type definition
        let (type_name, params_len, kind, visibility) = self.resolve_type_path_checked(path)?;

        // Check item visibility
        self.check_type_item_visibility(&type_name, visibility, span)?;

        // If it has no parameters, we can use it directly
        if params_len == 0 {
            self.type_def_to_type(name, &type_name, &kind, span)
        } else {
            // Type requires parameters
            Err(ElabError::new(
                span,
                ElabErrorKind::ArityMismatch {
                    expected: params_len,
                    found: 0,
                },
            )
            .with_note(format!(
                "`{}` requires {} type parameter(s)",
                name, params_len
            )))
        }
    }

    /// Map a builtin type name to its Type, if any.
    ///
    /// Builtin lookup precedes user types, which is why `register_type_name`
    /// refuses a `type` under one of these names (ADR 14.9.26c).
    pub(in crate::elaborate) fn builtin_type(name: &str) -> Option<Type> {
        Type::primitive_by_name(name)
    }

    /// Resolve a type path to its definition info, checking module visibility.
    fn resolve_type_path_checked(
        &self,
        path: &crate::ast::Path,
    ) -> ElabResult<(String, usize, TypeDefKind, crate::ast::Visibility)> {
        let span = path.span;
        let name = &path.item_name().name;

        let resolution_result = self.env.resolve_type_path(path, &self.current_module);
        match resolution_result {
            Ok(Some(type_def)) => {
                // For qualified paths, check module visibility
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
                            span,
                            module_path.to_string(),
                            self.current_module.to_string(),
                        ));
                    }
                }
                Ok((
                    type_def.name.clone(),
                    type_def.params.len(),
                    type_def.kind.clone(),
                    type_def.visibility,
                ))
            }
            Ok(None) => {
                if path.is_simple() {
                    Err(self.undefined_type_error(span, name))
                } else {
                    let module_str = path
                        .module_segments()
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::");
                    Err(ElabError::item_not_in_module(span, module_str, name))
                }
            }
            Err(PathResolutionError::ModuleNotFound(module)) => {
                Err(ElabError::module_not_found(span, module.to_string()))
            }
            Err(PathResolutionError::ItemNotFound { module, item }) => Err(
                ElabError::item_not_in_module(span, module.to_string(), item),
            ),
            Err(PathResolutionError::Ambiguous { item, candidates }) => {
                Err(ElabError::ambiguous_value(span, item, candidates))
            }
        }
    }

    /// Check type item visibility, returning an error if private.
    fn check_type_item_visibility(
        &self,
        type_name: &str,
        visibility: crate::ast::Visibility,
        span: crate::span::Span,
    ) -> ElabResult<()> {
        if let Some(item_module) = self.env.get_item_module(type_name) {
            // Re-export visibility capping (ADR 14.5.26c §2.3)
            let vis =
                self.env
                    .effective_type_visibility(type_name, visibility, &self.current_module);
            if !self
                .env
                .is_item_accessible(vis, item_module, &self.current_module)
            {
                return Err(ElabError::private_item(
                    span,
                    type_name,
                    "type",
                    item_module.to_string(),
                    self.current_module.to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Convert a resolved zero-param type def to a Type.
    fn type_def_to_type(
        &mut self,
        display_name: &str,
        type_name: &str,
        kind: &TypeDefKind,
        span: crate::span::Span,
    ) -> ElabResult<Type> {
        match kind {
            // A zero-parameter family's body is closed (ADR 4.10.26i D5).
            TypeDefKind::Alias(ty) | TypeDefKind::Family { body: ty, .. } => Ok(ty.clone()),
            TypeDefKind::ADT(_) => {
                // During Type-Body Collection (collection), defer ADT encoding as TyVar("@Name")
                // so mutual recursion groups can be computed first (ADR 18.4.26i §5).
                // Deferred-TyVar Resolution will resolve these to proper encodings.
                if self.collecting_type_bodies {
                    Ok(Type::TyVar(format!("@{}", type_name)))
                } else {
                    self.encode_adt_type(type_name, &[])
                }
            }
            TypeDefKind::Record(_) => Ok(Type::TyVar(format!("@{}", type_name))),
            TypeDefKind::Stub => {
                // IMPORTANT (ADR 30.1.26.1): Stubs may have incorrect params
                if let Some(actual_params) = self.resolve_stub_actual_params(type_name) {
                    if !actual_params.is_empty() {
                        return Err(ElabError::new(
                            span,
                            ElabErrorKind::ArityMismatch {
                                expected: actual_params.len(),
                                found: 0,
                            },
                        )
                        .with_note(format!(
                            "`{}` requires {} type parameter(s)",
                            display_name,
                            actual_params.len()
                        )));
                    }
                }
                Ok(Type::TyVar(format!("@{}", type_name)))
            }
        }
    }
}
