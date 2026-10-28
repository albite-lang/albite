//! Pass 1: Definition Collection
//!
//! Collects type and value signatures from top-level items so that
//! items can refer to each other (forward references).

use crate::ast;

use albite_core::types::Sort;
use albite_core::Type;

use crate::elaborate::env::{Constructor, TypeDef, TypeDefKind, ValueDef};
use crate::elaborate::error::{
    ElabError, ElabErrorKind, SortErrorKind, FAMILY_BODY_MUST_BE, FAMILY_PARAMETER_MUST_BE,
};
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;
use crate::span::{Span, Spanned};

/// E0067: a family part whose sort is not the one it must have (ADR 4.10.26i D5).
fn family_sort_mismatch(
    span: Span,
    family: &str,
    part: String,
    ty: Type,
    expected: &str,
) -> ElabError {
    ElabError::new(
        span,
        ElabErrorKind::Sort(SortErrorKind::FamilySortMismatch {
            family: family.to_string(),
            part,
            ty,
            expected: expected.to_string(),
        }),
    )
}

/// E0025 (ADR 7.10.26n D2): a declared field whose type holds a `Pi`. The
/// encoding never meets a binder at a field site; a `Pi` that reaches a
/// payload by instantiation (`Option<(n: Nat) -> …>`) is admitted.
fn refuse_dependent_field(span: Span, field: String, ty: &Type) -> ElabResult<()> {
    if !mentions_pi(ty) {
        return Ok(());
    }
    Err(ElabError::new(
        span,
        ElabErrorKind::Sort(SortErrorKind::DependentFunctionField {
            field,
            ty: ty.clone(),
        }),
    ))
}

fn mentions_pi(ty: &Type) -> bool {
    matches!(ty, Type::Pi(..)) || ty.children().into_iter().any(mentions_pi)
}

impl<'a> Elaborator<'a> {
    pub(super) fn collect_function(&mut self, func: &ast::FunctionDef) -> ElabResult<()> {
        // Check for duplicate (allow overwrite if Stub Registration stub, ADR 5.5.26c)
        let core_name = self.top_level_core_name_checked(&func.name)?;
        if self.env.has_value(&core_name) && !self.allow_value_overwrite {
            return Err(ElabError::duplicate(func.name.span, &func.name.name));
        }

        // Build function type
        let ty = self.build_function_type(func)?;

        self.env.define_value(ValueDef {
            name: core_name,
            ty,
            visibility: func.visibility,
            span: func.span,
        });

        Ok(())
    }

    pub(super) fn collect_type_def(&mut self, type_def: &ast::TypeDef) -> ElabResult<()> {
        // Check for duplicate (but allow replacing stubs from Type-Name Registration
        // and Stub Registration placeholders from per-module elaboration, ADR 5.5.26c)
        if let Some(existing) = self.env.lookup_type(&type_def.name.name) {
            if !existing.is_overwritable_by_collection() {
                return Err(ElabError::duplicate(
                    type_def.name.span,
                    &type_def.name.name,
                ));
            }
        }
        // A predicate parameter is prenex in a definition, never data's (ADR 6.10.26h D2)
        self.refuse_kinded_type_params(&type_def.name.name, &type_def.type_params)?;

        // Extract type parameters
        let params: Vec<String> = type_def
            .type_params
            .iter()
            .map(|p| p.name.name.clone())
            .collect();

        // The type's own name is a self-reference, kept apart from its
        // parameters, and both are popped on every path, the error path
        // included (ADR 6.10.26b D1).
        self.env
            .push_self_reference(type_def.name.name.clone(), params.clone());
        for param in &params {
            self.env.push_type_var(param.clone());
        }
        let collected = self.collect_type_body(&type_def.body);
        for _ in &params {
            self.env.pop_type_var();
        }
        self.env.pop_self_reference();
        let type_def_kind = collected?;

        let field_visibilities = match &type_def.body {
            ast::TypeBody::Record(record_fields) => {
                record_fields.iter().map(|f| f.visibility).collect()
            }
            ast::TypeBody::Sum(_) => Vec::new(),
        };

        self.env.define_type(TypeDef {
            name: type_def.name.name.clone(),
            params,
            kind: type_def_kind,
            visibility: type_def.visibility,
            span: type_def.span,
            defining_module: None,
            encoded_type: None,
            field_visibilities,
        });

        Ok(())
    }

    /// A type definition's constructors or fields, elaborated with its
    /// parameters and self-reference in scope. The caller owns the scope.
    fn collect_type_body(&mut self, body: &ast::TypeBody) -> ElabResult<TypeDefKind> {
        Ok(match body {
            ast::TypeBody::Sum(variants) => {
                // Elaborate sum type variants
                let mut constructors = Vec::new();
                for (index, variant) in variants.iter().enumerate() {
                    let mut fields = Vec::with_capacity(variant.fields.len());
                    for (position, field) in variant.fields.iter().enumerate() {
                        let ty = self.elab_type(&field.ty)?;
                        let label = format!("`{}`'s field {}", variant.name.name, position + 1);
                        refuse_dependent_field(field.ty.span(), label, &ty)?;
                        fields.push(ty);
                    }

                    constructors.push(Constructor {
                        name: variant.name.name.clone(),
                        fields,
                        index,
                        visibility: variant.visibility,
                        span: variant.span,
                    });
                }
                TypeDefKind::ADT(constructors)
            }
            ast::TypeBody::Record(record_fields) => {
                // Elaborate record type fields
                let mut fields = Vec::new();
                for field in record_fields {
                    let ty = self.elab_type(&field.ty)?;
                    let label = format!("field `{}`", field.name.name);
                    refuse_dependent_field(field.ty.span(), label, &ty)?;
                    fields.push((field.name.name.clone(), ty));
                }
                TypeDefKind::Record(fields)
            }
        })
    }

    pub(super) fn collect_type_alias(&mut self, alias: &ast::TypeAlias) -> ElabResult<()> {
        // Check for duplicate (but allow replacing stubs from Type-Name Registration
        // and Stub Registration placeholders from per-module elaboration, ADR 5.5.26c)
        if let Some(existing) = self.env.lookup_type(&alias.name.name) {
            if !existing.is_overwritable_by_collection() {
                return Err(ElabError::duplicate(alias.name.span, &alias.name.name));
            }
        }
        self.refuse_kinded_type_params(&alias.name.name, &alias.type_params)?;

        if alias.sort.is_some() {
            return self.collect_sorted_alias(alias);
        }

        // Extract type parameters
        let params: Vec<String> = alias
            .type_params
            .iter()
            .map(|p| p.name.name.clone())
            .collect();

        // Push type variables for elaboration
        for param in &params {
            self.env.push_type_var(param.clone());
        }

        // Elaborate the aliased type; the pops run on the error path too
        let elaborated = self.elab_type(&alias.ty);
        for _ in &params {
            self.env.pop_type_var();
        }
        let ty = elaborated?;
        self.refuse_recursive_alias(alias, &ty)?;
        self.define_alias(alias, params, TypeDefKind::Alias(ty));
        Ok(())
    }

    /// `type Name(x: A, …): Prop = <type>` (ADR 4.10.26i D5): the parameters
    /// are bound as terms, as a signature's are, the body must sort `Prop`,
    /// and a definition with parameters is stored as a `Family`.
    fn collect_sorted_alias(&mut self, alias: &ast::TypeAlias) -> ElabResult<()> {
        let family = &alias.name.name;
        if alias.sort == Some(ast::SortAnnot::Type) && !alias.value_params.is_empty() {
            return Err(ElabError::unsupported(
                alias.span,
                "a value-indexed data type (`: Type` with parameters; owner: batch 8 planning)",
            ));
        }
        if !alias.type_params.is_empty() {
            return Err(ElabError::unsupported(
                alias.span,
                "type parameters on a sorted definition (owner: batch 8 planning)",
            ));
        }
        let saved_depth = self.depth;
        self.env.push_scope();
        let built = self.elab_family_signature(family, &alias.value_params, &alias.ty);
        self.env.pop_scope();
        self.depth = saved_depth;
        let (params, body) = built?;
        if alias.sort == Some(ast::SortAnnot::Prop) && body.sort_of() == Some(Sort::Type0) {
            return Err(family_sort_mismatch(
                alias.ty.span(),
                family,
                "the body".to_string(),
                body,
                FAMILY_BODY_MUST_BE,
            ));
        }
        self.refuse_recursive_alias(alias, &body)?;
        let names = params.iter().map(|(name, _)| name.clone()).collect();
        let kind = if params.is_empty() {
            TypeDefKind::Alias(body)
        } else {
            TypeDefKind::Family { params, body }
        };
        self.define_alias(alias, names, kind);
        Ok(())
    }

    /// Bind each parameter as a term — its type must be data — then elaborate
    /// the body with them in scope. The caller owns the scope.
    fn elab_family_signature(
        &mut self,
        family: &str,
        value_params: &[ast::Param],
        body: &ast::TypeExpr,
    ) -> ElabResult<(Vec<(String, Type)>, Type)> {
        let mut params = Vec::with_capacity(value_params.len());
        for param in value_params {
            let name = self.pattern_to_name(&param.pattern)?;
            let ty = self.elab_type(&param.ty)?;
            if ty.sort_of() == Some(Sort::Prop) {
                let part = format!("the type of parameter `{name}`");
                return Err(family_sort_mismatch(
                    param.ty.span(),
                    family,
                    part,
                    ty,
                    FAMILY_PARAMETER_MUST_BE,
                ));
            }
            self.env.bind_local(name.clone(), ty.clone(), self.depth);
            self.depth += 1;
            params.push((name, ty));
        }
        Ok((params, self.elab_type(body)?))
    }

    /// Reject recursive alias cycles (ADR 15.5.26g §2.4).
    fn refuse_recursive_alias(&self, alias: &ast::TypeAlias, ty: &Type) -> ElabResult<()> {
        let at_name = format!("@{}", alias.name.name);
        if ty.references_name(&alias.name.name) || ty.references_name(&at_name) {
            return Err(ElabError::new(
                alias.span,
                ElabErrorKind::RecursiveAlias(alias.name.name.clone()),
            ));
        }
        Ok(())
    }

    fn define_alias(&mut self, alias: &ast::TypeAlias, params: Vec<String>, kind: TypeDefKind) {
        self.env.define_type(TypeDef {
            name: alias.name.name.clone(),
            params,
            kind,
            visibility: alias.visibility,
            span: alias.span,
            defining_module: None,
            encoded_type: None,
            field_visibilities: Vec::new(),
        });
    }

    pub(super) fn collect_theorem(&mut self, thm: &ast::TheoremDef) -> ElabResult<()> {
        // Check for duplicate (allow overwrite if Stub Registration stub, ADR 5.5.26c)
        let core_name = self.top_level_core_name_checked(&thm.name)?;
        if self.env.has_value(&core_name) && !self.allow_value_overwrite {
            return Err(ElabError::duplicate(thm.name.span, &thm.name.name));
        }

        // Build theorem type: ∀ type_params. (param_types) → prop
        let ty = self.build_theorem_type(thm)?;

        self.env.define_value(ValueDef {
            name: core_name,
            ty,
            visibility: thm.visibility,
            span: thm.span,
        });

        Ok(())
    }

    pub(super) fn collect_axiom(&mut self, axiom: &ast::AxiomDef) -> ElabResult<()> {
        // Check for duplicate (allow overwrite if Stub Registration stub, ADR 5.5.26c)
        let core_name = self.top_level_core_name_checked(&axiom.name)?;
        if self.env.has_value(&core_name) && !self.allow_value_overwrite {
            return Err(ElabError::duplicate(axiom.name.span, &axiom.name.name));
        }

        // Build axiom type (same as theorem)
        let ty = self.build_axiom_type(axiom)?;

        self.env.define_value(ValueDef {
            name: core_name,
            ty,
            visibility: axiom.visibility,
            span: axiom.span,
        });

        Ok(())
    }
}
