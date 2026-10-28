//! Strict α-equivalence for the proof judgment (ADR 29.9.26e D11).
//!
//! [`super::types_equal_alpha`] carries three shortcuts that exist to bridge
//! normalisation depths in elaborator output: a type variable equals any `μ`
//! whose binder shares its name (after stripping `α_` and `@`), a type
//! variable equals a nullary `App` of the same name, and poison equals
//! everything. Each one equates two *different* types, and the first two let
//! a closed term reach `Void` with no negative `μ` at all. This sibling keeps
//! none of them: same constructors, binders renamed consistently, and names
//! compared byte for byte.

use std::collections::HashMap;

use super::modulo_spans::terms_equal_modulo_spans;
use super::{non_dependent_pi_as_arrow, Type};
use crate::terms::substitution::capture::pi_codomains_unified;
use crate::terms::Term;
use crate::types::predicate::pred_abs_bodies_unified;

/// Two `Eq`s' witness pairs, `((l₁, r₁), (l₂, r₂))`.
pub type EqWitnessPairs<'a> = ((&'a Term, &'a Term), (&'a Term, &'a Term));

/// How an `Eq`'s witness pairs are compared, given the type-binder pairs in
/// scope (each enclosing binder of the left type with its partner in the
/// right, innermost last).
pub type EqWitnessComparator<'c> = dyn Fn(EqWitnessPairs<'_>, &[(String, String)]) -> bool + 'c;

/// Whether `a` and `b` are α-equivalent with no shortcut.
///
/// Free type variables are equal only when spelled identically, so `@List`
/// and `List` differ here although they print the same.
#[must_use]
pub fn types_equal_strict(a: &Type, b: &Type) -> bool {
    types_equal_strict_with(a, b, &eq_witnesses_equal_modulo_spans)
}

/// The strict walk, with `eq_terms` deciding each `Eq`'s witness pairs (ADR
/// 1.10.26c D2). Nothing else in the walk depends on it, so a comparator can
/// loosen the `Eq` arm without reopening any of the shortcuts above.
#[must_use]
pub fn types_equal_strict_with(a: &Type, b: &Type, eq_terms: &EqWitnessComparator<'_>) -> bool {
    StrictWalk { eq_terms }.equal(a, b, &mut Vec::new())
}

/// The strict walk's own `Eq` comparison: both witnesses equal modulo source
/// spans (ADR 29.9.26d D6), under the binders in scope.
#[must_use]
pub fn eq_witnesses_equal_modulo_spans(
    ((l1, r1), (l2, r2)): EqWitnessPairs<'_>,
    binders: &[(String, String)],
) -> bool {
    let mut env: HashMap<String, String> = binders.iter().cloned().collect();
    terms_equal_modulo_spans(l1, l2, &mut env) && terms_equal_modulo_spans(r1, r2, &mut env)
}

struct StrictWalk<'w, 'c> {
    eq_terms: &'w EqWitnessComparator<'c>,
}

impl StrictWalk<'_, '_> {
    /// `binders` pairs each enclosing binder of `a` with its partner in `b`,
    /// innermost last.
    fn equal(&self, a: &Type, b: &Type, binders: &mut Vec<(String, String)>) -> bool {
        match (a, b) {
            // A primitive equals only itself; a non-primitive `b` never matches.
            (a, _) if a.is_primitive() => a == b,
            (Type::TyVar(v1), Type::TyVar(v2)) => tyvars_strict(v1, v2, binders),
            (Type::Arrow(a1, a2), Type::Arrow(b1, b2))
            | (Type::Product(a1, a2), Type::Product(b1, b2))
            | (Type::Sum(a1, a2), Type::Sum(b1, b2)) => {
                self.equal(a1, b1, binders) && self.equal(a2, b2, binders)
            }
            (Type::Mu(v1, body1), Type::Mu(v2, body2))
            | (Type::Forall(v1, body1), Type::Forall(v2, body2)) => {
                binders.push((v1.clone(), v2.clone()));
                let equal = self.equal(body1, body2, binders);
                binders.pop();
                equal
            }
            // Dependent functions (ADR 30.9.26d D4): the term binders are renamed
            // to one fresh name, since `binders` pairs type variables only.
            // A dependent pair likewise, never against a `Product` (ADR 4.10.26h D1).
            (Type::Pi(x, dom1, cod1), Type::Pi(y, dom2, cod2))
            | (Type::Sigma(x, dom1, cod1), Type::Sigma(y, dom2, cod2)) => {
                let (cod1, cod2) = pi_codomains_unified((x, cod1), (y, cod2));
                self.equal(dom1, dom2, binders) && self.equal(&cod1, &cod2, binders)
            }
            (Type::Pi(x, dom, cod), arrow @ Type::Arrow(..))
            | (arrow @ Type::Arrow(..), Type::Pi(x, dom, cod)) => {
                non_dependent_pi_as_arrow(x, dom, cod)
                    .is_some_and(|as_arrow| self.equal(&as_arrow, arrow, binders))
            }
            (Type::Eq(ty1, l1, r1), Type::Eq(ty2, l2, r2)) => {
                self.equal(ty1, ty2, binders) && (self.eq_terms)(((l1, r1), (l2, r2)), binders)
            }
            (Type::Ptr(i1), Type::Ptr(i2)) | (Type::Ref(i1), Type::Ref(i2)) => {
                self.equal(i1, i2, binders)
            }
            (Type::App(name1, args1), Type::App(name2, args2)) => {
                name1 == name2 && self.all_strict(args1, args2, binders)
            }
            (Type::Adt(name1, args1, variants1), Type::Adt(name2, args2, variants2)) => {
                name1 == name2
                    && variants1.len() == variants2.len()
                    && self.all_strict(args1, args2, binders)
                    && variants1
                        .iter()
                        .zip(variants2)
                        .all(|((c1, p1), (c2, p2))| c1 == c2 && self.equal(p1, p2, binders))
            }
            (Type::PredApp(p1, args1), Type::PredApp(p2, args2)) => {
                // Each argument pair through the `Eq` comparator, as `(a, a)`
                // against `(b, b)`, so proof mode converts it (ADR 6.10.26h D6)
                tyvars_strict(p1, p2, binders)
                    && args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2)
                        .all(|(a, b)| (self.eq_terms)(((a, a), (b, b)), binders))
            }
            (Type::ForallPred(p1, doms1, body1), Type::ForallPred(p2, doms2, body2)) => {
                if !self.all_strict(doms1, doms2, binders) {
                    return false;
                }
                binders.push((p1.clone(), p2.clone()));
                let equal = self.equal(body1, body2, binders);
                binders.pop();
                equal
            }
            (Type::PredAbs(params1, body1), Type::PredAbs(params2, body2)) => {
                params1.len() == params2.len()
                    && params1
                        .iter()
                        .zip(params2)
                        .all(|((_, d1), (_, d2))| self.equal(d1, d2, binders))
                    && pred_abs_bodies_unified((params1, body1), (params2, body2))
                        .is_some_and(|(b1, b2)| self.equal(&b1, &b2, binders))
            }
            (Type::Error, Type::Error) => true,
            _ => false,
        }
    }

    fn all_strict(&self, a: &[Type], b: &[Type], binders: &mut Vec<(String, String)>) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.equal(x, y, binders))
    }
}

/// A variable bound on either side must be bound by the *same* pair of
/// binders on both; two free variables are equal only when identical.
fn tyvars_strict(v1: &str, v2: &str, binders: &[(String, String)]) -> bool {
    for (left, right) in binders.iter().rev() {
        let (bound_left, bound_right) = (left == v1, right == v2);
        if bound_left || bound_right {
            return bound_left && bound_right;
        }
    }
    v1 == v2
}
