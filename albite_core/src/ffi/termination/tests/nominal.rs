//! A structurally encoded parameter named back to its declared type (ADR
//! 3.9.26f).
//!
//! The shape is the self-host's: `MaybeE` reaches itself only through `E`, so
//! the self-hosted encoder leaves it an unwrapped `(Unit + E)` — which Phase 1
//! refuses as a root — while the bootstrap μ-wraps it and admits the pair.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::ffi::termination::nominal_roots::{nominally_encoded_names, DeclaredKind, DeclaredType};
use crate::terms::Term;
use crate::types::Type;

const SUM: u64 = 0;
const RECORD: u64 = 1;
const ALIAS: u64 = 2;

/// Note one declared type and the names its body spells, through the externs.
fn note(name: &str, kind: u64, constructors: u64, references: &[&str]) {
    unsafe {
        assert!(tg_termination_note_type(cstr(name), kind, constructors));
        for referenced in references {
            assert!(tg_termination_note_type_ref(cstr(name), cstr(referenced)));
        }
    }
}

/// `MaybeE = NoE | SomeE(E)`, `E = Lit(Nat) | Blk(MaybeE)`, and two
/// non-recursive controls: `Two` (two constructors) and `Three` (three).
fn note_corpus() {
    note("MaybeE", SUM, 2, &["E"]);
    note("E", SUM, 2, &["Nat", "MaybeE"]);
    note("Two", SUM, 2, &["Nat"]);
    note("Three", SUM, 3, &[]);
}

fn var(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn call(callee: &str, argument: Term) -> Term {
    Term::App(
        Box::new(Term::Global(callee.to_string())),
        Box::new(argument),
    )
}

fn case(scrutinee: Term, left: (&str, Term), right: (&str, Term)) -> Term {
    Term::Case(
        Box::new(scrutinee),
        left.0.to_string(),
        Box::new(left.1),
        right.0.to_string(),
        Box::new(right.1),
    )
}

/// `fm(x: MaybeE) = case x of NoE => 0 | SomeE(e) => fe(e)`, with `x`'s
/// binder in the self-host's structural spelling.
fn maybe_member() -> Term {
    let structural = Type::Sum(Box::new(Type::Unit), Box::new(Type::TyVar("E".into())));
    Term::Lambda(
        "x".into(),
        structural,
        Box::new(case(
            var("x"),
            ("u", Term::Zero),
            ("e", call("fe", var("e"))),
        )),
    )
}

/// `fe(x: E) = case unfold x of Lit(n) => n | Blk(m) => fm(m)` — a μ-encoding,
/// so `fe` alone was always a candidate.
fn expr_member() -> Term {
    let body = Type::Sum(Box::new(Type::Nat), Box::new(Type::TyVar("MaybeE".into())));
    let mu = Type::Mu("E".into(), Box::new(body));
    Term::Lambda(
        "x".into(),
        mu.clone(),
        Box::new(case(
            Term::Unfold(mu, Box::new(var("x"))),
            ("n", var("n")),
            ("m", call("fm", var("m"))),
        )),
    )
}

/// Run the pair through the registry and return the rejection count.
fn check_pair(note_the_parameter: bool) -> usize {
    if note_the_parameter {
        assert!(unsafe { tg_termination_note_param_type(cstr("fm"), 0, cstr("MaybeE")) });
    }
    with_registry(|registry| {
        for name in ["fm", "fe"] {
            registry.declare(name).expect("declare");
        }
        for _stream in 0..2 {
            registry
                .add_def("fm", Type::Nat, maybe_member())
                .expect("fm");
            registry
                .add_def("fe", Type::Nat, expr_member())
                .expect("fe");
            if registry.phase() == Phase::Reduce {
                assert_eq!(registry.plan(true).expect("plan"), 2);
            }
        }
        registry.check().expect("check")
    })
}

#[test]
fn the_plan_names_cluster_members_and_flat_adts_but_not_a_two_constructor_sum() {
    tg_init();
    tg_termination_reset();
    note_corpus();
    assert_eq!(tg_termination_plan(true), 0);
    let named = with_registry(|registry| registry.nominal_types_for_test());
    let expected: BTreeSet<String> = ["E", "MaybeE", "Three"].map(String::from).into();
    assert_eq!(named, expected);
}

#[test]
fn a_taint_only_plan_names_nothing() {
    tg_init();
    tg_termination_reset();
    note_corpus();
    assert_eq!(tg_termination_plan(false), 0);
    assert!(with_registry(|registry| registry.nominal_types_for_test()).is_empty());
}

// 3.9.26f AC2: the structurally encoded cluster member is accepted as a
// decreasing root once its declared name travels with it — and refused, as
// before, when it does not.
#[test]
fn a_structural_cluster_member_is_a_root_only_when_its_name_travels() {
    tg_init();
    tg_termination_reset();
    note_corpus();
    assert_eq!(check_pair(false), 1, "the unnamed sum is refused as a root");

    tg_init();
    tg_termination_reset();
    note_corpus();
    assert_eq!(
        check_pair(true),
        0,
        "the named sum descends like the bootstrap's μ"
    );
}

#[test]
fn a_renote_replaces_the_previous_reading_of_a_type() {
    tg_init();
    tg_termination_reset();
    note_corpus();
    // `E` re-collected with no reference back to `MaybeE`: the cycle is gone.
    note("E", SUM, 2, &["Nat"]);
    assert_eq!(tg_termination_plan(true), 0);
    let named = with_registry(|registry| registry.nominal_types_for_test());
    let expected: BTreeSet<String> = ["Three"].map(String::from).into();
    assert_eq!(named, expected);
}

#[test]
fn a_reference_for_an_unnoted_type_or_an_unknown_kind_is_refused() {
    tg_init();
    tg_termination_reset();
    assert!(!unsafe { tg_termination_note_type_ref(cstr("Ghost"), cstr("Nat")) });
    assert!(last_error().contains("`Ghost`"));
    assert!(!unsafe { tg_termination_note_type(cstr("T"), 7, 1) });
    assert!(last_error().contains("type kind 7"));
}

#[test]
fn an_unreadable_name_is_refused_on_every_type_extern() {
    tg_init();
    tg_termination_reset();
    let null = std::ptr::null();
    assert!(!unsafe { tg_termination_note_type(null, SUM, 1) });
    assert!(last_error().contains("note_type"));
    assert!(!unsafe { tg_termination_note_type_ref(null, cstr("T")) });
    assert!(!unsafe { tg_termination_note_type_ref(cstr("T"), null) });
    assert!(last_error().contains("note_type_ref"));
    assert!(!unsafe { tg_termination_note_param_type(null, 0, cstr("T")) });
    assert!(!unsafe { tg_termination_note_param_type(cstr("f"), 0, null) });
    assert!(last_error().contains("note_param_type"));
}

#[test]
fn a_parameter_note_after_the_plan_is_refused_and_names_the_call() {
    tg_init();
    tg_termination_reset();
    assert_eq!(tg_termination_plan(false), 0);
    assert!(!unsafe { tg_termination_note_param_type(cstr("f"), 0, cstr("T")) });
    assert!(last_error().contains("note_parameter_type"));
}

fn declared(kind: DeclaredKind, constructors: usize, references: &[&str]) -> DeclaredType {
    DeclaredType {
        kind,
        constructors,
        references: references.iter().map(|name| (*name).to_string()).collect(),
    }
}

#[test]
fn a_record_is_named_back_only_on_a_cycle_and_an_alias_never_is() {
    let types = BTreeMap::from([
        (
            "Flat".to_string(),
            declared(DeclaredKind::Record, 1, &["Nat"]),
        ),
        (
            "Node".to_string(),
            declared(DeclaredKind::Record, 1, &["Kids"]),
        ),
        (
            "Kids".to_string(),
            declared(DeclaredKind::Alias, 0, &["List", "Node"]),
        ),
        (
            "Many".to_string(),
            declared(DeclaredKind::Alias, 0, &["Many"]),
        ),
    ]);
    assert_eq!(
        nominally_encoded_names(&types),
        BTreeSet::from(["Node".to_string()]),
        "a cycle through an alias still counts; the alias itself is never named"
    );
}

#[test]
fn a_self_referencing_singleton_is_a_cluster_of_one() {
    let types = BTreeMap::from([
        ("Rec".to_string(), declared(DeclaredKind::Sum, 2, &["Rec"])),
        (
            "Leafy".to_string(),
            declared(DeclaredKind::Sum, 2, &["Nat"]),
        ),
    ]);
    assert_eq!(
        nominally_encoded_names(&types),
        BTreeSet::from(["Rec".to_string()])
    );
}

#[test]
fn record_and_alias_codes_decode_and_nothing_else_does() {
    assert_eq!(DeclaredKind::from_code(RECORD), Some(DeclaredKind::Record));
    assert_eq!(DeclaredKind::from_code(ALIAS), Some(DeclaredKind::Alias));
    assert_eq!(DeclaredKind::from_code(3), None);
}
