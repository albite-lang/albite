//! A foreign call as a taint seed on the seam's carried path (ADR 3.10.26q).
//!
//! Every extern wrapper is non-recursive, so the plan never retains one: its
//! seed reaches the engine only through `carried()`. These drive the externs
//! with the wrapper term the self-host now builds — `extern_call "__c_<sym>" []`.

use super::*;
use crate::ffi::terms::ext::tg_term_extern_call;

/// Stream `wrapper` (whose payload calls `symbol`) and a proof that mentions
/// it through both phases, and return the failure count.
fn proof_over_wrapper(symbol: &str) -> usize {
    tg_init();
    tg_termination_reset();
    let wrapper_payload = || unsafe { tg_term_extern_call(cstr(symbol)) };
    let proof_payload = || unsafe { tg_term_app(tg_term_global(cstr("wrapper")), tg_term_zero()) };
    unsafe {
        tg_termination_note_item(cstr("thm"), false, std::ptr::null(), true);
        tg_termination_declare(cstr("wrapper"));
        tg_termination_declare(cstr("thm"));
        assert!(tg_termination_add_def(
            cstr("wrapper"),
            tg_type_nat(),
            wrapper_payload()
        ));
        assert!(tg_termination_add_def(
            cstr("thm"),
            nat_to_nat(),
            proof_payload()
        ));
    }
    assert_eq!(
        tg_termination_plan(true),
        0,
        "nothing recurses: all carried"
    );
    unsafe {
        tg_termination_add_def(cstr("wrapper"), tg_type_nat(), wrapper_payload());
        tg_termination_add_def(cstr("thm"), nat_to_nat(), proof_payload());
    }
    tg_termination_check() as usize
}

// 3.10.26q AC2: a reduced foreign wrapper is a carried seed, and the proof
// over it is the proof-boundary wire kind, naming the extern.
#[test]
fn a_proof_over_a_carried_foreign_wrapper_is_rejected() {
    assert_eq!(proof_over_wrapper("__c_g"), 1);
    let (kind, function, message) = rendered(0);
    assert_eq!(kind, "1", "the proof boundary, not a descent failure");
    assert_eq!(function, "thm");
    assert!(
        message.contains("reaches foreign code") && message.contains("`g`"),
        "the engine's own wording, naming the unprefixed extern: {message}"
    );
}

// 3.10.26q AC3: the registry's `pure` exemption holds on the seam.
#[test]
fn a_proof_over_a_pure_extern_is_admitted() {
    assert_eq!(proof_over_wrapper("__c_tg_string_compare"), 0);
}

// 3.10.26q AC3a / D4: reducing a name again replaces its foreign state.
//
// Since 13.10.26m D5 a name reduced twice is two definitions sharing a bare
// name, so it carries partial taint and `thm` is still refused — but as a
// partial dependency, not a foreign one: the newer, pure payload cleared the
// foreign seed.
#[test]
fn a_later_payload_without_a_foreign_call_clears_the_seed() {
    tg_init();
    tg_termination_reset();
    unsafe {
        tg_termination_note_item(cstr("thm"), false, std::ptr::null(), true);
        tg_termination_declare(cstr("g"));
        tg_termination_declare(cstr("thm"));
        tg_termination_add_def(cstr("g"), tg_type_nat(), tg_term_extern_call(cstr("__c_g")));
        tg_termination_add_def(cstr("g"), tg_type_nat(), tg_term_zero());
        tg_termination_add_def(
            cstr("thm"),
            nat_to_nat(),
            tg_term_app(tg_term_global(cstr("g")), tg_term_zero()),
        );
    }
    tg_termination_plan(true);
    assert_eq!(
        tg_termination_check(),
        1,
        "a repeated name taints the proof"
    );
    let reason = with_registry(|registry| registry.failures()[0].reason.clone());
    assert!(
        matches!(
            reason,
            crate::terms::termination::FailureReason::PartialInProof { .. }
        ),
        "the newer, pure payload cleared the foreign seed: {reason:?}"
    );
}

// 3.10.26q AC3a, the converse: a later foreign payload seeds a name an earlier
// pure one left clean, so replacement is not merely "remove".
#[test]
fn a_later_foreign_payload_seeds_a_previously_clean_name() {
    tg_init();
    tg_termination_reset();
    unsafe {
        tg_termination_note_item(cstr("thm"), false, std::ptr::null(), true);
        tg_termination_declare(cstr("g"));
        tg_termination_declare(cstr("thm"));
        tg_termination_add_def(cstr("g"), tg_type_nat(), tg_term_zero());
        tg_termination_add_def(cstr("g"), tg_type_nat(), tg_term_extern_call(cstr("__c_g")));
        tg_termination_add_def(
            cstr("thm"),
            nat_to_nat(),
            tg_term_app(tg_term_global(cstr("g")), tg_term_zero()),
        );
    }
    tg_termination_plan(true);
    assert_eq!(tg_termination_check(), 1);
}

#[test]
fn a_foreign_rejection_travels_as_the_proof_boundary_kind() {
    // The wire must not demote a foreign rejection to a descent failure
    // (ADR 3.10.26c D6): the self-host reports it through its proof-boundary
    // diagnostic (ADR 3.10.26q D6).
    let failure = crate::terms::termination::TerminationFailure {
        function: "thm".to_string(),
        group: Vec::new(),
        span: None,
        reason: crate::terms::termination::FailureReason::ForeignInProof {
            extern_symbol: "g".to_string(),
            via: vec!["g".to_string()],
        },
    };

    assert_eq!(
        super::super::wire::failure_kind(&failure),
        super::super::wire::PARTIAL_IN_PROOF
    );
}
