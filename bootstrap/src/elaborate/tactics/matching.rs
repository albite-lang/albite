//! `apply`'s first-order matching against a goal's conclusion (ADR 2.10.26c
//! D3), and `rewrite`'s against one subterm (`match_term`, ADR 2.10.26e D10),
//! over one shared binding step. Pure: no elaborator, no conversion, no δ.
//!
//! A pattern variable binds at its first occurrence. Every later occurrence,
//! and every rigid part, is then checked by instantiating the pattern and
//! comparing it with the target under the core's span-insensitive,
//! α-respecting [`types_equal_alpha`] — syntactic, so `0 + ?n` never matches
//! `k * 1` however the two compute.

use std::collections::BTreeMap;

use albite_core::{types_equal_alpha, Term, Type};

/// The prefix of a pattern variable's internal spelling while it is
/// instantiated: like a hole's, impossible as a source name, so substituting
/// one binding can never rewrite a name inside another's term (D2, P0.4).
pub(crate) const PATTERN_VAR_PREFIX: &str = "%var";

/// Match `pattern` — a lemma's conclusion over the pattern variables `vars` —
/// against `target`. `Some(bindings)` holds a term for each variable that
/// occurs in the pattern; one that does not occur is absent, and the caller
/// decides what that means. `None` is "does not match".
#[must_use]
pub(crate) fn match_conclusion(
    pattern: &Type,
    vars: &[String],
    target: &Type,
) -> Option<BTreeMap<String, Term>> {
    let mut bindings = BTreeMap::new();
    collect_type(pattern, target, vars, &mut bindings);
    types_equal_alpha(&instantiate(pattern, &bindings), target).then_some(bindings)
}

/// Match the term `pattern` over `vars` against `target`, threading
/// `bindings` so a variable already bound must meet the same term (ADR
/// 2.10.26e D10). True when the instantiated pattern equals `target` after
/// `strip_spans()`; on false, `bindings` may hold partial work the caller
/// discards. `rewrite`'s occurrence search calls this once per subterm.
pub(crate) fn match_term(
    pattern: &Term,
    vars: &[String],
    target: &Term,
    bindings: &mut BTreeMap<String, Term>,
) -> bool {
    collect_term(pattern, target, vars, bindings);
    instantiate_term(pattern, bindings).strip_spans() == target.strip_spans()
}

/// [`instantiate`] for a term: every bound variable replaced simultaneously.
#[must_use]
pub(crate) fn instantiate_term(term: &Term, bindings: &BTreeMap<String, Term>) -> Term {
    let mut renamed = term.clone();
    for (index, var) in bindings.keys().enumerate() {
        let internal = Term::var(format!("{PATTERN_VAR_PREFIX}{index}"));
        renamed = renamed.substitute_avoiding(var, &internal);
    }
    for (index, term) in bindings.values().enumerate() {
        renamed = renamed.substitute_avoiding(&format!("{PATTERN_VAR_PREFIX}{index}"), term);
    }
    renamed
}

/// `ty` with every bound variable replaced by its term, simultaneously: the
/// variables are first renamed apart into [`PATTERN_VAR_PREFIX`] spellings, so
/// a binding that mentions another variable's name is never rewritten.
#[must_use]
pub(crate) fn instantiate(ty: &Type, bindings: &BTreeMap<String, Term>) -> Type {
    let mut renamed = ty.clone();
    for (index, var) in bindings.keys().enumerate() {
        let internal = Term::var(format!("{PATTERN_VAR_PREFIX}{index}"));
        renamed = renamed.substitute_term(var, &internal);
    }
    for (index, term) in bindings.values().enumerate() {
        renamed = renamed.substitute_term(&format!("{PATTERN_VAR_PREFIX}{index}"), term);
    }
    renamed
}

/// Walk two types in lockstep, binding first occurrences inside `Eq`
/// witnesses — the only place a term variable can occur in a type — under
/// arrows, `Π`s, products, sums and `exists`.
fn collect_type(
    pattern: &Type,
    target: &Type,
    vars: &[String],
    bindings: &mut BTreeMap<String, Term>,
) {
    match (pattern, target) {
        (Type::Eq(p_ty, p_left, p_right), Type::Eq(t_ty, t_left, t_right)) => {
            collect_type(p_ty, t_ty, vars, bindings);
            collect_term(p_left, t_left, vars, bindings);
            collect_term(p_right, t_right, vars, bindings);
        }
        (Type::Arrow(p_dom, p_cod), Type::Arrow(t_dom, t_cod))
        | (Type::Pi(_, p_dom, p_cod), Type::Pi(_, t_dom, t_cod))
        | (Type::Product(p_dom, p_cod), Type::Product(t_dom, t_cod))
        | (Type::Sum(p_dom, p_cod), Type::Sum(t_dom, t_cod)) => {
            collect_type(p_dom, t_dom, vars, bindings);
            collect_type(p_cod, t_cod, vars, bindings);
        }
        // An `exists` (ADR 6.10.26e D3): the binder's type, then the body,
        // where a pattern variable spelled like the binder is shadowed and
        // binds nothing. The final α-check still decides, so a binding that
        // captured the target's own binder is refused there, never passed.
        (Type::Sigma(p_bound, p_dom, p_body), Type::Sigma(_, t_dom, t_body)) => {
            collect_type(p_dom, t_dom, vars, bindings);
            let unshadowed: Vec<String> =
                vars.iter().filter(|var| *var != p_bound).cloned().collect();
            collect_type(p_body, t_body, &unshadowed, bindings);
        }
        _ => {}
    }
}

fn collect_term(
    pattern: &Term,
    target: &Term,
    vars: &[String],
    bindings: &mut BTreeMap<String, Term>,
) {
    let (pattern, target) = (peel_spans(pattern), peel_spans(target));
    if let Term::Var(name) = pattern {
        if vars.contains(name) {
            bindings
                .entry(name.clone())
                .or_insert_with(|| target.strip_spans());
            return;
        }
    }
    if std::mem::discriminant(pattern) != std::mem::discriminant(target) {
        return;
    }
    let mut pattern_children = Vec::new();
    pattern.for_each_subterm(|child| pattern_children.push(child.clone()));
    let mut target_children = Vec::new();
    target.for_each_subterm(|child| target_children.push(child.clone()));
    for (p, t) in pattern_children.iter().zip(&target_children) {
        collect_term(p, t, vars, bindings);
    }
}

fn peel_spans(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) => peel_spans(inner),
        _ => term,
    }
}
