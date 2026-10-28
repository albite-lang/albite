//! Phase 3: Arm Classification
//!
//! Groups match arms by constructor index vs catch-all patterns.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::ast::{self, Pattern};
use crate::span::{Span, Spanned};
use albite_core::Type;

use super::context::{AdtMatchContext, ClassifiedArms};
use super::ownership;
use crate::elaborate::env::{self as elab_env, ModulePath, PathResolutionError};
use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::{ElabResult, Elaborator};

impl<'a> Elaborator<'a> {
    /// Classify match arms by constructor index vs catch-all.
    ///
    /// Builds a map from constructor index to arm, and identifies the catch-all
    /// arm if present. Also emits warnings for unreachable arms.
    ///
    /// Every arm must name a constructor of `ctx.owner` (ADR 4.10.26d D1 a):
    /// an arm is filed under its index, and an index means nothing outside
    /// the type that assigned it.
    pub(super) fn classify_match_arms<'b>(
        &mut self,
        arms: &'b [ast::MatchArm],
        ctx: &AdtMatchContext,
        scrutinee_ty: &Type,
    ) -> ElabResult<ClassifiedArms<'b>> {
        let constructors = &ctx.constructors;
        let mut ctor_arms: HashMap<usize, Vec<&ast::MatchArm>> = HashMap::new();
        let mut catch_all_arm: Option<&ast::MatchArm> = None;
        let mut catch_all_span: Option<Span> = None;

        for arm in arms {
            match &arm.pattern {
                Pattern::Constructor(ref path, _, _) => {
                    if let Some(info) = self.classify_constructor_arm(arm, path, catch_all_span)? {
                        check_arm_owner(path.item_name(), &info.type_name, ctx, scrutinee_ty)?;
                        ctor_arms.entry(info.index).or_default().push(arm);
                    }
                }
                Pattern::Var(ref ident) => {
                    // Check if this variable name is actually a nullary constructor
                    // (the parser can't distinguish `Zero` as constructor vs variable)
                    if let Some(idx) = constructors
                        .iter()
                        .position(|c| c.name == ident.name && c.fields.is_empty())
                    {
                        if let Some(catch_span) = catch_all_span {
                            self.warn(
                                ElabError::new(arm.pattern.span(), ElabErrorKind::UnreachableArm)
                                    .with_span_note(
                                        catch_span,
                                        "this catch-all pattern matches all values",
                                    )
                                    .with_help(
                                        "remove this arm or move the catch-all pattern to the end",
                                    ),
                            );
                        } else {
                            ctor_arms.entry(idx).or_default().push(arm);
                        }
                    } else if let Some(with_fields) =
                        constructors.iter().find(|c| c.name == ident.name)
                    {
                        // A bare name for one of this type's constructors
                        // with fields is `Wb()`'s arity error, not a binder
                        // that takes every value (ADR 6.10.26k D2 c).
                        return Err(ElabError::new(
                            ident.span,
                            ElabErrorKind::ArityMismatch {
                                expected: with_fields.fields.len(),
                                found: 0,
                            },
                        ));
                    } else {
                        // A binder spelled like another type's nullary
                        // constructor names that constructor (D1 a); a name
                        // no constructor owns stays a binding (3.10.26l).
                        if let Some(foreign) = self.nullary_constructor_owner(ident) {
                            check_arm_owner(ident, &foreign, ctx, scrutinee_ty)?;
                        }
                        self.classify_catch_all_arm(
                            arm,
                            ident.span,
                            &mut catch_all_arm,
                            &mut catch_all_span,
                        );
                    }
                }
                Pattern::Wildcard(s) => {
                    self.classify_catch_all_arm(arm, *s, &mut catch_all_arm, &mut catch_all_span);
                }
                _ => {
                    // Defensive: exotic pattern forms are rejected with E0021
                    // before classification — uncoded by design (ADR 15.8.26b).
                    return Err(ElabError::new(
                        arm.pattern.span(),
                        ElabErrorKind::Other("unsupported pattern in match arm".to_string()),
                    ));
                }
            }
        }

        Ok(ClassifiedArms {
            ctor_arms,
            catch_all: catch_all_arm,
        })
    }

    /// Whether `ident`, read as a one-segment path from the current module,
    /// names a nullary constructor of any type.
    pub(super) fn names_nullary_constructor(&self, ident: &ast::Ident) -> bool {
        self.nullary_constructor_owner(ident).is_some()
    }

    /// The owner of the nullary constructor `ident` names, if it names one.
    fn nullary_constructor_owner(&self, ident: &ast::Ident) -> Option<String> {
        let path = ast::Path::simple(ident.clone());
        match self
            .env
            .resolve_constructor_path(&path, &self.current_module)
        {
            Ok(Some(info)) if info.arity == 0 => Some(info.type_name.clone()),
            _ => None,
        }
    }

    /// Whether a bare name in a match arm names a constructor, so the match
    /// belongs to the constructor matcher (ADR 6.10.26k D2): a nullary
    /// constructor of any type, or (D2 c) a constructor with fields whose
    /// type is the scrutinee's. Any other bare name is a binder.
    pub(in crate::elaborate::exprs) fn bare_name_routes_to_constructor_matcher(
        &self,
        ident: &ast::Ident,
        scrutinee_ty: &Type,
    ) -> bool {
        let path = ast::Path::simple(ident.clone());
        let Ok(Some(info)) = self
            .env
            .resolve_constructor_path(&path, &self.current_module)
        else {
            return false;
        };
        if info.arity == 0 {
            return true;
        }
        let Some(elab_env::TypeDefKind::ADT(ctors)) =
            self.env.lookup_type(&info.type_name).map(|def| &def.kind)
        else {
            return false;
        };
        let normalized = self.normalize_for_comparison(scrutinee_ty);
        ownership::scrutinee_owned_by(ctors, &normalized)
    }

    /// Resolve a constructor pattern arm; `None` when a preceding catch-all
    /// makes it unreachable (warned, not filed).
    fn classify_constructor_arm(
        &mut self,
        arm: &ast::MatchArm,
        path: &ast::Path,
        catch_all_span: Option<Span>,
    ) -> ElabResult<Option<elab_env::ConstructorInfo>> {
        let name = path.item_name();

        // Warn if this arm is unreachable due to preceding catch-all
        if let Some(catch_span) = catch_all_span {
            self.warn(
                ElabError::new(arm.pattern.span(), ElabErrorKind::UnreachableArm)
                    .with_span_note(catch_span, "this catch-all pattern matches all values")
                    .with_help("remove this arm or move the catch-all pattern to the end"),
            );
            return Ok(None);
        }

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

        // Resolve the constructor (multiple arms may exist for same constructor)
        let info = match self
            .env
            .resolve_constructor_path(path, &self.current_module)
        {
            Ok(Some(info)) => info.clone(),
            Ok(None) => return Err(self.undefined_constructor_error(name.span, &name.name)),
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
        };

        Ok(Some(info))
    }

    /// Classify a catch-all (wildcard or variable) arm.
    fn classify_catch_all_arm<'b>(
        &mut self,
        arm: &'b ast::MatchArm,
        pattern_span: Span,
        catch_all_arm: &mut Option<&'b ast::MatchArm>,
        catch_all_span: &mut Option<Span>,
    ) {
        if catch_all_arm.is_some() {
            // Multiple catch-alls - warn about unreachable
            self.warn(
                ElabError::new(arm.pattern.span(), ElabErrorKind::UnreachableArm)
                    .with_span_note(
                        catch_all_span.unwrap(),
                        "previous catch-all pattern already matches all values",
                    )
                    .with_help("remove this redundant catch-all pattern"),
            );
            return;
        }

        *catch_all_arm = Some(arm);
        *catch_all_span = Some(pattern_span);
    }
}

/// Rewrite every bare name inside a constructor pattern's sub-patterns that
/// `is_nullary` resolves into that constructor's empty-bracket pattern, so
/// `Som(A)` reads as `Som(A())` and `Cons(x, Nil)` as `Cons(x, Nil())`
/// (ADR 13.10.26h D1). A top-level bare name is left alone: classification
/// already files it by name (6.10.26k). A name no nullary constructor owns
/// stays a binder.
///
/// Borrowed when nothing is rewritten, so the common match never clones
/// its arm bodies.
pub(super) fn name_bare_nullary_subpatterns(
    arms: &[ast::MatchArm],
    is_nullary: impl Fn(&ast::Ident) -> bool,
) -> Cow<'_, [ast::MatchArm]> {
    let rewrites = |arm: &ast::MatchArm| {
        matches!(arm.pattern, Pattern::Constructor(..))
            && has_bare_nullary_inner(&arm.pattern, &is_nullary)
    };
    if !arms.iter().any(rewrites) {
        return Cow::Borrowed(arms);
    }
    let named = arms
        .iter()
        .map(|arm| match &arm.pattern {
            Pattern::Constructor(..) => ast::MatchArm {
                pattern: name_nested_nullary(&arm.pattern, &is_nullary),
                ..arm.clone()
            },
            _ => arm.clone(),
        })
        .collect();
    Cow::Owned(named)
}

/// Whether D1's rewrite changes anything below `pattern`.
fn has_bare_nullary_inner(pattern: &Pattern, is_nullary: &impl Fn(&ast::Ident) -> bool) -> bool {
    let inner = |p: &Pattern| match p {
        Pattern::Var(ident) => is_nullary(ident),
        other => has_bare_nullary_inner(other, is_nullary),
    };
    match pattern {
        Pattern::Constructor(_, subs, _) | Pattern::Tuple(subs, _) => subs.iter().any(inner),
        Pattern::Or(left, right, _) => inner(left) || inner(right),
        _ => false,
    }
}

/// D1's rewrite below a constructor pattern, at any depth.
fn name_nested_nullary(pattern: &Pattern, is_nullary: &impl Fn(&ast::Ident) -> bool) -> Pattern {
    let rewrite_all = |subs: &[Pattern]| -> Vec<Pattern> {
        subs.iter()
            .map(|p| name_bare_inner(p, is_nullary))
            .collect()
    };
    match pattern {
        Pattern::Constructor(path, subs, span) => {
            Pattern::Constructor(path.clone(), rewrite_all(subs), *span)
        }
        Pattern::Tuple(subs, span) => Pattern::Tuple(rewrite_all(subs), *span),
        Pattern::Or(left, right, span) => Pattern::Or(
            Box::new(name_bare_inner(left, is_nullary)),
            Box::new(name_bare_inner(right, is_nullary)),
            *span,
        ),
        other => other.clone(),
    }
}

/// One sub-pattern: a bare nullary name becomes its bracketed spelling.
fn name_bare_inner(pattern: &Pattern, is_nullary: &impl Fn(&ast::Ident) -> bool) -> Pattern {
    match pattern {
        Pattern::Var(ident) if is_nullary(ident) => {
            Pattern::Constructor(ast::Path::simple(ident.clone()), Vec::new(), ident.span)
        }
        other => name_nested_nullary(other, is_nullary),
    }
}

/// D1(a)'s refusal: `ctor` belongs to `arm_owner`, not to the type the match
/// was resolved to. E0022 names the arm's own type against the scrutinee, so
/// the headline stays true when the scrutinee agrees with the first arm, and
/// the note names the type the match was taken from (D2).
fn check_arm_owner(
    ctor: &ast::Ident,
    arm_owner: &str,
    ctx: &AdtMatchContext,
    scrutinee_ty: &Type,
) -> ElabResult<()> {
    if ownership::owner_agrees(&ctx.owner, arm_owner) {
        return Ok(());
    }
    Err(ElabError::new(
        ctor.span,
        ElabErrorKind::MatchScrutineeNotAdt {
            adt_name: Some(arm_owner.to_string()),
            found: scrutinee_ty.clone(),
        },
    )
    .with_note(format!(
        "`{}` is a constructor of `{}`, but this match is over `{}`, taken from its first \
         constructor pattern",
        ctor.name, arm_owner, ctx.owner
    )))
}
