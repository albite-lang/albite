//! `rewrite`'s occurrence search and motive (ADR 2.10.26e D3, D4). Pure:
//! functions over `Type` and `Term`, no elaborator.
//!
//! The search visits a goal `Eq σ l r`'s endpoints, `l` then `r`, each in
//! pre-order — a node before its children, children left to right — and never
//! descends under a binder, where abstracting a subterm could capture.
//! Equality is syntactic: equal after `strip_spans()`.

use std::collections::BTreeMap;

use albite_core::{Term, Type};

use crate::elaborate::exprs::combinators::{synthesised_motive, SYNTHESISED_MOTIVE_BINDER};
use crate::elaborate::tactics::matching::match_term;

/// A goal's two `Eq` endpoints and their base type, or `None` when the goal
/// is not an equation.
#[must_use]
pub(crate) fn goal_endpoints(goal: &Type) -> Option<(&Type, &Term, &Term)> {
    match goal {
        Type::Eq(base, left, right) => Some((base, left, right)),
        _ => None,
    }
}

/// Whether `term` binds a variable over one of its children: the walk stops
/// at such a node (D3), whose subterms may mention what it binds.
pub(crate) fn binds(term: &Term) -> bool {
    matches!(
        term,
        Term::Lambda(..)
            | Term::Let(..)
            | Term::Fix(..)
            | Term::Case(..)
            | Term::AdtMatch(..)
            | Term::NatInd(..)
            | Term::NatRec(..)
            | Term::TyAbs(..)
            | Term::Subst(..)
    )
}

/// Visit `term` and its subterms in pre-order, outside binders, until `visit`
/// returns `true`. Spans are transparent: a `Spanned` node is not itself a
/// candidate, its inner term is.
fn walk_outside_binders(term: &Term, visit: &mut impl FnMut(&Term) -> bool) -> bool {
    if let Term::Spanned(inner, _) = term {
        return walk_outside_binders(inner, visit);
    }
    if visit(term) {
        return true;
    }
    if binds(term) {
        return false;
    }
    let mut found = false;
    term.for_each_subterm(|child| {
        found = found || walk_outside_binders(child, visit);
    });
    found
}

/// The bindings of the first subterm, in D3's traversal order, that `pattern`
/// matches over `vars`. With no `vars` this is the first syntactic occurrence.
#[must_use]
pub(crate) fn first_match(
    pattern: &Term,
    vars: &[String],
    left: &Term,
    right: &Term,
) -> Option<BTreeMap<String, Term>> {
    let mut found = None;
    let mut visit = |candidate: &Term| {
        let mut bindings = BTreeMap::new();
        if match_term(pattern, vars, candidate, &mut bindings) {
            found = Some(bindings);
            return true;
        }
        false
    };
    if !walk_outside_binders(left, &mut visit) {
        walk_outside_binders(right, &mut visit);
    }
    found
}

/// Whether `pattern` occurs syntactically in `term`, outside binders.
#[must_use]
pub(crate) fn occurs(pattern: &Term, term: &Term) -> bool {
    let pattern = pattern.strip_spans();
    walk_outside_binders(term, &mut |candidate| candidate.strip_spans() == pattern)
}

/// `term` with every occurrence of `pattern` outside binders replaced by
/// `replacement`. Spans around a replaced occurrence go with it.
#[must_use]
pub(crate) fn abstract_occurrences(term: &Term, pattern: &Term, replacement: &Term) -> Term {
    let stripped = pattern.strip_spans();
    replace_outside_binders(term, &stripped, replacement)
}

fn replace_outside_binders(term: &Term, pattern: &Term, replacement: &Term) -> Term {
    if term.strip_spans() == *pattern {
        return replacement.clone();
    }
    if binds(term) {
        return term.clone();
    }
    let mut rebuilt = term.clone();
    for child in rebuilt.subterms_mut() {
        *child = replace_outside_binders(child, pattern, replacement);
    }
    rebuilt
}

/// D4's motive and new goal for rewriting `from` into `to` in the goal
/// `Eq σ' l r` (its `endpoints`), whose occurrences of `from` are abstracted
/// over `σ`: `P = Π #x:σ. Eq σ' l[from ↦ #x] r[from ↦ #x]` and
/// `G' = P[#x := to]`, computed with the kernel's own `substitute_term`.
#[must_use]
pub(crate) fn motive_and_new_goal(
    (goal_base, left, right): (&Type, &Term, &Term),
    base: &Type,
    from: &Term,
    to: &Term,
) -> (Type, Type) {
    let bound = Term::var(SYNTHESISED_MOTIVE_BINDER);
    let family = Type::eq(
        goal_base.clone(),
        abstract_occurrences(left, from, &bound),
        abstract_occurrences(right, from, &bound),
    );
    let motive = synthesised_motive(base, |_| family.clone());
    let new_goal = family.substitute_term(SYNTHESISED_MOTIVE_BINDER, to);
    (motive, new_goal)
}
