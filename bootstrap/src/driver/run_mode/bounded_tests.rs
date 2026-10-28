//! Tests for the seams ADR 5.10.26d made public for `fuzz-test`'s
//! properties: the step-bounded `main` (`bounded_eval`, moved here from the
//! playground), the residual's head label, and the quiet, cache-honouring
//! file elaboration with its cold/warm equivalence check.

use albite_core::eval::EvalStopped;
use albite_core::Term;

use super::{bounded_eval, head_label};
use crate::driver::{compare_elab_outputs, elaborate_file_quiet, QuietElabFailure};
use crate::elaborate::ElabOutput;

/// Elaborate `source` in-process (no cache, no file).
fn elaborate(source: &str) -> ElabOutput {
    let (ast, errors) = crate::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    let mut ctx = albite_core::Context::new();
    crate::elaborate_with_warnings(&ast, &mut ctx, None).expect("elaborates")
}

fn main_term(output: &ElabOutput) -> Term {
    output
        .defs
        .iter()
        .find(|d| d.name == "main")
        .unwrap()
        .term
        .term
        .clone()
}

#[test]
fn bounded_eval_evaluates_main_against_the_other_globals() {
    // `main`'s value differs from `two`'s, so evaluating the wrong def fails.
    let output = elaborate("fn two() -> Nat { 2 }\n\nfn main() -> Nat { two() + 1 }\n");
    let value = bounded_eval(&main_term(&output), &output, 10_000).expect("a value");
    assert_eq!(crate::driver::format_value(&value), "3");
}

#[test]
fn bounded_eval_stops_at_its_step_limit() {
    let output = elaborate("fn two() -> Nat { 2 }\n\nfn main() -> Nat { two() }\n");
    let stopped = bounded_eval(&main_term(&output), &output, 0).unwrap_err();
    assert!(
        matches!(stopped, EvalStopped::StepLimit { limit: 0 }),
        "{stopped:?}"
    );
}

#[test]
fn bounded_eval_does_not_offer_main_as_a_global() {
    // `main` is the job at hand, not a lookup: a term naming it stays stuck.
    let output = elaborate("fn main() -> Nat { 1 }\n");
    let residual = bounded_eval(&Term::Global("main".into()), &output, 1_000).unwrap();
    assert!(!residual.is_value(), "{residual:?}");
}

#[test]
fn head_label_names_the_residual_shape() {
    let app = Term::App(Box::new(Term::Zero), Box::new(Term::Zero));
    assert_eq!(head_label(&app), "application");
    assert_eq!(head_label(&Term::Global("g".into())), "global");
    assert_eq!(head_label(&Term::Zero), "term");
}

/// A `main.tg` in a scratch dir the guard removes — cache dir included.
fn program(tag: &str, source: &str) -> (crate::scratch::ScratchDir, std::path::PathBuf) {
    let dir = crate::scratch::ScratchDir::new(tag);
    let path = dir.join("main.tg");
    std::fs::write(&path, source).unwrap();
    (dir, path)
}

#[test]
fn elaborate_file_quiet_returns_the_output_or_the_errors() {
    let ok = program("quiet-ok", "fn main() -> Nat { 1 }\n");
    let output = elaborate_file_quiet(&ok.1).expect("accepted");
    assert!(output.defs.iter().any(|d| d.name == "main"));
    // Equivalence with itself is empty; a dropped def is not.
    assert!(compare_elab_outputs(&output, &output).is_empty());
    let mut dropped = elaborate_file_quiet(&ok.1).unwrap();
    dropped.defs.clear();
    assert!(!compare_elab_outputs(&output, &dropped).is_empty());

    let bad = program("quiet-bad", "fn main() -> Nat { true }\n");
    match elaborate_file_quiet(&bad.1) {
        Err(QuietElabFailure::Rejected(errors)) => {
            assert!(
                errors.iter().any(|e| e.kind.code() == "E0010"),
                "{errors:?}"
            );
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    let missing = ok.0.join("no-such-dir").join("main.tg");
    assert!(matches!(
        elaborate_file_quiet(&missing),
        Err(QuietElabFailure::Pipeline(_))
    ));
}
