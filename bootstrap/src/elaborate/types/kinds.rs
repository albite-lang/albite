//! Predicate parameters at the surface (ADR 6.10.26h D2, D3, D5).
//!
//! `<P: Nat -> Prop>` gives a type parameter a *kind*: a non-empty list of
//! closed data domains ending in `Prop`. Such a `P` is applied to terms,
//! `P<n>`, which elaborates to `PredApp(P, [n])`, and a citation passes it
//! with the turbofish at a statement family or at a predicate parameter in
//! scope, which elaborates to the `PredAbs` the kernel instantiates with.
//! Every other kind is E0069, one message per case.

use crate::ast::{Path, TypeExpr, TypeParam};
use crate::elaborate::env::TypeDefKind;
use crate::elaborate::error::{ConfusedSlot, ElabError, ElabErrorKind, KindRefusal, SortErrorKind};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::{Span, Spanned};
use albite_core::types::Sort;
use albite_core::{Term, Type};

use super::families::term_type_confusion;

/// The prefix of the parameters an in-scope predicate is eta-expanded over.
/// `$` cannot occur in a source identifier, so no argument can capture one.
const FORWARDED_PARAMETER_PREFIX: &str = "pred$";

/// E0069 for `param`'s kind, at `span`.
fn kind_refused(span: Span, param: &str, why: KindRefusal) -> ElabError {
    ElabError::new(
        span,
        ElabErrorKind::Sort(SortErrorKind::PredicateKindRefused {
            param: param.to_string(),
            why,
        }),
    )
}

/// The written kind split at its arrows: the domains, and whether the last
/// position is `Prop`. Parentheses are seen through.
pub(in crate::elaborate) fn kind_shape(bound: &TypeExpr) -> (Vec<&TypeExpr>, bool) {
    let mut domains = Vec::new();
    let mut current = bound;
    loop {
        match current {
            TypeExpr::Paren(inner, _) => current = inner,
            TypeExpr::Arrow(domain, rest, _) => {
                domains.push(&**domain);
                current = rest;
            }
            TypeExpr::Prop(_) => return (domains, true),
            _ => return (domains, false),
        }
    }
}

impl Elaborator<'_> {
    /// The kind of `param`: `None` for a plain type parameter, the domains
    /// for a predicate parameter. Type parameters to its left must already
    /// be in scope, so a domain naming one is seen (D2).
    pub(in crate::elaborate) fn elab_type_param_kind(
        &mut self,
        param: &TypeParam,
    ) -> ElabResult<Option<Vec<Type>>> {
        let Some(bound) = param.bounds.first() else {
            return Ok(None);
        };
        let name = &param.name.name;
        let refuse = |why| kind_refused(bound.span(), name, why);
        let (written, ends_in_prop) = kind_shape(bound);
        match (written.is_empty(), ends_in_prop) {
            (true, true) => return Err(refuse(KindRefusal::RangesOverPropositions)),
            (true, false) => return Err(refuse(KindRefusal::NotAKind)),
            (false, false) => return Err(refuse(KindRefusal::CodomainNotProp)),
            (false, true) => {}
        }
        let mut domains = Vec::with_capacity(written.len());
        for domain in written {
            let ty = self.elab_type(domain)?;
            if let Some(parameter) = ty
                .free_type_vars()
                .into_iter()
                .find(|v| self.env.has_type_var(v))
            {
                return Err(refuse(KindRefusal::DomainNamesTypeParameter {
                    domain: parameter,
                }));
            }
            if ty.sort_of() == Some(Sort::Prop) {
                return Err(refuse(KindRefusal::DomainIsProposition { domain: ty }));
            }
            domains.push(ty);
        }
        Ok(Some(domains))
    }

    /// E0069 when any of a type definition's parameters has a kind (D2).
    pub(in crate::elaborate) fn refuse_kinded_type_params(
        &self,
        type_name: &str,
        params: &[TypeParam],
    ) -> ElabResult<()> {
        match params.iter().find(|p| !p.bounds.is_empty()) {
            Some(param) => Err(kind_refused(
                param.bounds[0].span(),
                &param.name.name,
                KindRefusal::OnTypeDefinition {
                    type_name: type_name.to_string(),
                },
            )),
            None => Ok(()),
        }
    }

    /// `P<args>` for a predicate variable `P` over `domains` (D3): each
    /// argument re-read as a term and checked against its domain, as a
    /// family's are; E0012 on a different count.
    pub(in crate::elaborate::types) fn elab_predicate_application(
        &mut self,
        pred: &str,
        domains: &[Type],
        args: &[TypeExpr],
        span: Span,
    ) -> ElabResult<Type> {
        if domains.len() != args.len() {
            return Err(
                ElabError::arity_mismatch(span, domains.len(), args.len()).with_note(format!(
                    "`{pred}` is a predicate over {} argument(s)",
                    domains.len()
                )),
            );
        }
        let mut terms = Vec::with_capacity(args.len());
        for (position, (domain, arg)) in domains.iter().zip(args).enumerate() {
            let refuse = |misplaced: &TypeExpr| {
                term_type_confusion(
                    misplaced,
                    ConfusedSlot::FamilyParameter {
                        family: pred.to_string(),
                        param: format!("#{}", position + 1),
                        param_ty: domain.clone(),
                    },
                )
            };
            let expr = self.family_argument_expr(arg, &refuse)?;
            terms.push(self.check(&expr, domain)?);
        }
        Ok(Type::pred_app(pred, terms))
    }

    /// A turbofish argument against a predicate parameter over `domains`
    /// (D5): a statement family whose parameter types are the domains, or a
    /// predicate parameter in scope of the same kind. No predicate is ever
    /// inferred, and no other type is one.
    pub(crate) fn elab_predicate_argument(
        &mut self,
        arg: &TypeExpr,
        domains: &[Type],
    ) -> ElabResult<Type> {
        let not_a_predicate = |elab: &mut Self| {
            let found = elab.elab_type(arg).unwrap_or(Type::Error);
            ElabError::new(
                arg.span(),
                ElabErrorKind::ExpectedType {
                    expected: "a predicate: a statement family, or a predicate parameter"
                        .to_string(),
                    found,
                },
            )
        };
        let TypeExpr::Path(path) = arg else {
            return Err(not_a_predicate(self));
        };
        if path.is_simple() {
            if let Some(kind) = self.env.type_var_kind(&path.item_name().name) {
                let kind = kind.to_vec();
                return forwarded_predicate(&path.item_name().name, &kind, domains, arg.span());
            }
        }
        match self.family_named(path) {
            Some((params, body)) => family_predicate(params, body, domains, arg.span()),
            None => Err(not_a_predicate(self)),
        }
    }

    /// The parameters and body of the statement family `path` names.
    fn family_named(&self, path: &Path) -> Option<(Vec<(String, Type)>, Type)> {
        let def = self
            .env
            .resolve_type_path(path, &self.current_module)
            .ok()
            .flatten()?;
        match &def.kind {
            TypeDefKind::Family { params, body } => Some((params.clone(), body.clone())),
            _ => None,
        }
    }
}

/// `λparams. body` when the family's parameter types are the domains.
fn family_predicate(
    params: Vec<(String, Type)>,
    body: Type,
    domains: &[Type],
    span: Span,
) -> ElabResult<Type> {
    check_kind_matches(params.iter().map(|(_, ty)| ty), domains, span)?;
    Ok(Type::pred_abs(params, body))
}

/// `λxs. Q<xs>` for a predicate parameter `Q` in scope of the same kind.
fn forwarded_predicate(
    pred: &str,
    kind: &[Type],
    domains: &[Type],
    span: Span,
) -> ElabResult<Type> {
    check_kind_matches(kind.iter(), domains, span)?;
    let params: Vec<(String, Type)> = kind
        .iter()
        .enumerate()
        .map(|(index, ty)| (format!("{FORWARDED_PARAMETER_PREFIX}{index}"), ty.clone()))
        .collect();
    let args = params.iter().map(|(x, _)| Term::var(x.clone())).collect();
    Ok(Type::pred_abs(params, Type::pred_app(pred, args)))
}

/// E0012 on a different count of domains, E0010 on a different domain.
fn check_kind_matches<'t>(
    given: impl ExactSizeIterator<Item = &'t Type>,
    domains: &[Type],
    span: Span,
) -> ElabResult<()> {
    if given.len() != domains.len() {
        return Err(ElabError::arity_mismatch(span, domains.len(), given.len())
            .with_note("a predicate argument takes as many arguments as the parameter's kind"));
    }
    for (got, expected) in given.zip(domains) {
        if !albite_core::types_equal(got, expected) {
            return Err(ElabError::type_mismatch(
                span,
                expected.clone(),
                got.clone(),
            ));
        }
    }
    Ok(())
}

/// The kinds of a definition's type parameters, read back from its
/// collected signature's prenex prefix: `Some(domains)` under a
/// `ForallPred`, `None` under a `Forall` (or past a poisoned signature).
pub(in crate::elaborate) fn kinds_from_signature(
    ty: &Type,
    count: usize,
) -> Vec<Option<Vec<Type>>> {
    let mut kinds = Vec::with_capacity(count);
    let mut current = ty;
    for _ in 0..count {
        match current {
            Type::ForallPred(_, domains, body) => {
                kinds.push(Some(domains.clone()));
                current = body;
            }
            Type::Forall(_, body) => {
                kinds.push(None);
                current = body;
            }
            _ => kinds.push(None),
        }
    }
    kinds
}
