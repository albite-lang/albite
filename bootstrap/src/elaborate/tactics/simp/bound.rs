//! What "bounded" means for `simp` (ADR 4.10.26e D3), as pure functions: the
//! round loop under its step bound, and the permutative-lemma test.

use std::collections::{BTreeMap, BTreeSet};

use albite_core::Term;

use crate::elaborate::tactics::matching::match_term;

/// How many rewrite steps one `simp` call may take (D3). A call that has
/// taken this many and would take one more is refused with E0086; a goal
/// stable after exactly this many is not. Changed only by amendment.
pub(crate) const SIMP_STEP_BOUND: usize = 64;

/// How many of the last rules applied E0086 names: enough to show a cycle.
pub(super) const LAST_RULES_SHOWN: usize = 4;

/// How a `simp` call's rounds ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RoundsOutcome {
    /// No rule occurs any more, after `steps` rewrites.
    Stable { steps: usize },
    /// The bound was reached with a rule still applying: the 1-based
    /// positions of the last rules applied, oldest first.
    BoundReached { last_rules: Vec<usize> },
}

/// Run `round` until it applies nothing (D2). Each call applies the first
/// rule in list order that occurs and returns its position, or `None` when
/// none does. A round that applies a rule after `bound` steps ends the call.
pub(super) fn run_rounds<E>(
    bound: usize,
    mut round: impl FnMut() -> Result<Option<usize>, E>,
) -> Result<RoundsOutcome, E> {
    let mut steps = 0;
    let mut last_rules = Vec::with_capacity(LAST_RULES_SHOWN);
    while let Some(position) = round()? {
        if steps == bound {
            return Ok(RoundsOutcome::BoundReached { last_rules });
        }
        steps += 1;
        if last_rules.len() == LAST_RULES_SHOWN {
            last_rules.remove(0);
        }
        last_rules.push(position);
    }
    Ok(RoundsOutcome::Stable { steps })
}

/// Whether the equation `left = right` over the lemma variables `vars` only
/// renames them (D3): `n + m = m + n` is, `0 + n = n` is not, and neither is
/// an equation whose sides are already the same term, which rewrites
/// nothing. A bare lemma like that would rewrite forever, so `simp` refuses
/// it; an applied instance has no variables left and is not permutative.
pub(super) fn is_permutative(vars: &[String], left: &Term, right: &Term) -> bool {
    if vars.is_empty() || left.strip_spans() == right.strip_spans() {
        return false;
    }
    let mut bindings = BTreeMap::new();
    if !match_term(left, vars, right, &mut bindings) {
        return false;
    }
    let mut targets = BTreeSet::new();
    bindings.values().all(|term| match term {
        Term::Var(name) => vars.contains(name) && targets.insert(name.clone()),
        _ => false,
    })
}
