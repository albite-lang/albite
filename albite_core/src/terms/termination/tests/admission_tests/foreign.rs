//! The foreign-call seed beside the `#[partial]` one (ADR 3.10.26c): a
//! definition whose term calls an extern the registry does not mark `pure`
//! carries taint, and a proof that reaches it is rejected naming the extern.

use std::collections::BTreeMap;

use crate::eval::extern_registry::{entry_for, ExternKind};
use crate::terms::termination::admission::taint_path;
use crate::terms::termination::{
    analyze, analyze_with_carried, AdmissionState, CarriedDefs, FailureReason, MentionIndex,
    OccurrenceGraph,
};
use crate::terms::Term;
use crate::types::Type;

use crate::terms::termination::tests::fixtures::{var, views, Def};

/// A nullary extern wrapper, as `elaborate_extern_fn` emits it: the body is
/// the `ExternCall`, under the `__c_` prefix the elaborator prepends.
fn extern_wrapper(name: &str, symbol: &str) -> Def {
    Def::new(
        name,
        Type::Nat,
        Term::ExternCall(symbol.to_string(), Vec::new()),
    )
}

/// `name = target()`, an ordinary definition that only calls `target`.
fn caller(name: &str, target: &str) -> Def {
    Def::new(name, Type::Nat, Term::Global(target.to_string()))
}

/// A theorem whose body mentions `target`.
fn theorem_over(target: &str) -> Def {
    caller("thm", target).proof()
}

// 3.10.26c AC1: a wrapper over a non-exempt extern is a seed, and a definition
// that only calls it is tainted transitively — executable code is not rejected.
#[test]
fn a_foreign_call_seeds_taint_that_reaches_its_callers() {
    let defs = [extern_wrapper("g", "__c_g"), caller("wrapper", "g")];
    let report = analyze(&views(&defs));

    assert_eq!(report.tainted, vec!["g".to_string(), "wrapper".to_string()]);
    assert_eq!(report.state_of("g"), AdmissionState::Partial);
    assert_eq!(report.state_of("wrapper"), AdmissionState::Partial);
    assert!(report.is_clean(), "{:?}", report.failures);
    assert_eq!(
        report.foreign_seeds,
        BTreeMap::from([("g".to_string(), "g".to_string())])
    );
}

#[test]
fn a_foreign_call_nested_inside_a_body_still_seeds() {
    let body = Term::Lambda(
        "x".to_string(),
        Type::Nat,
        Box::new(Term::Succ(Box::new(Term::ExternCall(
            "__c_g".to_string(),
            vec![var("x")],
        )))),
    );
    let defs = [Def::new("g", Type::Nat, body)];

    assert_eq!(
        analyze(&views(&defs)).state_of("g"),
        AdmissionState::Partial
    );
}

// 3.10.26c AC2: only the registry's `pure` kind is exempt; `arena` and an
// unknown symbol seed; a `__c_`-prefixed name resolves like its bare symbol.
#[test]
fn only_a_pure_registry_extern_is_exempt() {
    // Non-vacuity: the fixtures rest on these two registry kinds.
    assert_eq!(
        entry_for("tg_string_compare").map(|e| e.kind),
        Some(ExternKind::Pure)
    );
    assert_eq!(
        entry_for("tg_type_nat").map(|e| e.kind),
        Some(ExternKind::Arena)
    );
    let cases = [
        ("__c_tg_string_compare", AdmissionState::Total),
        ("tg_string_compare", AdmissionState::Total),
        ("__c_tg_type_nat", AdmissionState::Partial),
        ("tg_type_nat", AdmissionState::Partial),
        ("__c_never_registered", AdmissionState::Partial),
    ];
    for (symbol, expected) in cases {
        let defs = [extern_wrapper("w", symbol)];
        assert_eq!(analyze(&views(&defs)).state_of("w"), expected, "{symbol}");
    }
}

// 3.10.26c AC3: the chain ends at the wrapper, and the failure names the
// extern's symbol without the `__c_` prefix.
#[test]
fn a_proof_through_a_wrapper_is_rejected_naming_the_chain_and_the_extern() {
    let defs = [
        extern_wrapper("g", "__c_g"),
        caller("wrapper", "g"),
        theorem_over("wrapper"),
    ];
    let graph = OccurrenceGraph::build(defs.iter().map(|def| (def.name.as_str(), &def.term)));
    let index = MentionIndex::build(&views(&defs), &graph, &CarriedDefs::default());

    assert_eq!(
        taint_path("thm", &index),
        Some(vec!["wrapper".to_string(), "g".to_string()])
    );
    let report = analyze(&views(&defs));
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert_eq!(
        report.failures[0].reason,
        FailureReason::ForeignInProof {
            extern_symbol: "g".to_string(),
            via: vec!["wrapper".to_string(), "g".to_string()],
        }
    );
}

#[test]
fn a_proof_that_names_the_wrapper_directly_has_a_one_link_chain() {
    let defs = [extern_wrapper("g", "__c_g"), theorem_over("g")];

    assert_eq!(
        analyze(&views(&defs)).failures[0].reason,
        FailureReason::ForeignInProof {
            extern_symbol: "g".to_string(),
            via: vec!["g".to_string()],
        }
    );
}

#[test]
fn an_annotation_outranks_a_foreign_call_on_the_same_definition() {
    let defs = [extern_wrapper("g", "__c_g").partial(), theorem_over("g")];

    assert_eq!(
        analyze(&views(&defs)).failures[0].reason,
        FailureReason::PartialInProof {
            tainted: "g".to_string(),
            via: Vec::new(),
        }
    );
}

#[test]
fn a_proof_over_a_pure_extern_is_admitted() {
    let defs = [
        extern_wrapper("cmp", "__c_tg_string_compare"),
        theorem_over("cmp"),
    ];
    let report = analyze(&views(&defs));

    assert!(report.is_clean(), "{:?}", report.failures);
    assert!(report.foreign_seeds.is_empty());
}

// 3.10.26c D5: a carried definition has no term, so its foreign seed has to
// arrive in `CarriedDefs` — and a carried *proof* is reached through the
// statement edges carried beside it.
#[test]
fn a_carried_foreign_seed_rejects_a_carried_proof_that_reaches_it() {
    let carried = CarriedDefs {
        mentions: BTreeMap::from([
            ("g".to_string(), Default::default()),
            ("ax".to_string(), ["g".to_string()].into()),
        ]),
        partial: Default::default(),
        foreign: BTreeMap::from([("g".to_string(), "g".to_string())]),
        proofs: ["ax".to_string()].into(),
    };
    let report = analyze_with_carried(&BTreeMap::new(), &carried);

    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert_eq!(report.failures[0].function, "ax");
    assert!(report.failures[0].reason.is_proof_boundary());
    assert_eq!(report.state_of("g"), AdmissionState::Partial);
}
