//! α-equivalent type equality directly on the node DAG (ADR 2.7.26a §4).
//!
//! Line-by-line port of `types::equality::types_equal_with_env` to
//! [`TypeNode`] handles — `tg_types_equal` is called from ~90 self-host elaboration
//! sites, so materializing both sides per call would replace the removed
//! retention cost with a transient-allocation storm. The
//! `handle_equality_matches_owned_oracle` test keeps this port pinned to the
//! owned implementation.

use std::collections::HashMap;

use super::nodes::TypeNode;
use crate::ffi::{Arena, TypeHandle};

/// α-equivalence on node handles; mirrors `types_equal_alpha`.
/// Invalid handles are unequal to everything (the FFI boundary's contract).
pub(crate) fn type_handles_equal(arena: &Arena, a: TypeHandle, b: TypeHandle) -> bool {
    // Dangling handles fall out of the walk's node lookups (never equal).
    equal_with_env(arena, a, b, &mut HashMap::new())
}

/// Strip the `@` prefix from named-type `TyVars` (ADR 13.4.26c §2).
fn strip_named(name: &str) -> &str {
    name.strip_prefix('@').unwrap_or(name)
}

/// TyVar-name vs μ-binder match, honouring the `α_` convention and env.
fn tyvar_matches_mu(v: &str, mu_var: &str, env: &HashMap<String, String>) -> bool {
    let v = strip_named(v);
    let target = mu_var.strip_prefix("α_").unwrap_or(mu_var);
    env.get(v)
        .map_or(v == target, |mapped| strip_named(mapped) == target)
}

#[allow(clippy::match_same_arms)] // Reason: a type-equality table whose arm order is the documentation; it mirrors the owned walk arm for arm
fn equal_with_env(
    arena: &Arena,
    a: TypeHandle,
    b: TypeHandle,
    env: &mut HashMap<String, String>,
) -> bool {
    // Same handle ⇒ same node ⇒ equal under any env-consistent reading is
    // NOT generally true (env maps a-side names to b-side names), so no
    // handle fast path — semantics first (mirrors the owned walk exactly).
    let (Some(na), Some(nb)) = (arena.get_type_node(a), arena.get_type_node(b)) else {
        return false;
    };
    match (na, nb) {
        (TypeNode::Bool, TypeNode::Bool)
        | (TypeNode::Nat, TypeNode::Nat)
        | (TypeNode::Int, TypeNode::Int)
        | (TypeNode::Unit, TypeNode::Unit)
        | (TypeNode::Void, TypeNode::Void)
        | (TypeNode::String, TypeNode::String) => true,

        (TypeNode::TyVar(v1), TypeNode::TyVar(v2)) => tyvars_equal(v1, v2, env),

        // 0-arity App is equivalent to TyVar
        (TypeNode::TyVar(v), TypeNode::App(name, args))
        | (TypeNode::App(name, args), TypeNode::TyVar(v)) => tyvar_app_equal(v, name, args, env),

        // TyVar vs Mu — normalization depth asymmetry
        (TypeNode::TyVar(v), TypeNode::Mu(mu_var, _))
        | (TypeNode::Mu(mu_var, _), TypeNode::TyVar(v)) => tyvar_matches_mu(v, mu_var, env),

        (TypeNode::Arrow(a1, a2), TypeNode::Arrow(b1, b2))
        | (TypeNode::Product(a1, a2), TypeNode::Product(b1, b2))
        | (TypeNode::Sum(a1, a2), TypeNode::Sum(b1, b2)) => {
            let (a1, a2, b1, b2) = (*a1, *a2, *b1, *b2);
            equal_with_env(arena, a1, b1, env) && equal_with_env(arena, a2, b2, env)
        }

        (TypeNode::Mu(v1, body1), TypeNode::Mu(v2, body2))
        | (TypeNode::Forall(v1, body1), TypeNode::Forall(v2, body2)) => {
            let (v1, v2, body1, body2) = (v1.clone(), v2.clone(), *body1, *body2);
            with_binding(env, &v1, &v2, |env| {
                equal_with_env(arena, body1, body2, env)
            })
        }

        // Eq types: same underlying type + structurally equal (owned) terms.
        (TypeNode::Eq(ty1, l1, r1), TypeNode::Eq(ty2, l2, r2)) => {
            let (ty1, ty2, l1, r1, l2, r2) = (*ty1, *ty2, *l1, *r1, *l2, *r2);
            equal_with_env(arena, ty1, ty2, env)
                && terms_materialize_equal(arena, l1, l2)
                && terms_materialize_equal(arena, r1, r2)
        }

        (TypeNode::Ptr(i1), TypeNode::Ptr(i2)) | (TypeNode::Ref(i1), TypeNode::Ref(i2)) => {
            let (i1, i2) = (*i1, *i2);
            equal_with_env(arena, i1, i2, env)
        }

        (TypeNode::App(name1, args1), TypeNode::App(name2, args2)) => {
            name1 == name2 && all_equal(arena, &args1.clone(), &args2.clone(), env)
        }

        (TypeNode::Adt(name1, args1, vars1), TypeNode::Adt(name2, args2, vars2)) => {
            if name1 != name2 || args1.len() != args2.len() || vars1.len() != vars2.len() {
                return false;
            }
            let (args1, args2) = (args1.clone(), args2.clone());
            let ctors_match = vars1
                .iter()
                .zip(vars2.iter())
                .all(|((c1, _), (c2, _))| c1 == c2);
            let payloads: Vec<(TypeHandle, TypeHandle)> = vars1
                .iter()
                .zip(vars2.iter())
                .map(|((_, p1), (_, p2))| (*p1, *p2))
                .collect();
            ctors_match
                && all_equal(arena, &args1, &args2, env)
                && payloads
                    .iter()
                    .all(|(p1, p2)| equal_with_env(arena, *p1, *p2, env))
        }

        // A `Pi` on either side: the core's own walk, seeded with this α map (ADR 2.10.26f D6)
        // A `Sigma` takes the same walk, which never equates it with a product (ADR 4.10.26h)
        // So do the predicate binders (ADR 6.10.26h D10)
        (
            TypeNode::Pi(..)
            | TypeNode::Sigma(..)
            | TypeNode::PredApp(..)
            | TypeNode::ForallPred(..)
            | TypeNode::PredAbs(..),
            _,
        )
        | (
            _,
            TypeNode::Pi(..)
            | TypeNode::Sigma(..)
            | TypeNode::PredApp(..)
            | TypeNode::ForallPred(..)
            | TypeNode::PredAbs(..),
        ) => super::pi::pi_types_equal(arena, a, b, env),

        // TypeError (poison) unifies with any type
        (TypeNode::Error, _) | (_, TypeNode::Error) => true,

        _ => false,
    }
}

/// Structural (non-alpha) term comparison via transient materialization —
/// Eq-type term components are a cold path (proof code only).
fn terms_materialize_equal(
    arena: &Arena,
    a: crate::ffi::TermHandle,
    b: crate::ffi::TermHandle,
) -> bool {
    use crate::ffi::terms::nodes::materialize_term;
    match (materialize_term(arena, a), materialize_term(arena, b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn tyvars_equal(v1: &str, v2: &str, env: &HashMap<String, String>) -> bool {
    let v1 = strip_named(v1);
    let v2 = strip_named(v2);
    env.get(v1)
        .map_or(v1 == v2, |mapped| strip_named(mapped) == v2)
}

fn tyvar_app_equal(
    v: &str,
    name: &str,
    args: &[TypeHandle],
    env: &HashMap<String, String>,
) -> bool {
    if !args.is_empty() {
        return false;
    }
    let v = strip_named(v);
    env.get(v)
        .map_or(v == name, |mapped| strip_named(mapped) == name)
}

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

fn all_equal(
    arena: &Arena,
    a: &[TypeHandle],
    b: &[TypeHandle],
    env: &mut HashMap<String, String>,
) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| equal_with_env(arena, *x, *y, env))
}

// Tests: node_tests/equality_tests.rs
#[cfg(test)]
#[path = "node_tests/equality_tests.rs"]
mod tests;
