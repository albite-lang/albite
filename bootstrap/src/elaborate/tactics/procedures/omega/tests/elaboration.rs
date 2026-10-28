//! `omega` through the elaborator (ADR 6.10.26f AC 2, AC 3): its refusals
//! and their codes, and the corpus through the kernel gate.

use std::path::Path;

use crate::elaborate::error::{OmegaFailure, TacticErrorKind};
use crate::elaborate::tactics::procedures::omega::{OMEGA_NEEDS_A_SHAPE, OMEGA_NEEDS_THE_LIBRARY};
use crate::elaborate::tactics::tests::tactic_error;

/// The local `Le` family every source below states its goals with.
const LE: &str = "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n";

/// E0082's `why` for `source`, asserting the tactic is `omega`.
fn omega_not_applicable(source: &str) -> String {
    let kind = tactic_error(&format!("{LE}{source}"));
    assert_eq!(kind.code(), "E0082", "{kind:?}");
    let TacticErrorKind::TacticNotApplicable { tactic, why, .. } = kind else {
        panic!("expected E0082, got {kind:?}");
    };
    assert_eq!(tactic, "omega");
    why
}

/// E0088's failure for `source`.
fn omega_failure(source: &str) -> OmegaFailure {
    let kind = tactic_error(&format!("{LE}{source}"));
    assert_eq!(kind.code(), "E0088", "{kind:?}");
    let TacticErrorKind::OmegaUnproved(failure) = kind else {
        panic!("expected E0088, got {kind:?}");
    };
    failure
}

// ADR 6.10.26f AC3 (D2, D7): a goal of neither shape is E0082.
#[test]
fn a_bool_goal_is_e0082() {
    let why = omega_not_applicable("theorem t(a: Bool) -> Eq<Bool, a, a> by { omega }");
    assert_eq!(why, OMEGA_NEEDS_A_SHAPE);
}

// ADR 6.10.26f AC3 (D5): a goal that needs a citation, with no `mod nat_arith;`.
#[test]
fn omega_without_the_library_is_e0082() {
    let why = omega_not_applicable(
        "theorem t(a: Nat, b: Nat, c: Nat, h1: Le<a, b>, h2: Le<b, c>) -> Le<a, c> by { omega }",
    );
    assert_eq!(why, OMEGA_NEEDS_THE_LIBRARY);
    assert!(why.contains("mod nat_arith;"));
}

// ADR 6.10.26f AC3: nothing to act on after the goal closed.
#[test]
fn omega_after_the_goal_closed_is_e0082() {
    let why = omega_not_applicable("theorem t(n: Nat) -> Eq<Nat, n, n> by { refl; omega }");
    assert_eq!(why, crate::elaborate::error::NO_GOAL_LEFT);
}

// ADR 6.10.26f AC3 (D7, second message): the false goal is refused before any
// citation, so no library is needed to reach it.
#[test]
fn a_false_goal_is_e0088_no_witness() {
    let failure = omega_failure("theorem t(n: Nat) -> Le<n + 1, n> by { omega }");
    assert_eq!(
        failure.message(),
        "`omega`: no witness: `t - s` is `-1`, with a negative term `-1`"
    );
}

// ADR 6.10.26f AC3 (D7, third message): the unused hypothesis is named.
#[test]
fn an_unoriented_hypothesis_is_e0088_naming_it() {
    let failure = omega_failure(
        "theorem t(a: Nat, b: Nat, c: Nat, d: Nat, h: Le<a + b, c + d>) -> Le<b + a, d + c> \
         by { omega }",
    );
    let OmegaFailure::Unoriented { unused } = &failure else {
        panic!("expected unused, got {failure:?}");
    };
    assert_eq!(unused[0].0, "h");
}

// ADR 6.10.26f AC2 (D2): an `intro`'d local is a candidate beside the
// theorem's parameters, which it does not hide — `h1` is still read, and
// named unused.
#[test]
fn a_goal_local_joins_the_parameters_as_a_candidate() {
    let failure = omega_failure(
        "theorem t(a: Nat, b: Nat, c: Nat, d: Nat, h1: Le<a + b, c + d>) \
         -> Le<a, a> -> Le<b + a, d + c> by { intro h2; omega }",
    );
    let OmegaFailure::Unoriented { unused } = &failure else {
        panic!("expected unused, got {failure:?}");
    };
    let names: Vec<&str> = unused.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["h1"]);
}

// ADR 6.10.26f AC3 (D7): E0088 is reported at the `omega` step.
#[test]
fn e0088_is_reported_at_the_omega_step() {
    let source = format!("{LE}theorem t(n: Nat) -> Le<n + 1, n> by {{ omega }}");
    let errors = crate::elaborate::tests::elab_err(&source);
    let error = errors
        .iter()
        .find(|e| e.kind.code() == "E0088")
        .unwrap_or_else(|| panic!("no E0088 in {errors:?}"));
    let span = error.span;
    assert_eq!(&source[span.start as usize..span.end as usize], "omega");
}

// ADR 6.10.26f AC2: the suite builds cold through the module-tree walk, whose
// admission gate discharges every tactic proof in the kernel.
#[test]
fn the_omega_corpus_passes_the_kernel_gate() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/compiler/test_tactic_omega.tg");
    let project = crate::driver::elaborate_project_cold(&corpus, 20)
        .unwrap_or_else(|errors| panic!("the corpus is refused: {errors:?}"));
    for theorem in [
        "omega_le_refl",
        "omega_le_trans",
        "omega_le_antisymm",
        "omega_cancel_summand",
        "omega_from_eq",
        "ex3_le_of_lt",
        "ex3_chain",
    ] {
        assert!(
            project.defs.iter().any(|def| def.name.ends_with(theorem)),
            "{theorem}"
        );
    }
}
