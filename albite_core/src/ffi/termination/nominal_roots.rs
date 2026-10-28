//! Give a structurally encoded parameter back the name it was declared with
//! (ADR 3.9.26f).
//!
//! Descent's Phase 1 picks a decreasing root by looking for a **named inductive
//! type or a μ-encoding** (`is_supported_inductive`), and it reads that type
//! off the definition's own `Lambda` binders. The two compilers encode the same
//! declaration differently:
//!
//! - the bootstrap μ-wraps every member of a recursive type **cluster**, and
//!   encodes a non-recursive type of three or more constructors as a flat
//!   `Adt` — so `e: MaybeExpr` arrives as `μα_MaybeExpr. …`;
//! - the self-host μ-wraps a type only when a field names it **directly**, so
//!   a member reached through another type (`MaybeExpr` → `Expr` →
//!   `MaybeExpr`) or through a generic argument (`List<UseTree>`) arrives as
//!   the bare `(Unit + Expr)` — a shape Phase 1 refuses.
//!
//! The engine is right and does not change (D1). What changes is what the
//! self-host hands it: each parameter's **declared type name** travels beside
//! the structure, and for a name the bootstrap would have encoded nominally the
//! binder is replaced, for the analysis only, by the unbound marker
//! `TyVar(name)` — the same spelling a non-entry cluster member already reaches
//! Phase 1 in (ADR 11.8.26b).
//!
//! ## Why the type graph is the DECLARED one
//!
//! Which names qualify is an SCC question over the program's type references,
//! and it is asked of the names the **source** spells, noted per type by
//! [`super::tg_termination_note_type`]. The obvious alternative — the positivity
//! mirror's registered graph — was measured and refused: it records
//! *elaborated* field types, and once `MaybeExpr` has been encoded as
//! `(Unit + @Expr)` the field of `Expr` that holds one carries no `MaybeExpr`
//! name at all. The `Expr → MaybeExpr` edge then depends on declaration order,
//! and three of the ten definitions this ADR targets stayed refused.

use std::collections::{BTreeMap, BTreeSet};

use crate::terms::termination::{is_recursive, tarjan_scc};
use crate::terms::Term;
use crate::types::Type;

/// The fewest constructors a non-recursive type needs before the bootstrap
/// encodes it as a flat `Adt` rather than a nested sum. Measured, not read:
/// a two-constructor type arrives as `(Unit + Nat)` and a three-constructor one
/// as `Three[A | B | C]` (ADR 3.9.26f §6).
const FLAT_ADT_MIN_CONSTRUCTORS: usize = 3;

/// What a type declaration is, as far as its encoding is concerned. The wire
/// codes are the positivity seam's (`POSITIVITY_KIND_*`), so the self-host
/// spells a kind one way for both mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclaredKind {
    Sum,
    Record,
    Alias,
}

impl DeclaredKind {
    /// Decode a wire code; `None` for anything outside the three.
    pub(crate) fn from_code(code: u64) -> Option<Self> {
        match code {
            0 => Some(DeclaredKind::Sum),
            1 => Some(DeclaredKind::Record),
            2 => Some(DeclaredKind::Alias),
            _ => None,
        }
    }
}

/// One declared type: its kind, its constructor count and every type name its
/// body spells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredType {
    pub kind: DeclaredKind,
    pub constructors: usize,
    pub references: BTreeSet<String>,
}

/// The type names the bootstrap hands Phase 1 nominally: every sum or record
/// on a recursive cycle of the declared graph (a μ-encoding) and every sum
/// with at least [`FLAT_ADT_MIN_CONSTRUCTORS`] constructors (a flat `Adt`).
///
/// An alias is a node, so a cycle through one is still a cycle, but never a
/// member of the result: nothing is μ-bound under an alias's name.
pub(crate) fn nominally_encoded_names(types: &BTreeMap<String, DeclaredType>) -> BTreeSet<String> {
    let adjacency: BTreeMap<String, BTreeSet<String>> = types
        .iter()
        .map(|(name, declared)| {
            let known = declared
                .references
                .iter()
                .filter(|reference| types.contains_key(*reference))
                .cloned()
                .collect();
            (name.clone(), known)
        })
        .collect();
    let mut names: BTreeSet<String> = tarjan_scc(&adjacency)
        .into_iter()
        .filter(|component| is_recursive(component, &adjacency))
        .flatten()
        .collect();
    names.extend(
        types
            .iter()
            .filter(|(_, declared)| {
                declared.kind == DeclaredKind::Sum
                    && declared.constructors >= FLAT_ADT_MIN_CONSTRUCTORS
            })
            .map(|(name, _)| name.clone()),
    );
    names.retain(|name| types[name].kind != DeclaredKind::Alias);
    names
}

/// Replace the binder type of every noted parameter whose declared type the
/// bootstrap would have encoded nominally.
///
/// `declared` maps a value-parameter position (counting `Lambda`s only, the
/// way `signature` does) to the type name its annotation spelled.
///
/// Only a **structural** binder (`Sum` or `Product`) is replaced. A μ-encoding
/// is already a root, and anything else — a primitive, an arrow, a type
/// parameter that happens to share a declared type's name — is a shape whose
/// refusal the bootstrap shares, so rewriting it would make the mirror agree by
/// accepting more rather than by seeing the same thing.
///
/// Returns how many binders were replaced.
pub(crate) fn nominalize_parameters(
    term: &mut Term,
    declared: &BTreeMap<usize, String>,
    inductive: &BTreeSet<String>,
) -> usize {
    let mut position = 0;
    let mut replaced = 0;
    let mut cursor = term;
    loop {
        match cursor {
            Term::Spanned(inner, _) | Term::Annot(inner, _) | Term::TyAbs(_, inner) => {
                cursor = inner;
            }
            Term::Lambda(_, binder, inner) => {
                if let Some(name) = declared.get(&position) {
                    if inductive.contains(name) && is_structural(binder) {
                        *binder = Type::TyVar(name.clone());
                        replaced += 1;
                    }
                }
                position += 1;
                cursor = inner;
            }
            _ => return replaced,
        }
    }
}

/// Whether `ty` is the structural encoding of a declared sum or record.
fn is_structural(ty: &Type) -> bool {
    matches!(ty, Type::Sum(..) | Type::Product(..))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum_of_unit_and(name: &str) -> Type {
        Type::Sum(
            Box::new(Type::Unit),
            Box::new(Type::TyVar(name.to_string())),
        )
    }

    fn lambda(binder: &str, ty: Type, body: Term) -> Term {
        Term::Lambda(binder.to_string(), ty, Box::new(body))
    }

    fn binder_types(term: &Term) -> Vec<Type> {
        let mut out = Vec::new();
        let mut cursor = term;
        loop {
            match cursor {
                Term::Spanned(inner, _) | Term::Annot(inner, _) | Term::TyAbs(_, inner) => {
                    cursor = inner;
                }
                Term::Lambda(_, ty, inner) => {
                    out.push(ty.clone());
                    cursor = inner;
                }
                _ => return out,
            }
        }
    }

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|name| (*name).to_string()).collect()
    }

    fn declared(list: &[(usize, &str)]) -> BTreeMap<usize, String> {
        list.iter()
            .map(|(position, name)| (*position, (*name).to_string()))
            .collect()
    }

    // 3.9.26f AC2: a structural binder declared as a cluster member becomes a
    // nominal marker, which is what Phase 1 accepts as a root.
    #[test]
    fn a_structural_binder_named_as_a_cluster_member_becomes_a_marker() {
        let mut term = lambda("e", sum_of_unit_and("Expr"), Term::Var("e".into()));
        let replaced = nominalize_parameters(
            &mut term,
            &declared(&[(0, "MaybeExpr")]),
            &names(&["MaybeExpr", "Expr"]),
        );
        assert_eq!(replaced, 1);
        assert_eq!(binder_types(&term), vec![Type::TyVar("MaybeExpr".into())]);
    }

    #[test]
    fn a_name_the_bootstrap_encodes_structurally_is_left_alone() {
        let mut term = lambda("t", sum_of_unit_and("Nat"), Term::Var("t".into()));
        let replaced =
            nominalize_parameters(&mut term, &declared(&[(0, "Two")]), &names(&["Expr"]));
        assert_eq!(replaced, 0);
        assert_eq!(binder_types(&term), vec![sum_of_unit_and("Nat")]);
    }

    #[test]
    fn a_non_structural_binder_is_never_rewritten_even_when_the_name_qualifies() {
        let mu = Type::Mu("Expr".into(), Box::new(sum_of_unit_and("Expr")));
        let mut term = lambda(
            "a",
            mu.clone(),
            lambda("b", Type::Nat, Term::Var("a".into())),
        );
        let replaced = nominalize_parameters(
            &mut term,
            &declared(&[(0, "Expr"), (1, "Expr")]),
            &names(&["Expr"]),
        );
        assert_eq!(replaced, 0);
        assert_eq!(binder_types(&term), vec![mu, Type::Nat]);
    }

    #[test]
    fn positions_count_value_parameters_through_type_abstractions_and_wrappers() {
        let product = Type::Product(Box::new(Type::Nat), Box::new(Type::Nat));
        let inner = lambda(
            "first",
            Type::Nat,
            lambda("second", product.clone(), Term::Var("second".into())),
        );
        let mut term = Term::TyAbs(
            "T".into(),
            Box::new(Term::Annot(Box::new(inner), Type::Nat)),
        );
        let replaced =
            nominalize_parameters(&mut term, &declared(&[(1, "Stmt")]), &names(&["Stmt"]));
        assert_eq!(replaced, 1);
        assert_eq!(
            binder_types(&term),
            vec![Type::Nat, Type::TyVar("Stmt".into())]
        );
    }
}
