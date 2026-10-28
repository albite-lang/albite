//! The δ environment and the discharge (ADR 28.9.26d §2.3, D4–D9; ADR
//! 29.9.26c D1, D2, D8).
//!
//! Pure over injected data: the candidate bodies, the admission map and the
//! obligations come in, the environment and the failures go out.
//!
//! Moved here from the bootstrap by ADR 7.10.26u, so both compilers discharge
//! with one engine, as 30.9.26g did for the δ-free half. An obligation is its
//! two sides only; a failure is its index in the caller's slice and its note,
//! and each compiler renders its own error.

mod deferral;
mod numerals;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::conversion::{
    convertible, rename_apart, unfoldable_by_name, unfoldable_globals, Convertible, Renaming,
};
use crate::eval::{eval_with_env_and_limit, EvalEnv, EvalStopped};
use crate::terms::termination::AdmissionState;
use crate::Term;

pub use deferral::{deferral, is_admitted_total, mentions_global, performs_effect, Verdict};
pub use numerals::canonicalize_numerals;

/// One deferred conversion: the two sides as written.
pub type Sides<'a> = (&'a Term, &'a Term);

/// One obligation the discharge could not settle: its index in the caller's
/// slice, and the note (D9) the caller attaches to its own error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DischargeFailure {
    pub index: usize,
    pub note: Option<String>,
}

/// What δ may unfold, and what it had to leave out.
#[derive(Debug, Default)]
pub struct DeltaEnv {
    /// Every global the discharge may unfold. Each value here is a value (or a
    /// stuck residual): no lookup can start unbudgeted work (D6).
    pub globals: HashMap<String, Term>,
    /// Nullary globals dropped because their value did not settle, with why —
    /// the source of D9's "was not unfolded" note.
    pub dropped: BTreeMap<String, EvalStopped>,
}

/// Every global name `term` mentions.
#[must_use]
pub fn global_names(term: &Term) -> BTreeSet<String> {
    fn walk(term: &Term, found: &mut BTreeSet<String>) {
        if let Term::Global(name) = term {
            found.insert(name.clone());
        }
        term.for_each_subterm(|child| walk(child, found));
    }
    let mut found = BTreeSet::new();
    walk(term, &mut found);
    found
}

/// The D4-eligible globals in the transitive mention closure of the
/// obligations: admitted `Total`, effect-free. An ineligible name is not
/// expanded — it is absent from the env, so a reference to it sticks.
fn eligible_closure(
    candidates: &HashMap<String, Term>,
    admission: &BTreeMap<String, AdmissionState>,
    obligations: &[Sides<'_>],
) -> BTreeMap<String, Term> {
    let mut closure = BTreeMap::new();
    let mut queue: Vec<String> = obligations
        .iter()
        .flat_map(|(left, right)| global_names(left).into_iter().chain(global_names(right)))
        .collect();
    while let Some(name) = queue.pop() {
        if closure.contains_key(&name) || !is_admitted_total(admission, &name) {
            continue;
        }
        let Some(body) = candidates.get(&name).filter(|body| !performs_effect(body)) else {
            continue;
        };
        queue.extend(global_names(body));
        closure.insert(name, body.clone());
    }
    closure
}

/// The names reachable from `start`'s body through any chain of closure
/// globals, functions included (D6's nullary→nullary relation is this,
/// restricted to the pending nullaries).
fn reachable_from(start: &str, closure: &BTreeMap<String, Term>) -> BTreeSet<String> {
    let mut reached = BTreeSet::new();
    let mut queue: Vec<String> = closure
        .get(start)
        .map(|body| global_names(body).into_iter().collect())
        .unwrap_or_default();
    while let Some(name) = queue.pop() {
        if let Some(body) = closure.get(&name) {
            if reached.insert(name) {
                queue.extend(global_names(body));
            }
        }
    }
    reached
}

/// Order `pending` dependencies-first (D6), skipping the `cyclic` ones.
#[must_use]
pub fn settle_order(
    pending: &BTreeMap<String, BTreeSet<String>>,
    cyclic: &BTreeSet<String>,
) -> Vec<String> {
    fn visit(
        name: &str,
        pending: &BTreeMap<String, BTreeSet<String>>,
        cyclic: &BTreeSet<String>,
        order: &mut Vec<String>,
    ) {
        if cyclic.contains(name) || order.iter().any(|n| n == name) {
            return;
        }
        for dep in &pending[name] {
            visit(dep, pending, cyclic, order);
        }
        order.push(name.to_string());
    }
    let mut order = Vec::new();
    for name in pending.keys() {
        visit(name, pending, cyclic, &mut order);
    }
    order
}

/// Build the discharge environment (§2.3, D4, D6).
///
/// Non-nullary globals (a `Lambda` body — already a value) and nullary globals
/// whose body is already a value go in unchanged. Every other nullary global is
/// pre-evaluated once under `budget`, dependencies first, in an env holding the
/// values plus the nullaries already settled; on any stop it is dropped. So
/// `EvalEnv::lookup`, whose forcing loop is unbounded, never meets an
/// unevaluated body during the discharge.
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: both callers hold std `HashMap`s, as `unfoldable_globals`'s do
pub fn delta_globals(
    candidates: &HashMap<String, Term>,
    admission: &BTreeMap<String, AdmissionState>,
    obligations: &[Sides<'_>],
    budget: usize,
) -> DeltaEnv {
    let closure = eligible_closure(candidates, admission, obligations);
    let mut env = DeltaEnv::default();
    let mut pending: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, body) in &closure {
        if body.is_value() {
            env.globals.insert(name.clone(), body.clone());
        } else {
            pending.insert(name.clone(), BTreeSet::new());
        }
    }
    let reach: BTreeMap<String, BTreeSet<String>> = pending
        .keys()
        .map(|name| (name.clone(), reachable_from(name, &closure)))
        .collect();
    let mut cyclic = BTreeSet::new();
    for (name, deps) in &mut pending {
        *deps = reach[name]
            .iter()
            .filter(|dep| reach.contains_key(*dep) && *dep != name)
            .cloned()
            .collect();
        if reach[name].contains(name) {
            cyclic.insert(name.clone());
        }
    }
    for name in &cyclic {
        let mut cycle: Vec<String> = cyclic
            .iter()
            .filter(|other| reach[name].contains(*other) && reach[*other].contains(name))
            .cloned()
            .collect();
        cycle.push(cycle[0].clone());
        env.dropped
            .insert(name.clone(), EvalStopped::BlackHole { cycle });
    }
    for name in settle_order(&pending, &cyclic) {
        let scope = EvalEnv::new(env.globals.clone());
        match eval_with_env_and_limit(&closure[&name], &scope, budget) {
            Ok(value) => {
                env.globals.insert(name, value);
            }
            Err(stop) => {
                env.dropped.insert(name, stop);
            }
        }
    }
    env
}

/// Evaluate one side in its own fresh env: a recorded stop poisons an env for
/// every later evaluation (`finished_or_black_hole`), so no side may inherit
/// another's.
fn reduce(side: &Term, env: &DeltaEnv, budget: usize) -> Result<Term, EvalStopped> {
    eval_with_env_and_limit(side, &EvalEnv::new(env.globals.clone()), budget)
        .map(|value| canonicalize_numerals(&value))
}

/// D7's note for a side that stopped.
fn stop_note(stop: &EvalStopped) -> String {
    match stop {
        EvalStopped::StepLimit { limit } => {
            format!("conversion stopped after {limit} steps without reaching a normal form")
        }
        other => other.to_string(),
    }
}

/// D7's note for a nullary global D6 dropped.
fn dropped_note(name: &str, stop: &EvalStopped) -> String {
    match stop {
        EvalStopped::StepLimit { limit } => {
            format!("`{name}` was not unfolded: its value did not settle within {limit} steps")
        }
        other => format!("`{name}` was not unfolded: {other}"),
    }
}

/// The one note a failed obligation carries (D9 of 28.9.26d, D8 of
/// 29.9.26c), first match wins: a CBV or WHNF stop, then a dropped nullary
/// either side reached, then — for closed results — a side δ changed, and for
/// open ones the first pair the WHNF comparison found different.
fn failure_note(
    sides: (&Term, &Term),
    reduced: (&Result<Term, EvalStopped>, &Result<Term, EvalStopped>),
    whnf_verdict: Option<&Convertible>,
    env: &DeltaEnv,
) -> Option<String> {
    let (left, right) = match reduced {
        (Err(stop), _) | (_, Err(stop)) => return Some(stop_note(stop)),
        (Ok(left), Ok(right)) => (left, right),
    };
    if let Some(Convertible::Stopped(stop)) = whnf_verdict {
        return Some(stop_note(stop));
    }
    let reached: BTreeSet<String> = global_names(left)
        .into_iter()
        .chain(global_names(right))
        .collect();
    if let Some((name, stop)) = env.dropped.iter().find(|(name, _)| reached.contains(*name)) {
        return Some(dropped_note(name, stop));
    }
    let closed = left.free_vars().is_empty() && right.free_vars().is_empty();
    if let (false, Some(Convertible::No { first_mismatch })) = (closed, whnf_verdict) {
        let (a, b) = first_mismatch;
        return Some(format!(
            "the sides first differ at `{}` and `{}`",
            Renaming::display(a),
            Renaming::display(b)
        ));
    }
    let env_free = |side: &Term| canonicalize_numerals(&crate::eval::eval(side));
    let changed = *left != env_free(sides.0) || *right != env_free(sides.1);
    changed.then(|| {
        format!(
            "after unfolding total definitions, the sides are `{}` and `{}`",
            Renaming::display(left),
            Renaming::display(right)
        )
    })
}

/// Discharge each obligation (§2.3): both sides are renamed apart (D2 of
/// 29.9.26c) and reduce, under `budget`, to values equal up to numeral
/// spelling (D8); failing that, and only when both CBV sides returned, the
/// WHNF comparison runs over the same environment (D1). Otherwise the
/// obligation's index is returned with its note. A stop is a failure, never an
/// abort and never "equal" (D7).
#[must_use]
pub fn discharge(
    obligations: &[Sides<'_>],
    env: &DeltaEnv,
    budget: usize,
) -> Vec<DischargeFailure> {
    let whnf_globals = unfoldable_globals(&env.globals);
    let mut failures = Vec::new();
    for (index, (left, right)) in obligations.iter().enumerate() {
        let (left_side, right_side) = rename_apart(left, right);
        let left = reduce(&left_side, env, budget);
        let right = reduce(&right_side, env, budget);
        if matches!((&left, &right), (Ok(l), Ok(r)) if l == r) {
            continue;
        }
        let by_name = left.is_ok()
            && right.is_ok()
            && unfoldable_by_name(&left_side)
            && unfoldable_by_name(&right_side);
        let whnf_verdict =
            by_name.then(|| convertible(&left_side, &right_side, &whnf_globals, budget));
        if whnf_verdict == Some(Convertible::Yes) {
            continue;
        }
        let note = failure_note(
            (&left_side, &right_side),
            (&left, &right),
            whnf_verdict.as_ref(),
            env,
        );
        failures.push(DischargeFailure { index, note });
    }
    failures
}
