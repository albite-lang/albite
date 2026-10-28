//! Path resolution and visibility checking for check/infer.

use crate::ast::Expr;
use crate::elaborate::conversion::eq_fallback;
use crate::elaborate::env::{ModulePath, PathResolutionError, ResolvedValue};
use crate::elaborate::error::ElabError;
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;
use crate::span::Spanned;
use albite_core::{Term, Type};

impl<'a> Elaborator<'a> {
    /// Infer the type of a path expression (variable, global, or constructor).
    pub(crate) fn infer_path(&mut self, path: &crate::ast::Path) -> ElabResult<(Term, Type)> {
        let name = &path.item_name().name;

        // Check module visibility for qualified paths
        if !path.is_simple() {
            self.check_qualified_path_visibility(path)?;
        }

        // Use path resolution (handles both simple and qualified paths)
        match self
            .env
            .resolve_value_path(path, self.depth, &self.current_module)
        {
            Ok(Some(ResolvedValue::Local(_idx, ty))) => {
                self.check_not_captured_mutable(name, path.span)?;
                Ok((Term::var(name), ty))
            }
            Ok(Some(ResolvedValue::Global(global_name, ty))) => {
                // Check item visibility
                // Read from the resolved definition, so a private namesake in
                // another module is never the one judged (ADR 4.10.26j D2).
                if let Some(value_def) = self.env.lookup_value(&global_name) {
                    if let Some(item_module) = self.env.value_module(&global_name) {
                        // Apply re-export visibility capping (ADR 14.5.26c §2.3)
                        let effective_vis = self.env.effective_value_visibility(
                            name,
                            value_def.visibility,
                            &self.current_module,
                        );
                        if !self.env.is_item_accessible(
                            effective_vis,
                            item_module,
                            &self.current_module,
                        ) {
                            return Err(ElabError::private_item(
                                path.span,
                                self.env.value_source_name(&global_name),
                                "function",
                                item_module.to_string(),
                                self.current_module.to_string(),
                            ));
                        }
                    }
                }
                Ok((Term::global(global_name), ty))
            }
            Ok(Some(ResolvedValue::Constructor(info))) => {
                self.check_constructor_visibility(name, &info, path.span)?;
                self.elab_constructor_ref(name, &info, path.span)
            }
            Ok(None) => {
                if path.is_simple() {
                    Err(self.bare_name_not_found(path.span, name))
                } else {
                    let module_str = path
                        .module_segments()
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join("::");
                    Err(ElabError::item_not_in_module(path.span, module_str, name))
                }
            }
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

    /// E0001 for a bare name — and when it is defined, but in a package the
    /// current one did not declare, a help line saying which (ADR 3.10.26m
    /// D5): to this package the name does not exist.
    fn bare_name_not_found(&self, span: crate::span::Span, name: &str) -> ElabError {
        let err = self.undefined_variable_error(span, name);
        let module = &self.current_module;
        match self.env.unreachable_bare_name(module, name) {
            Some(owner) => err.with_help(self.env.undeclared_package_help(module, name, owner)),
            None => err,
        }
    }

    /// Check a path expression against an expected type.
    /// Handles constructor resolution with type-directed checking.
    pub(crate) fn check_path(
        &mut self,
        path: &crate::ast::Path,
        expected: &Type,
        expr: &Expr,
    ) -> ElabResult<Term> {
        // Check module visibility for qualified paths
        if !path.is_simple() {
            self.check_qualified_path_visibility(path)?;
        }

        // Check if this is a constructor using path resolution
        let resolution = self
            .env
            .resolve_value_path(path, self.depth, &self.current_module);
        match resolution {
            Ok(Some(ResolvedValue::Constructor(info))) => {
                let name = path.item_name().name.clone();
                self.check_constructor_visibility(&name, &info, path.span)?;
                return self.check_constructor_ref(&name, &info, expected, path.span);
            }
            Ok(Some(_) | None) => {}
            Err(PathResolutionError::ModuleNotFound(module)) => {
                return Err(ElabError::module_not_found(path.span, module.to_string()));
            }
            Err(PathResolutionError::ItemNotFound { module, item }) => {
                return Err(ElabError::item_not_in_module(
                    path.span,
                    module.to_string(),
                    item,
                ));
            }
            Err(PathResolutionError::Ambiguous { item, candidates }) => {
                return Err(ElabError::ambiguous_value(path.span, item, candidates));
            }
        }
        // Not a constructor — infer and compare
        self.check_via_infer(expr, expected)
    }

    /// Check an application expression against an expected type.
    /// Handles constructor application with type-directed checking.
    pub(crate) fn check_app(
        &mut self,
        func: &Expr,
        args: &[Expr],
        expected: &Type,
        expr: &Expr,
        span: crate::span::Span,
    ) -> ElabResult<Term> {
        if let Expr::Path(path) = func {
            if path.is_simple() {
                let ident = path.item_name();
                // A constructor in an undeclared package, not bound by a `use`,
                // takes the ordinary path, which refuses it (ADR 3.10.26m D5).
                let module = &self.current_module;
                let reachable = |info: &crate::elaborate::env::ConstructorInfo| {
                    self.env
                        .lookup_constructor_import(module, &ident.name)
                        .is_some()
                        || self.env.unreachable_constructor(module, info).is_none()
                };
                let found = self
                    .env
                    .lookup_constructor(&ident.name)
                    .filter(|i| reachable(i));
                if let Some(info) = found.cloned() {
                    self.check_constructor_visibility(&ident.name, &info, path.span)?;
                    return self.check_constructor_application(
                        &ident.name,
                        &info,
                        args,
                        expected,
                        span,
                    );
                }
            }
        }
        // Not a constructor: the built-in `inl`/`inr` (ADR 7.10.26l D2) or
        // `absurd` (ADR 7.10.26m D4), where lookup fails, else infer and compare.
        if let Some(term) = self.check_sum_intro(func, args, expected)? {
            return Ok(term);
        }
        if let Some(term) = self.check_absurd(func, args, expected)? {
            return Ok(term);
        }
        self.check_via_infer(expr, expected)
    }

    /// Infer type and verify it matches expected — shared fallthrough for check
    /// arms. An `Eq` against an `Eq` converts its endpoints (ADR 30.9.26e D5),
    /// and a nested `Eq`'s closed endpoints convert (ADR 7.10.26i D1).
    fn check_via_infer(&mut self, expr: &Expr, expected: &Type) -> ElabResult<Term> {
        let (term, inferred) = self.infer(expr)?;
        if !self.types_equal(&inferred, expected) {
            let mismatch = |this: &Self| this.check_via_infer_mismatch(expr, expected, &inferred);
            if eq_fallback(&inferred, expected) {
                self.check_eq_against(&inferred, expected, mismatch)?;
            } else if !self.closed_endpoints_convert(&inferred, expected) {
                return Err(mismatch(self));
            }
        }
        Ok(term)
    }

    /// `check_via_infer`'s refusal, built only when one is raised.
    fn check_via_infer_mismatch(&self, expr: &Expr, expected: &Type, inferred: &Type) -> ElabError {
        let mut err = self.type_mismatch_error(expr.span(), expected.clone(), inferred.clone());

        // Cross-file enrichment for function calls (ADR 15.5.26a).
        // Note: only covers direct calls (Expr::App(Expr::Path(...), ...)).
        // Higher-order calls like `let f = get_value; f()` won't get
        // cross-file notes or trace frames.
        if let Expr::App(func, _, call_span) = expr {
            if let Expr::Path(path) = func.as_ref() {
                let name = &path.item_name().name;
                if let Some((file_path, def_span)) = self.cross_file_info_for_function(name) {
                    err = err
                        .with_cross_file_note(
                            def_span,
                            file_path.clone(),
                            format!("return type declared in `{}`", name),
                        )
                        .with_trace_frame(
                            *call_span,
                            self.get_current_file().unwrap_or_default(),
                            format!("{} call (expected {})", name, expected),
                        )
                        .with_trace_frame(
                            def_span,
                            file_path,
                            format!("return type declared in `{}`", name),
                        );
                }
            }
        }
        err
    }

    /// Check module visibility for a qualified path, returning an error if private.
    fn check_qualified_path_visibility(&self, path: &crate::ast::Path) -> ElabResult<()> {
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
        Ok(())
    }

    /// Check constructor visibility, returning an error if private.
    fn check_constructor_visibility(
        &self,
        name: &str,
        info: &crate::elaborate::env::ConstructorInfo,
        span: crate::span::Span,
    ) -> ElabResult<()> {
        // First check the base constructor accessibility
        if !self
            .env
            .is_constructor_accessible(info, &self.current_module)
        {
            if let Some(item_module) = self.env.get_item_module(&info.type_name) {
                return Err(ElabError::private_item(
                    span,
                    name,
                    "constructor",
                    item_module.to_string(),
                    self.current_module.to_string(),
                ));
            }
        }
        // Also check re-export visibility capping (ADR 14.5.26c §2.3)
        if let Some(ctor_vis) = self.env.get_constructor_visibility(info) {
            let effective_vis =
                self.env
                    .effective_constructor_visibility(name, ctor_vis, &self.current_module);
            if let Some(item_module) = self.env.get_item_module(&info.type_name) {
                if !self
                    .env
                    .is_item_accessible(effective_vis, item_module, &self.current_module)
                {
                    return Err(ElabError::private_item(
                        span,
                        name,
                        "constructor",
                        item_module.to_string(),
                        self.current_module.to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}
