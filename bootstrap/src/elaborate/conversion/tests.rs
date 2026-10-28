//! ADR 28.9.26d AC 4: discharge admits exactly `Total` ∩ effect-free, within
//! budget, with a twin for each half. Pure: every test injects its candidate
//! bodies and admission map, so nothing here needs the driver.

use std::collections::{BTreeMap, HashMap};

use albite_core::terms::termination::AdmissionState;
use albite_core::{Context, Term, Type};

use super::numerals::canonicalize_numerals;
use super::*;
use crate::elaborate::termination::TerminationInput;
use crate::span::Span;

const LIST: &str = "type Lst = Nil | Cons(Nat, Lst)
fn two() -> Nat { 2 }
fn length(l: Lst) -> Nat { match l { Nil => 0, Cons(h, t) => 1 + length(t) } }
fn three() -> Nat { length(Cons(1, Cons(2, Cons(3, Nil)))) }
fn length_two_lhs() -> Nat { length(Cons(1, Cons(2, Nil))) }
fn open_lhs(n: Nat) -> Nat { length(Cons(n, Nil)) }
fn open_sum_lhs(n: Nat) -> Nat { two() + n }
fn open_sum_rhs(n: Nat) -> Nat { 2 + n }
fn count(n: Nat) -> Nat { if n == 0 { 0 } else { count(n - 1) } }
fn big() -> Nat { count(1000) }
fn main() -> Nat { 0 }";

/// Elaborated bodies and their real admission verdicts.
struct Fixture {
    candidates: HashMap<String, Term>,
    admission: BTreeMap<String, AdmissionState>,
}

fn fixture(source: &str) -> Fixture {
    let (ast, parse_errors) = crate::parser::parse(source);
    assert!(parse_errors.is_empty(), "{parse_errors:?}");
    let mut ctx = Context::new();
    let output = crate::elaborate::elaborate_with_warnings(&ast, &mut ctx, None)
        .unwrap_or_else(|errors| panic!("fixture must elaborate: {errors:?}"));
    let report = TerminationInput::from_meta(&output.termination_meta).check(&output.defs);
    Fixture {
        candidates: crate::comparator::eval::defs_as_globals(&output.defs),
        admission: report.admission,
    }
}

impl Fixture {
    fn body(&self, name: &str) -> Term {
        self.candidates[name].strip_spans()
    }

    /// The body under a one-parameter function's `Lambda` — an open term.
    fn open_body(&self, name: &str) -> Term {
        match self.body(name) {
            Term::Lambda(_, _, body) => *body,
            other => panic!("`{name}` is not a function: {other}"),
        }
    }

    fn with_state(mut self, name: &str, state: AdmissionState) -> Self {
        self.admission.insert(name.to_string(), state);
        self
    }

    fn run(&self, obligations: &[ConversionObligation], budget: usize) -> Vec<ElabError> {
        let env = delta_globals(&self.candidates, &self.admission, obligations, budget);
        discharge(obligations, &env, budget)
    }
}

pub(super) fn obligation(left: Term, right: Term) -> ConversionObligation {
    ConversionObligation {
        error: ElabError::invalid_refl(Span::new(0, 4), left.clone(), right.clone()),
        left,
        right,
        ty: Type::Nat,
    }
}

fn two_is_two() -> ConversionObligation {
    obligation(Term::Global("two".into()), Term::nat(2))
}

fn notes(error: &ElabError) -> String {
    error
        .notes
        .iter()
        .map(|n| n.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn admission_of(pairs: &[(&str, AdmissionState)]) -> BTreeMap<String, AdmissionState> {
    pairs.iter().map(|(n, s)| (n.to_string(), *s)).collect()
}

#[test]
fn a_total_global_discharges_and_nothing_else_does() {
    let admitted = fixture(LIST);
    assert_eq!(admitted.admission["two"], AdmissionState::Total);
    assert!(admitted.run(&[two_is_two()], 10_000).is_empty());

    for state in [AdmissionState::Partial, AdmissionState::Rejected] {
        let errors = fixture_with("two", state).run(&[two_is_two()], 10_000);
        assert_eq!(errors.len(), 1, "`two` {state:?} must stay folded");
        // Nothing δ could see changed, so the error is byte-identical to today's.
        assert!(errors[0].notes.is_empty(), "{:?}", errors[0].notes);
    }

    let mut absent = fixture(LIST);
    absent.admission.remove("two");
    assert_eq!(
        absent.run(&[two_is_two()], 10_000).len(),
        1,
        "unknown ⇒ not admitted"
    );
}

fn fixture_with(name: &str, state: AdmissionState) -> Fixture {
    fixture(LIST).with_state(name, state)
}

#[test]
fn effectful_bodies_are_excluded_and_unknown_externs_are_not() {
    let console = Term::let_in(
        "_",
        Type::Unit,
        Term::extern_call(
            "__c_tg_println".to_string(),
            vec![Term::StringLit("x".into())],
        ),
        Term::nat(2),
    );
    let unknown = Term::extern_call("tg_not_in_the_registry".to_string(), vec![]);
    let candidates: HashMap<String, Term> = [
        ("loud".to_string(), console),
        ("quiet".to_string(), Term::nat(2)),
        ("opaque".to_string(), unknown),
    ]
    .into();
    let total = AdmissionState::Total;
    let admission = admission_of(&[("loud", total), ("quiet", total), ("opaque", total)]);
    let obligations: Vec<_> = ["loud", "quiet", "opaque"]
        .iter()
        .map(|name| obligation(Term::Global(name.to_string()), Term::nat(2)))
        .collect();

    let env = delta_globals(&candidates, &admission, &obligations, 1_000);
    assert!(
        !env.globals.contains_key("loud"),
        "a Console extern is an effect"
    );
    assert!(env.globals.contains_key("quiet"));
    assert!(
        env.globals.contains_key("opaque"),
        "an unknown extern is not an effect"
    );

    let failures = discharge(&obligations, &env, 1_000);
    let failed: Vec<String> = failures.iter().map(|e| e.message.clone()).collect();
    assert_eq!(
        failures.len(),
        2,
        "loud stays folded, opaque's call stays stuck: {failed:?}"
    );
    assert!(failed.iter().any(|m| m.contains("global:loud")));
    assert!(failed.iter().any(|m| m.contains("global:opaque")));
}

#[test]
fn the_budget_bounds_each_side_and_exhaustion_is_a_noted_failure() {
    let fixture = fixture(LIST);
    let length_two = obligation(fixture.body("length_two_lhs"), Term::nat(2));
    let errors = fixture.run(std::slice::from_ref(&length_two), 5);
    assert_eq!(errors.len(), 1);
    assert!(
        notes(&errors[0]).contains("after 5 steps"),
        "{}",
        notes(&errors[0])
    );
    assert!(fixture.run(&[length_two], 10_000).is_empty());
}

#[test]
fn an_expensive_nullary_is_dropped_under_a_small_budget() {
    let fixture =
        fixture_with("count", AdmissionState::Total).with_state("big", AdmissionState::Total);
    let big_is_zero = obligation(Term::Global("big".into()), Term::Zero);

    let env = delta_globals(
        &fixture.candidates,
        &fixture.admission,
        &[big_is_zero.clone()],
        50,
    );
    assert!(!env.globals.contains_key("big"));
    assert!(
        env.globals.contains_key("count"),
        "a function is never pre-evaluated or dropped"
    );
    let errors = discharge(std::slice::from_ref(&big_is_zero), &env, 50);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        notes(&errors[0]),
        "`big` was not unfolded: its value did not settle within 50 steps"
    );

    assert!(fixture.run(&[big_is_zero], 100_000).is_empty());
}

#[test]
fn a_black_holed_global_fails_with_a_note_not_a_panic() {
    let candidates: HashMap<String, Term> =
        [("loop".to_string(), Term::Global("loop".into()))].into();
    let admission = admission_of(&[("loop", AdmissionState::Total)]);
    let obligations = [obligation(Term::Global("loop".into()), Term::nat(2))];
    let env = delta_globals(&candidates, &admission, &obligations, 1_000);
    let errors = discharge(&obligations, &env, 1_000);
    assert_eq!(errors.len(), 1);
    assert!(
        notes(&errors[0]).contains("black hole: loop → loop"),
        "{}",
        notes(&errors[0])
    );
}

#[test]
fn numerals_compare_insensitive_to_spelling() {
    assert_eq!(
        canonicalize_numerals(&Term::NatLit(2)),
        canonicalize_numerals(&Term::nat(2))
    );
    assert_ne!(
        canonicalize_numerals(&Term::NatLit(2)),
        canonicalize_numerals(&Term::NatLit(3))
    );
    // Inside a larger term too, and past the unary threshold.
    let pair = |a, b| Term::pair(a, b);
    assert_eq!(
        canonicalize_numerals(&pair(Term::NatLit(1), Term::nat(100))),
        canonicalize_numerals(&pair(Term::nat(1), Term::NatLit(100)))
    );
    let errors = fixture(LIST).run(&[obligation(Term::NatLit(2), Term::nat(2))], 100);
    assert!(
        errors.is_empty(),
        "a literal and a Succ chain are the same 2"
    );
}

#[test]
fn a_stop_outranks_a_value_mismatch_in_the_note() {
    let fixture = fixture(LIST);
    // Left stops under the budget; right is a different value.
    let errors = fixture.run(
        &[obligation(fixture.body("length_two_lhs"), Term::nat(7))],
        5,
    );
    assert_eq!(
        notes(&errors[0]),
        "conversion stopped after 5 steps without reaching a normal form"
    );
}

#[test]
fn a_false_total_proof_names_the_unfolded_sides() {
    let errors = fixture(LIST).run(
        &[obligation(Term::Global("two".into()), Term::nat(3))],
        1_000,
    );
    assert_eq!(
        notes(&errors[0]),
        "after unfolding total definitions, the sides are `(succ (succ zero))` and `(succ (succ (succ zero)))`"
    );
}

/// ADR 29.9.26c AC 3 / §2.6: Phase 1's open-term pin, flipped. CBV still
/// sticks on `n`, but the WHNF comparison enters `length` through the open
/// argument; its twin against `2` fails naming the first differing pair.
// 29.9.26c AC5: the only Phase 1 test that moved — its `two()` half is kept.
#[test]
fn open_terms_enter_the_global_under_whnf() {
    let fixture = fixture(LIST);
    let open_length = obligation(fixture.open_body("open_lhs"), Term::nat(1));
    assert!(fixture.run(&[open_length], 10_000).is_empty());
    let twin = obligation(fixture.open_body("open_lhs"), Term::nat(2));
    let errors = fixture.run(&[twin], 10_000);
    assert_eq!(errors.len(), 1, "length(Cons(n, Nil)) is 1, never 2");
    assert!(
        notes(&errors[0]).starts_with("the sides first differ at `"),
        "{}",
        notes(&errors[0])
    );
    assert!(
        !notes(&errors[0]).contains('?'),
        "reserved names are mapped back"
    );
    let open_sum = obligation(
        fixture.open_body("open_sum_lhs"),
        fixture.open_body("open_sum_rhs"),
    );
    assert!(fixture.run(&[open_sum], 10_000).is_empty());
}

#[test]
fn a_nullary_is_pre_evaluated_through_a_recursive_function() {
    let fixture = fixture(LIST);
    assert_eq!(fixture.admission["three"], AdmissionState::Total);
    let three = obligation(Term::Global("three".into()), Term::nat(3));
    let env = delta_globals(
        &fixture.candidates,
        &fixture.admission,
        &[three.clone()],
        10_000,
    );
    assert!(
        env.globals.contains_key("length"),
        "the recursive function stays"
    );
    assert_eq!(canonicalize_numerals(&env.globals["three"]), Term::nat(3));
    assert!(discharge(&[three], &env, 10_000).is_empty());
}

#[test]
fn duplicate_obligations_report_once_and_in_source_order() {
    let fixture = fixture(LIST);
    let at = |start, right| {
        let mut placed = obligation(Term::Global("two".into()), Term::nat(right));
        placed.error.span = Span::new(start, start + 4);
        placed
    };
    let unique = dedup_and_sort(vec![at(40, 3), at(10, 4), at(40, 3)]);
    assert_eq!(unique.len(), 2, "same (file, span, left, right) collapses");
    assert_eq!(unique[0].error.span.start, 10, "sorted by span");
    // Same span, different sides: both kept.
    assert_eq!(dedup_and_sort(vec![at(10, 3), at(10, 4)]).len(), 2);

    let failures = discharge_after_admission(
        &[],
        &fixture.admission,
        vec![at(40, 3), at(40, 3)],
        2,
        false,
    );
    assert_eq!(failures.len(), 1);
}
