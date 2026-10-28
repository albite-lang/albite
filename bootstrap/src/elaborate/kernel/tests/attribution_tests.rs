//! AC 4 — attribution names a minimal refused step, over an injected checker.

use std::collections::BTreeMap;

use albite_core::{Context, Term, Type};

use super::{eq_nat, TraceBuilder};
use crate::elaborate::kernel::attribution::{attribute, Culprit, WHOLE_BLOCK};
use crate::elaborate::kernel::{attribute_refusal, RecordPresentation};
use crate::span::Span;

fn steps() -> Vec<(&'static str, Span)> {
    vec![
        ("intro", Span::new(1, 2)),
        ("apply", Span::new(3, 4)),
        ("refl", Span::new(5, 6)),
        ("exact", Span::new(7, 8)),
    ]
}

// ADR 2.10.26c AC4: the walk runs last to first and stops at the first
// refusal, so a step that contains a refused one is never blamed for it.
#[test]
fn attribution_names_a_minimal_refused_step() {
    let mut checked = Vec::new();
    let culprit = attribute(&steps(), Span::new(0, 9), |index| {
        checked.push(index);
        index <= 2 // `intro`, `apply` and `refl` refuse; `exact` is accepted
    });
    assert_eq!(
        culprit,
        Culprit {
            tactic: "refl".into(),
            span: Span::new(5, 6)
        }
    );
    assert_eq!(
        checked,
        vec![3, 2],
        "stops at the first refusal from the end"
    );
}

// ADR 2.10.26c AC4: no single step refused — a context fault — names the
// block, at its first tactic.
#[test]
fn attribution_falls_back_to_the_block() {
    let culprit = attribute(&steps(), Span::new(0, 9), |_| false);
    assert_eq!(
        culprit,
        Culprit {
            tactic: WHOLE_BLOCK.into(),
            span: Span::new(1, 2)
        }
    );
    let empty = attribute(&[], Span::new(0, 9), |_| true);
    assert_eq!(
        empty.span,
        Span::new(0, 9),
        "an empty block is named by its own span"
    );
}

// ADR 2.10.26c AC4: each step is checked in a context holding the theorem's
// value parameters and its goal's own `intro` locals.
#[test]
fn attribution_context_holds_the_parameters_and_the_locals() {
    let n_eq = eq_nat(Term::var("n"), Term::var("n"));
    let mut trace = TraceBuilder::new(
        Type::arrow(n_eq.clone(), n_eq.clone()),
        vec![("n".into(), Type::Nat)],
    );
    let body = trace.goal(0, &[("h", n_eq.clone())], n_eq.clone());
    let hole = crate::elaborate::tactics::state::hole_name(body);
    trace.step(
        "intro",
        1,
        Term::lambda("h", n_eq.clone(), Term::var(hole)),
        &[body],
    );
    // `exact h` is sound only where `n` and `h` are both bound.
    trace.step("exact", 3, Term::var("h"), &[]);
    let culprit = attribute_refusal(
        &trace.finish(),
        &Context::new(),
        &BTreeMap::new(),
        &RecordPresentation::EMPTY,
    );
    assert_eq!(
        culprit.tactic, WHOLE_BLOCK,
        "neither step is refused on its own"
    );
}
