//! Type application: `Name<Arg, …>` against the definition it names, a
//! type variable, or the type being defined (ADR 6.10.26b D5).

use crate::ast::TypeExpr;
use crate::elaborate::env::TypeDefKind;
use crate::elaborate::error::{ElabError, ElabErrorKind, SortErrorKind};
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;
use crate::span::{Span, Spanned};
use albite_core::Type;

/// E0068: arguments on a name the definition in scope binds (ADR 6.10.26b D3).
fn type_parameter_applied(span: Span, name: &str, own_params: Option<Vec<String>>) -> ElabError {
    ElabError::new(
        span,
        ElabErrorKind::Sort(SortErrorKind::TypeParameterApplied {
            name: name.to_string(),
            own_params,
        }),
    )
}

/// How a recursive use of the type being defined passes its arguments
/// (ADR 6.10.26b D4).
#[derive(Debug, PartialEq, Eq)]
pub(in crate::elaborate::types) enum SelfReferenceArguments {
    /// Its own parameters, in order: `List<T>` inside `type List<T>`.
    Own,
    /// As many arguments as it has parameters, but not its own: `Nest<Nat>`.
    Other,
    /// A different number of arguments: `L<T, T>` inside `type L<T>`.
    WrongArity,
}

/// Classify `args` against the definition's own `params`. Only a bare name
/// equal to the parameter in its position counts as passing it.
pub(in crate::elaborate::types) fn classify_self_reference_arguments(
    params: &[String],
    args: &[TypeExpr],
) -> SelfReferenceArguments {
    if params.len() != args.len() {
        return SelfReferenceArguments::WrongArity;
    }
    let passes_own = |(param, arg): (&String, &TypeExpr)| match arg {
        TypeExpr::Path(path) => path.is_simple() && path.item_name().name == *param,
        _ => false,
    };
    if params.iter().zip(args).all(passes_own) {
        SelfReferenceArguments::Own
    } else {
        SelfReferenceArguments::Other
    }
}

/// A recursive use of the type being defined: its own parameters elaborate
/// to the self-reference `TyVar(name)`, exactly as before; any other
/// arguments are refused, since the μ-encoding would read them as its own
/// (ADR 6.10.26b D4).
fn self_reference_application(
    span: Span,
    name: &str,
    own_params: &[String],
    args: &[TypeExpr],
) -> ElabResult<Type> {
    match classify_self_reference_arguments(own_params, args) {
        SelfReferenceArguments::Own => Ok(Type::TyVar(name.to_string())),
        SelfReferenceArguments::Other => Err(type_parameter_applied(
            span,
            name,
            Some(own_params.to_vec()),
        )),
        SelfReferenceArguments::WrongArity => {
            Err(
                ElabError::arity_mismatch(span, own_params.len(), args.len()).with_note(format!(
                    "`{name}` is defined with {} type parameter(s)",
                    own_params.len()
                )),
            )
        }
    }
}

impl<'a> Elaborator<'a> {
    /// Elaborate a type application (e.g., `List<Nat>`).
    pub(in crate::elaborate::types) fn elab_type_app(
        &mut self,
        base: &TypeExpr,
        args: &[TypeExpr],
        span: crate::span::Span,
    ) -> ElabResult<Type> {
        // Get the base type name
        // Defensive: the grammar only parses `<…>` after a path, so a
        // non-path base cannot reach elaboration — uncoded by design
        // (ADR 15.8.26b).
        let TypeExpr::Path(base_path) = base else {
            return Err(ElabError::new(
                base.span(),
                ElabErrorKind::Other("expected type name in type application".to_string()),
            ));
        };

        let name = &base_path.item_name().name;

        // A name the definition in scope binds (ADR 6.10.26b D2). The parameter
        // stack is asked first: a parameter is the innermost binding, so in
        // `type F<F> = Mk(F<Nat>)` the field's `F` is the parameter.
        if base_path.is_simple() {
            // A predicate variable applied, asked before both of 6.10.26b's
            // branches: its arguments are terms (ADR 6.10.26h D3).
            if let Some(domains) = self.env.type_var_kind(name) {
                let domains = domains.to_vec();
                return self.elab_predicate_application(name, &domains, args, span);
            }
            if self.env.has_type_var(name) {
                return Err(type_parameter_applied(span, name, None)
                    .with_note("higher-kinded type parameters are not supported"));
            }
            if let Some(own_params) = self.env.self_reference_params(name) {
                return self_reference_application(span, name, own_params, args);
            }
        }

        // Look up the type definition using shared path resolution
        let (type_name, params_len, kind, visibility) =
            self.resolve_type_path_checked(base_path)?;

        // Get the params for alias substitution
        let params =
            if let Ok(Some(td)) = self.env.resolve_type_path(base_path, &self.current_module) {
                td.params.clone()
            } else {
                Vec::new()
            };

        // Check item visibility
        self.check_type_item_visibility(&type_name, visibility, span)?;
        self.check_application_arity(&kind, &type_name, params_len, args.len(), span)?;
        if let TypeDefKind::Family { params, body } = &kind {
            return self.expand_family(name, params, body, args); // arguments are terms
        }

        // Elaborate argument types
        let mut arg_types = Vec::with_capacity(args.len());
        for arg in args {
            arg_types.push(self.elab_type(arg)?);
        }

        // Apply the type arguments
        match &kind {
            TypeDefKind::Alias(ty) => {
                // Substitute type parameters
                let mut result = ty.clone();
                for (param, arg) in params.iter().zip(arg_types.iter()) {
                    result = result.substitute(param, arg);
                }
                Ok(result)
            }
            TypeDefKind::ADT(_) => {
                // During Type-Body Collection, defer ADT encoding (ADR 18.4.26i §5)
                if self.collecting_type_bodies {
                    Ok(Type::app(name.to_string(), arg_types))
                } else {
                    self.encode_adt_type(name, &arg_types)
                }
            }
            TypeDefKind::Record(_) => {
                // For records, return the nominal type as Type::App
                // (Records don't support type parameters currently,
                // but use App for consistency with ADTs)
                // The encoding to Product happens at codegen time
                Ok(Type::app(type_name, arg_types))
            }
            TypeDefKind::Stub | TypeDefKind::Family { .. } => {
                // Stub type with type arguments - create a deferred application
                // This will be resolved in Deferred-TyVar Resolution after all types are elaborated
                Ok(Type::app(name.to_string(), arg_types))
            }
        }
    }

    /// E0012 unless `found` arguments match the arity `type_name` declares.
    fn check_application_arity(
        &self,
        kind: &TypeDefKind,
        type_name: &str,
        params_len: usize,
        found: usize,
        span: Span,
    ) -> ElabResult<()> {
        // IMPORTANT (ADR 30.1.26.1): For stubs, the params may be incorrect (Vec::new())
        // if created during module discovery. Re-resolve to find actual arity.
        let resolved_stub_params =
            matches!(kind, TypeDefKind::Stub).then(|| self.resolve_stub_actual_params(type_name));
        let expected = match &resolved_stub_params {
            Some(Some(params)) => params.len(),
            _ => params_len,
        };
        if expected == found {
            return Ok(());
        }
        let error = ElabError::arity_mismatch(span, expected, found);
        // A stub nothing resolves is a type not yet collected: in a family
        // body, a family declared below this one (ADR 6.10.26d D4).
        Err(if matches!(resolved_stub_params, Some(None)) {
            error.with_note(format!(
                "`{type_name}` is not collected yet at this use: if it is declared below, \
                 declare it before citing it"
            ))
        } else {
            error
        })
    }

    /// Resolve a stub type to find the actual type definition's parameters.
    ///
    /// Stubs created during module discovery may have incorrect params (Vec::new())
    /// because the actual type definition wasn't available yet. This method follows
    /// imports to find the real type definition and returns its actual parameters.
    ///
    /// See ADR 30.1.26.1 for details on this fix.
    pub(super) fn resolve_stub_actual_params(&self, type_name: &str) -> Option<Vec<String>> {
        // 1. Check if there's an import for this type in the current module
        if let Some(import_info) = self.env.lookup_type_import(&self.current_module, type_name) {
            // 2. Resolve to the source module's actual type definition
            if let Some(actual_def) = self
                .env
                .lookup_type_in_module(&import_info.source_module, &import_info.original_name)
            {
                // Only use if it's not also a stub (avoid infinite loops)
                if !matches!(actual_def.kind, TypeDefKind::Stub) {
                    return Some(actual_def.params.clone());
                }
            }
        }

        // 3. Also try direct lookup - the type might have been elaborated after the stub was created
        if let Some(type_def) = self.env.lookup_type(type_name) {
            if !matches!(type_def.kind, TypeDefKind::Stub) {
                return Some(type_def.params.clone());
            }
        }

        None
    }
}
