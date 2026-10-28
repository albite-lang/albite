//! Alpha-equivalent type equality.
//!
//! Checks whether two types are equal up to renaming of bound variables.
//! Handles μ-types, ∀-types, TyVar/App normalization, and TyVar/Mu equivalence.

mod modulo_spans;
mod strict;

pub use strict::{
    eq_witnesses_equal_modulo_spans, types_equal_strict, types_equal_strict_with,
    EqWitnessComparator, EqWitnessPairs,
};

use super::predicate::pred_abs_bodies_unified;
use super::Type;
use crate::terms::substitution::capture::pi_codomains_unified;
use std::collections::HashMap;

/// Check if two types are α-equivalent (equal up to renaming of bound variables).
///
/// This handles:
/// - μα.F ≡ μβ.F[β/α] (μ-types with different bound variable names)
/// - ∀α.τ ≡ ∀β.τ[β/α] (forall types with different bound variable names)
#[must_use]
pub fn types_equal_alpha(a: &Type, b: &Type) -> bool {
    types_equal_with_env(a, b, &mut HashMap::new())
}

/// Strip the `@` prefix from named type `TyVars` (ADR 13.4.26c §2).
/// Named types use `@` prefix to distinguish from genuine type variables,
/// but comparison should treat `@X` and `X` as the same named type.
fn strip_named(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}

/// Check whether a `TyVar` name matches a Mu recursion variable.
///
/// Handles both the α_ naming convention (`α_X` → `X`) and the self-hosted
/// compiler convention where the Mu variable name matches directly.
fn tyvar_matches_mu(v: &str, mu_var: &str, env: &HashMap<String, String>) -> bool {
    let v = strip_named(v);
    let target = mu_var.strip_prefix("α_").unwrap_or(mu_var);
    env.get(v)
        .map_or(v == target, |mapped| strip_named(mapped) == target)
}

/// Check type equality with an environment mapping bound variables.
///
/// The `env` maps bound variables in `a` to their corresponding bound variables in `b`.
/// Only populated when descending into Mu or Forall binders—free variables
/// are compared by name equality, not tracked here. `pub(crate)` so the FFI
/// arena's `Pi` arms run this walk with their own α map (ADR 2.10.26f D6).
pub(crate) fn types_equal_with_env(a: &Type, b: &Type, env: &mut HashMap<String, String>) -> bool {
    match (a, b) {
        // Primitive types: a pair of them is equal exactly when identical
        // (ADR 18.9.26f — no per-primitive arm to forget)
        (a, b) if a.is_primitive() && b.is_primitive() => a == b,

        // Type variables
        (Type::TyVar(v1), Type::TyVar(v2)) => tyvars_equal(v1, v2, env),

        // 0-arity App is equivalent to TyVar
        (Type::TyVar(v), Type::App(name, args)) | (Type::App(name, args), Type::TyVar(v)) => {
            tyvar_app_equal(v, name, args, env)
        }

        // TyVar vs Mu — handles normalization depth asymmetry
        (Type::TyVar(v), Type::Mu(mu_var, _)) | (Type::Mu(mu_var, _), Type::TyVar(v)) => {
            tyvar_matches_mu(v, mu_var, env)
        }

        // Binary recursive type constructors
        (Type::Arrow(a1, a2), Type::Arrow(b1, b2))
        | (Type::Product(a1, a2), Type::Product(b1, b2))
        | (Type::Sum(a1, a2), Type::Sum(b1, b2)) => {
            types_equal_with_env(a1, b1, env) && types_equal_with_env(a2, b2, env)
        }

        // Binding forms (μ / ∀): bind variables and compare bodies
        (Type::Mu(v1, body1), Type::Mu(v2, body2))
        | (Type::Forall(v1, body1), Type::Forall(v2, body2)) => {
            with_binding(env, v1, v2, |env| types_equal_with_env(body1, body2, env))
        }

        // Dependent functions: binders compared up to renaming (ADR 30.9.26d D4)
        // A dependent pair the same way; never equal to a `Product` (ADR 4.10.26h D1)
        (Type::Pi(x, dom1, cod1), Type::Pi(y, dom2, cod2))
        | (Type::Sigma(x, dom1, cod1), Type::Sigma(y, dom2, cod2)) => {
            let (cod1, cod2) = pi_codomains_unified((x, cod1), (y, cod2));
            types_equal_with_env(dom1, dom2, env) && types_equal_with_env(&cod1, &cod2, env)
        }
        (Type::Pi(x, dom, cod), arrow @ Type::Arrow(..))
        | (arrow @ Type::Arrow(..), Type::Pi(x, dom, cod)) => {
            non_dependent_pi_as_arrow(x, dom, cod)
                .is_some_and(|as_arrow| types_equal_with_env(&as_arrow, arrow, env))
        }

        // Equality types (quasi-dependent)
        (Type::Eq(ty1, t1a, t1b), Type::Eq(ty2, t2a, t2b)) => {
            eq_types_equal((ty1, t1a, t1b), (ty2, t2a, t2b), env)
        }

        // Wrapper types
        (Type::Ptr(i1), Type::Ptr(i2)) | (Type::Ref(i1), Type::Ref(i2)) => {
            types_equal_with_env(i1, i2, env)
        }

        // Predicate binders (ADR 6.10.26h D6): the variable as a type
        // variable, the arguments as `Eq`'s are, the binders up to α
        (Type::PredApp(p1, args1), Type::PredApp(p2, args2)) => {
            tyvars_equal(p1, p2, env)
                && args1.len() == args2.len()
                && args1
                    .iter()
                    .zip(args2)
                    .all(|(a, b)| a == b || modulo_spans::terms_equal_modulo_spans(a, b, env))
        }
        (Type::ForallPred(p1, doms1, body1), Type::ForallPred(p2, doms2, body2)) => {
            all_types_equal(doms1, doms2, env)
                && with_binding(env, p1, p2, |env| types_equal_with_env(body1, body2, env))
        }
        (Type::PredAbs(params1, body1), Type::PredAbs(params2, body2)) => {
            params1.len() == params2.len()
                && params1
                    .iter()
                    .zip(params2)
                    .all(|((_, d1), (_, d2))| types_equal_with_env(d1, d2, env))
                && pred_abs_bodies_unified((params1, body1), (params2, body2))
                    .is_some_and(|(b1, b2)| types_equal_with_env(&b1, &b2, env))
        }

        // Type applications
        (Type::App(name1, args1), Type::App(name2, args2)) => {
            name1 == name2 && all_types_equal(args1, args2, env)
        }

        // ADT types
        (Type::Adt(name1, args1, vars1), Type::Adt(name2, args2, vars2)) => {
            adts_equal((name1, args1, vars1), (name2, args2, vars2), env)
        }

        // TypeError (poison) unifies with any type — suppress cascading errors
        (Type::Error, _) | (_, Type::Error) => true,

        _ => false,
    }
}

/// Check if two Eq types are equal: same underlying type + same term
/// components, modulo source spans (ADR 29.9.26d D6). The derived `==` is
/// tried first because this sits under the hot `types_equal_alpha`; only a
/// miss pays for the span-transparent walk.
fn eq_types_equal(
    eq1: (&Type, &crate::terms::Term, &crate::terms::Term),
    eq2: (&Type, &crate::terms::Term, &crate::terms::Term),
    env: &mut HashMap<String, String>,
) -> bool {
    let same_term = |a: &crate::terms::Term, b: &crate::terms::Term, env: &mut _| {
        a == b || modulo_spans::terms_equal_modulo_spans(a, b, env)
    };
    types_equal_with_env(eq1.0, eq2.0, env)
        && same_term(eq1.1, eq2.1, env)
        && same_term(eq1.2, eq2.2, env)
}

/// A `Pi` whose binder is not free in its codomain, as the `Arrow` it means;
/// `None` for a dependent one, which no `Arrow` describes (ADR 30.9.26d D4).
pub(super) fn non_dependent_pi_as_arrow(binder: &str, dom: &Type, cod: &Type) -> Option<Type> {
    (!cod.free_term_vars().contains(binder)).then(|| Type::arrow(dom.clone(), cod.clone()))
}

/// Check if two type variables are equal, respecting @-prefix stripping and env bindings.
fn tyvars_equal(v1: &str, v2: &str, env: &HashMap<String, String>) -> bool {
    let v1 = strip_named(v1);
    let v2 = strip_named(v2);
    env.get(v1)
        .map_or(v1 == v2, |mapped| strip_named(mapped) == v2)
}

/// Check if a `TyVar` equals a 0-arity App (which is semantically equivalent to a `TyVar`).
fn tyvar_app_equal(v: &str, name: &str, args: &[Type], env: &HashMap<String, String>) -> bool {
    if !args.is_empty() {
        return false;
    }
    let v = strip_named(v);
    env.get(v)
        .map_or(v == name, |mapped| strip_named(mapped) == name)
}

/// Check if two ADT types are structurally equal.
fn adts_equal(
    adt1: (&str, &[Type], &[(String, Type)]),
    adt2: (&str, &[Type], &[(String, Type)]),
    env: &mut HashMap<String, String>,
) -> bool {
    adt1.0 == adt2.0
        && adt1.1.len() == adt2.1.len()
        && adt1.2.len() == adt2.2.len()
        && all_types_equal(adt1.1, adt2.1, env)
        && adt1
            .2
            .iter()
            .zip(adt2.2.iter())
            .all(|((ctor1, pay1), (ctor2, pay2))| {
                ctor1 == ctor2 && types_equal_with_env(pay1, pay2, env)
            })
}

/// Temporarily bind `v1 → v2` in env, run `f`, then restore.
fn with_binding<R>(
    env: &mut HashMap<String, String>,
    v1: &str,
    v2: &str,
    f: impl FnOnce(&mut HashMap<String, String>) -> R,
) -> R {
    let old = env.insert(v1.to_owned(), v2.to_owned());
    let result = f(env);
    match old {
        Some(prev) => {
            env.insert(v1.to_owned(), prev);
        }
        None => {
            env.remove(v1);
        }
    }
    result
}

/// Check that two type-arg slices are pairwise equal.
fn all_types_equal(a: &[Type], b: &[Type], env: &mut HashMap<String, String>) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| types_equal_with_env(x, y, env))
}

#[cfg(test)]
mod equality_tests;
