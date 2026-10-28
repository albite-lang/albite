//! AC 2 — the proof state: goal order, grafting, the hole spelling.

use std::collections::BTreeMap;

use albite_core::{Term, Type};

use super::eq_nat;
use crate::elaborate::kernel::HYPOTHESIS_PREFIX;
use crate::elaborate::tactics::matching::PATTERN_VAR_PREFIX;
use crate::elaborate::tactics::state::{hole_name, Partial, ProofState, UnsolvedMeta, HOLE_PREFIX};

/// A partial with one hole per meta in `holes`, applied to `head` in order.
fn applied(head: &str, holes: &[usize]) -> Partial {
    let mut template = Term::var(head);
    for hole in holes {
        template = Term::app(template, Term::var(hole_name(*hole)));
    }
    Partial {
        template,
        holes: holes.iter().map(|h| (hole_name(*h), *h)).collect(),
    }
}

// ADR 2.10.26c AC2
#[test]
fn refine_puts_new_goals_first_in_order() {
    let mut state = ProofState::new(Type::Unit);
    let first = state.fresh_meta(vec![], Type::Nat);
    let second = state.fresh_meta(vec![], Type::Bool);
    state.refine(0, applied("pair", &[first, second]));
    assert_eq!(state.goals, vec![first, second]);
    // An older goal stays behind the new ones.
    let inner = state.fresh_meta(vec![], Type::String);
    state.refine(first, applied("wrap", &[inner]));
    assert_eq!(state.goals, vec![inner, second]);
    assert_eq!(state.focus(), Some(inner));
}

// ADR 2.10.26c AC2: the must-fail twin of capture-avoiding substitution —
// `Term::substitute` would rename the `λh` and the proof would be wrong.
#[test]
fn zonk_grafts_without_renaming() {
    let arrow = Type::arrow(
        eq_nat(Term::Zero, Term::Zero),
        eq_nat(Term::Zero, Term::Zero),
    );
    let mut state = ProofState::new(arrow);
    let body = state.fresh_meta(vec![("h".into(), Type::Nat)], Type::Nat);
    state.refine(
        0,
        Partial {
            template: Term::lambda("h", Type::Nat, Term::var(hole_name(body))),
            holes: vec![(hole_name(body), body)],
        },
    );
    state.refine(body, Partial::closed(Term::var("h")));
    assert_eq!(
        state.zonk(0),
        Ok(Term::lambda("h", Type::Nat, Term::var("h")))
    );
    assert!(state.goals.is_empty());
}

// ADR 2.10.26c AC2
#[test]
fn zonk_of_an_unsolved_meta_is_an_error() {
    let mut state = ProofState::new(Type::Unit);
    let open = state.fresh_meta(vec![], Type::Nat);
    state.refine(0, applied("f", &[open]));
    assert_eq!(state.zonk(0), Err(UnsolvedMeta(open)));
    assert_eq!(ProofState::new(Type::Unit).zonk(0), Err(UnsolvedMeta(0)));
}

#[test]
fn zonk_with_an_override_stands_in_for_the_solution() {
    let mut state = ProofState::new(Type::Unit);
    let open = state.fresh_meta(vec![], Type::Nat);
    state.refine(0, applied("f", &[open]));
    let overrides = BTreeMap::from([(open, Term::var("h"))]);
    assert_eq!(
        state.zonk_with(0, &overrides),
        Ok(Term::app(Term::var("f"), Term::var("h")))
    );
}

/// A hole under a span is still grafted; a name that is not a hole is kept.
#[test]
fn graft_reaches_under_a_span_and_leaves_other_names() {
    let mut state = ProofState::new(Type::Unit);
    let open = state.fresh_meta(vec![], Type::Nat);
    let span = albite_core::TermSpan::new(1, 2);
    let template = Term::app(
        Term::var("keep"),
        Term::Spanned(Box::new(Term::var(hole_name(open))), span),
    );
    state.refine(
        0,
        Partial {
            template,
            holes: vec![(hole_name(open), open)],
        },
    );
    state.refine(open, Partial::closed(Term::Zero));
    assert_eq!(
        state.zonk(0),
        Ok(Term::app(
            Term::var("keep"),
            Term::Spanned(Box::new(Term::Zero), span)
        ))
    );
}

// ADR 3.10.26e AC2: `induction`'s holes sit in a `natind`'s base and step;
// the motive, its binder and the scrutinee are kept as written.
#[test]
fn graft_walks_a_natind() {
    let mut state = ProofState::new(Type::Unit);
    let base = state.fresh_meta(vec![], Type::Nat);
    let step = state.fresh_meta(vec![], Type::Nat);
    let motive = Type::arrow(Type::Nat, eq_nat(Term::var("n"), Term::var("n")));
    let natind_with = |base_case: Term, step_case: Term| {
        Term::natind("n", motive.clone(), base_case, step_case, Term::var("n"))
    };
    state.refine(
        0,
        Partial {
            template: natind_with(Term::var(hole_name(base)), Term::var(hole_name(step))),
            holes: vec![(hole_name(base), base), (hole_name(step), step)],
        },
    );
    state.refine(base, Partial::closed(Term::Zero));
    state.refine(step, Partial::closed(Term::var("s")));
    assert_eq!(state.zonk(0), Ok(natind_with(Term::Zero, Term::var("s"))));
}

// ADR 2.10.26c AC2 (P0.4): every internal spelling fails `is_ident_start`
// and is none of the elaborator's synthesised prefixes.
#[test]
fn the_hole_spelling_is_not_a_source_name() {
    for prefix in [HOLE_PREFIX, HYPOTHESIS_PREFIX, PATTERN_VAR_PREFIX] {
        let first = prefix.chars().next().expect("non-empty prefix");
        assert!(!(first.is_ascii_alphabetic() || first == '_'), "{prefix}");
        for synthesised in ["@", "α_", "#", "?", "′"] {
            assert!(
                !prefix.starts_with(synthesised),
                "{prefix} vs {synthesised}"
            );
        }
        let (tokens, _) = crate::lexer::Lexer::new(prefix).tokenize();
        assert!(
            tokens
                .first()
                .map_or(true, |t| t.kind != crate::token::TokenKind::Ident),
            "{prefix} lexes as an identifier"
        );
    }
    assert_eq!(hole_name(3), format!("{HOLE_PREFIX}3"));
}

// ADR 6.10.26e AC2: `exists`'s annotated `Pair` is walked, both components,
// so a hole in either is grafted.
#[test]
fn zonk_grafts_inside_a_pair() {
    let mut state = ProofState::new(Type::Unit);
    let witness = state.fresh_meta(vec![], Type::Nat);
    let evidence = state.fresh_meta(vec![], Type::Nat);
    let pair_ty = Type::sigma("k", Type::Nat, eq_nat(Term::var("k"), Term::Zero));
    state.refine(
        0,
        Partial {
            template: Term::annot(
                Term::pair(
                    Term::var(hole_name(witness)),
                    Term::var(hole_name(evidence)),
                ),
                pair_ty.clone(),
            ),
            holes: vec![
                (hole_name(witness), witness),
                (hole_name(evidence), evidence),
            ],
        },
    );
    state.refine(witness, Partial::closed(Term::Zero));
    state.refine(evidence, Partial::closed(Term::var("e")));
    assert_eq!(
        state.zonk(0),
        Ok(Term::annot(Term::pair(Term::Zero, Term::var("e")), pair_ty))
    );
}
