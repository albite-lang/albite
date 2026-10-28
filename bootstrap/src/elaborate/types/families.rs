//! Statement-family expansion (ADR 4.10.26i D5, 6.10.26d D3).
//!
//! `type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m` is stored as a
//! `TypeDefKind::Family` whose body has `n` and `m` free. A use `Le<a, b>`
//! reads each argument as a TERM — any expression of the parameter's type —
//! and expands the body by substitution, so the kernel only ever sees the
//! `Sigma`/`Eq` and never the name `Le`.

use crate::ast::{BinOp, Expr, Path, TypeExpr};
use crate::elaborate::error::{ConfusedSlot, ElabError, ElabErrorKind, SortErrorKind};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::Spanned;
use albite_core::{Term, Type};

/// The prefix of the fresh names expansion renames each parameter to first.
/// `$` cannot occur in a source identifier, so no argument can capture one.
const FRESH_PARAMETER_PREFIX: &str = "family$";

impl Elaborator<'_> {
    /// Expand `family<args>`; the caller has checked the arity.
    pub(super) fn expand_family(
        &mut self,
        family: &str,
        params: &[(String, Type)],
        body: &Type,
        args: &[TypeExpr],
    ) -> ElabResult<Type> {
        let mut terms = Vec::with_capacity(args.len());
        for ((param, param_ty), arg) in params.iter().zip(args) {
            let refuse = |misplaced: &TypeExpr| {
                term_type_confusion(
                    misplaced,
                    ConfusedSlot::FamilyParameter {
                        family: family.to_string(),
                        param: param.clone(),
                        param_ty: param_ty.clone(),
                    },
                )
            };
            let expr = self.family_argument_expr(arg, &refuse)?;
            terms.push(self.check(&expr, param_ty)?);
        }
        Ok(expand_simultaneously(params, body, &terms))
    }

    /// One argument, re-read as the expression it spells (D3). The parser
    /// hands over a `Term` only when the tokens cannot be a type; `n + m` and
    /// `n * m` arrive as a `Sum` and a `Product`, whose precedences match
    /// `+` and `*`, so they re-read as `+` and `*`. A path that names only a
    /// type, and any other type former, is a type where a term belongs.
    /// Shared with a predicate variable's arguments (ADR 6.10.26h D3).
    pub(super) fn family_argument_expr(
        &self,
        arg: &TypeExpr,
        refuse: &dyn Fn(&TypeExpr) -> ElabError,
    ) -> ElabResult<Expr> {
        Ok(match arg {
            TypeExpr::Term(expr) => (**expr).clone(),
            TypeExpr::Path(path) if self.path_names_only_a_type(path) => return Err(refuse(arg)),
            TypeExpr::Path(path) => Expr::Path(path.clone()),
            TypeExpr::Sum(left, right, span) | TypeExpr::Product(left, right, span) => {
                let op = if matches!(arg, TypeExpr::Sum(..)) {
                    BinOp::Add
                } else {
                    BinOp::Mul
                };
                Expr::Binary(
                    Box::new(self.family_argument_expr(left, refuse)?),
                    op,
                    Box::new(self.family_argument_expr(right, refuse)?),
                    *span,
                )
            }
            TypeExpr::Paren(inner, span) => {
                Expr::Paren(Box::new(self.family_argument_expr(inner, refuse)?), *span)
            }
            _ => return Err(refuse(arg)),
        })
    }

    /// Whether `path` resolves as a type and as nothing a term can name. A
    /// path that is neither falls through to the expression, whose own
    /// lookup reports it unbound.
    fn path_names_only_a_type(&self, path: &Path) -> bool {
        let name = &path.item_name().name;
        let names_a_type = (path.is_simple() && Type::primitive_by_name(name).is_some())
            || matches!(
                self.env.resolve_type_path(path, &self.current_module),
                Ok(Some(_))
            );
        let names_a_term = (path.is_simple() && self.env.lookup_local(name).is_some())
            || matches!(
                self.env.resolve_value_path(path, 0, &self.current_module),
                Ok(Some(_))
            )
            || self.env.lookup_constructor(name).is_some();
        names_a_type && !names_a_term
    }
}

/// E0023 (ADR 6.10.26d D3): `misplaced` was written where `slot` belongs.
pub(super) fn term_type_confusion(misplaced: &TypeExpr, slot: ConfusedSlot) -> ElabError {
    ElabError::new(
        misplaced.span(),
        ElabErrorKind::Sort(SortErrorKind::TermTypeConfusion {
            found: describe_written(misplaced),
            slot,
        }),
    )
}

/// The written argument in words: its name when it has one, else its shape.
fn describe_written(written: &TypeExpr) -> String {
    match written {
        TypeExpr::Path(path) => format!("`{}`", path.item_name().name),
        TypeExpr::App(base, _, _) => describe_application(base),
        TypeExpr::Term(expr) => describe_term(expr),
        _ => "this type".to_string(),
    }
}

/// `Box<…>` for an application of a named type.
fn describe_application(base: &TypeExpr) -> String {
    match base {
        TypeExpr::Path(path) => format!("`{}<…>`", path.item_name().name),
        // Defensive: the grammar parses `<…>` only after a path.
        _ => "this type application".to_string(),
    }
}

/// A literal as written; any other expression by its shape.
fn describe_term(expr: &Expr) -> String {
    match expr {
        Expr::IntLiteral(value, _) => format!("`{value}`"),
        Expr::BoolLiteral(value, _) => format!("`{value}`"),
        Expr::StringLiteral(value, _) => format!("`{value:?}`"),
        _ => "this expression".to_string(),
    }
}

/// `body[params := args]`, all at once. One parameter at a time is wrong when
/// the arguments swap: `Le<m, n>` against `Le(n, m)` would turn `n := m` into
/// `m + k == m` and then `m := n` into `n + k == n`. So every parameter is
/// first renamed to a fresh name, and only then is each fresh name replaced.
pub(in crate::elaborate) fn expand_simultaneously(
    params: &[(String, Type)],
    body: &Type,
    args: &[Term],
) -> Type {
    let fresh: Vec<String> = (0..params.len())
        .map(|index| format!("{FRESH_PARAMETER_PREFIX}{index}"))
        .collect();
    let renamed = params
        .iter()
        .zip(&fresh)
        .fold(body.clone(), |ty, ((param, _), name)| {
            ty.substitute_term(param, &Term::var(name.clone()))
        });
    fresh
        .iter()
        .zip(args)
        .fold(renamed, |ty, (name, arg)| ty.substitute_term(name, arg))
}
