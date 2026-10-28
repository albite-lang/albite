//! Elaboration of `region { e }` (ADR 9.10.26o D3, D4).
//!
//! A region elaborates to `Term::Region` around its body's term, typed as the
//! body. Three refusals run here, on semantic types, so `expect_error` sees
//! them: E0094 (the value, or an exit crossing the region, is not first-order
//! data), E0095's type-variable half (the body names an outer variable whose
//! type holds a type parameter of the enclosing definition), and E0096 (a
//! region in a proof). E0095's handle half reads declarations, not types, so
//! it runs at admission beside `B4.1` (`handles/regions.rs`).

use std::collections::{BTreeSet, HashSet};

use crate::ast::Expr;
use crate::span::Span;
use albite_core::terms::RegionTypes;
use albite_core::{Term, Type};

use crate::elaborate::error::{ElabError, ElabErrorKind, RegionErrorKind, TYPE_PARAMETER_REASON};
use crate::elaborate::exprs::find_crossing_exit;
use crate::elaborate::{ElabResult, Elaborator};

#[cfg(test)]
mod tests;

impl Elaborator<'_> {
    /// Elaborate `region { body }`, against `expected` when it is known.
    pub(crate) fn elab_region(
        &mut self,
        body: &Expr,
        expected: Option<&Type>,
        span: Span,
    ) -> ElabResult<(Term, Type)> {
        if self.in_proof_body {
            return Err(region_error(span, RegionErrorKind::RegionInProof));
        }
        let outer = self.outer_type_variable_bindings();
        let (term, ty) = self.in_region(|elab| match expected {
            Some(expected) => Ok::<_, ElabError>((elab.check(body, expected)?, expected.clone())),
            None => elab.infer(body),
        })?;
        if let Some(found) = first_order_violation(&ty) {
            return Err(not_first_order(span, &found));
        }
        // An exit crossing the region carries a value out of it too, so it is
        // held to the same rule, against the innermost return context — and
        // that type is what the copy-out of a crossing exit reads (ADR
        // 13.10.26q D2).
        let mut exit = None;
        if let Some(exit_span) = find_crossing_exit(body) {
            if let Some(found) = self
                .current_return_type
                .as_ref()
                .and_then(first_order_violation)
            {
                return Err(not_first_order(exit_span, &found));
            }
            exit.clone_from(&self.current_return_type);
        }
        let free = term.free_vars();
        if let Some(name) = first_reached(&outer, &free) {
            return Err(region_error(
                span,
                RegionErrorKind::ReachesOutside {
                    name,
                    reason: TYPE_PARAMETER_REASON.to_string(),
                },
            ));
        }
        let types = RegionTypes {
            value: ty.clone(),
            exit,
        };
        Ok((Term::region(types, term), ty))
    }

    /// The locals in scope whose type mentions an in-scope type parameter.
    fn outer_type_variable_bindings(&self) -> BTreeSet<String> {
        self.env
            .local_bindings()
            .filter(|binding| {
                binding
                    .ty
                    .free_type_vars()
                    .iter()
                    .any(|var| self.env.has_type_var(var))
            })
            .map(|binding| binding.name.clone())
            .collect()
    }
}

/// The first outer type-variable binding the body names, in name order so
/// the diagnostic is deterministic.
fn first_reached(outer: &BTreeSet<String>, free: &HashSet<String>) -> Option<String> {
    outer.iter().find(|name| free.contains(*name)).cloned()
}

fn region_error(span: Span, kind: RegionErrorKind) -> ElabError {
    ElabError::new(span, ElabErrorKind::Region(kind))
}

fn not_first_order(span: Span, found: &Type) -> ElabError {
    region_error(
        span,
        RegionErrorKind::NotFirstOrder {
            found: found.to_string(),
        },
    )
}

/// The part of `ty` that is not first-order data (G3), or `None`.
///
/// An allow-list: `Nat`, `Int`, `Bool`, `String`, `Unit`, and products, sums,
/// ADTs and recursive types built only from them. Every other kind — a
/// function, `Ref`, `Ptr`, a free type variable, `Forall`, a proof type,
/// `Void` — is refused, so a new kind is refused until it is named here.
/// `Type::Error` is poison in transit and raises nothing.
#[must_use]
pub fn first_order_violation(ty: &Type) -> Option<Type> {
    violation_within(ty, &mut Vec::new(), &mut BTreeSet::new())
}

fn violation_within(
    ty: &Type,
    bound: &mut Vec<String>,
    visited: &mut BTreeSet<String>,
) -> Option<Type> {
    match ty {
        Type::Nat | Type::Int | Type::Bool | Type::String | Type::Unit | Type::Error => None,
        Type::Product(left, right) | Type::Sum(left, right) => {
            violation_within(left, bound, visited)
                .or_else(|| violation_within(right, bound, visited))
        }
        Type::Mu(var, body) => {
            bound.push(var.clone());
            let found = violation_within(body, bound, visited);
            bound.pop();
            found
        }
        Type::TyVar(var) if bound.contains(var) => None,
        Type::Adt(name, args, variants) => {
            if !visited.insert(name.clone()) {
                return None;
            }
            args.iter()
                .chain(variants.iter().map(|(_, payload)| payload))
                .find_map(|part| violation_within(part, bound, visited))
        }
        // A named type applied before its body was resolved: its arguments
        // are what a caller can choose.
        Type::App(_, args) => args
            .iter()
            .find_map(|arg| violation_within(arg, bound, visited)),
        _ => Some(ty.clone()),
    }
}
