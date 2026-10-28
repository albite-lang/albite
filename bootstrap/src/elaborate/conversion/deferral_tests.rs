//! ADR 28.9.26d: the elaboration-time half — the §2.1 deferral table, its
//! predicates, the verbose line, the D15 count, and D3's conservative default.
//! Split from `tests.rs` (the discharge half) to keep both under the size cap.

use std::collections::BTreeMap;

use albite_core::{Context, Term, Type};

use super::*;
use crate::span::Span;

#[test]
fn the_verbose_line_prints_its_counts() {
    assert_eq!(
        discharge_summary(0, 0),
        "conversion: 0 obligation(s), 0 discharged"
    );
    assert_eq!(
        discharge_summary(3, 1),
        "conversion: 3 obligation(s), 1 discharged"
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "D15")]
fn a_lost_obligation_trips_the_count() {
    let left = Term::Global("two".into());
    let lost = ConversionObligation {
        error: ElabError::invalid_refl(Span::new(0, 4), left.clone(), Term::nat(2)),
        left,
        right: Term::nat(2),
        ty: Type::Nat,
    };
    discharge_after_admission(&[], &BTreeMap::new(), vec![lost], 2, false);
}

#[test]
fn every_row_of_the_deferral_table() {
    let global = Term::Global("two".into());
    let effect = Term::extern_call("tg_println".to_string(), vec![]);
    let plain = Term::nat(2);
    // equal → Equal, whatever else holds
    assert_eq!(
        deferral(true, true, (&global, &plain), (&effect, &plain)),
        Verdict::Equal
    );
    // unequal, deferral off → Reject
    assert_eq!(
        deferral(false, false, (&global, &plain), (&global, &plain)),
        Verdict::Reject
    );
    // unequal, on, no global in either normal form → Reject
    assert_eq!(
        deferral(false, true, (&plain, &plain), (&global, &plain)),
        Verdict::Reject
    );
    // unequal, on, a global, an effect in either side → Reject
    assert_eq!(
        deferral(false, true, (&global, &plain), (&effect, &plain)),
        Verdict::Reject
    );
    assert_eq!(
        deferral(false, true, (&plain, &global), (&plain, &effect)),
        Verdict::Reject
    );
    // unequal, on, a global on either side, no effect → Defer
    assert_eq!(
        deferral(false, true, (&global, &plain), (&global, &plain)),
        Verdict::Defer
    );
    assert_eq!(
        deferral(false, true, (&plain, &global), (&plain, &global)),
        Verdict::Defer
    );
}

#[test]
fn a_global_is_found_however_deep() {
    assert!(mentions_global(&Term::succ(Term::Global("x".into()))));
    assert!(!mentions_global(&Term::succ(Term::Zero)));
    assert!(performs_effect(&Term::succ(Term::extern_call(
        "tg_print".to_string(),
        vec![]
    ))));
    assert!(!performs_effect(&Term::extern_call(
        "tg_string_compare".to_string(),
        vec![]
    )));
}

/// AC 6 / D3: a library entry never defers, so a δ proof is today's E0071 there.
#[test]
fn a_library_entry_refuses_a_delta_proof() {
    let source =
        "fn two() -> Nat { 2 }\ntheorem two_is_two(): two() == 2 { refl }\nfn main() -> Nat { 0 }";
    let (ast, parse_errors) = crate::parser::parse(source);
    assert!(parse_errors.is_empty(), "{parse_errors:?}");
    let errors = crate::elaborate::elaborate(&ast, &mut Context::new())
        .expect_err("deferral is off outside the per-module walk");
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0071"),
        "{errors:?}"
    );
}
