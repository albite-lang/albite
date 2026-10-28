//! Tests for Signature Collection failure guardrails (ADR 13.5.26g).

use crate::elaborate::{ElabError, ElabErrorKind};
use crate::span::Span;
use albite_core::Type;

/// Helper: create an E0001 (UndefinedVariable) error.
fn undefined_var_error(name: &str) -> ElabError {
    ElabError::new(
        Span::new(0, 0),
        ElabErrorKind::UndefinedVariable(name.to_string()),
    )
}

/// Helper: create an E0005 (ModuleNotFound) error.
fn module_not_found_error(module: &str) -> ElabError {
    ElabError::new(
        Span::new(0, 0),
        ElabErrorKind::ModuleNotFound {
            module: module.to_string(),
            suggestion: None,
        },
    )
}

/// Helper: create an E0010 (TypeMismatch) error — NOT a resolution error.
fn type_mismatch_error() -> ElabError {
    ElabError::new(
        Span::new(0, 0),
        ElabErrorKind::TypeMismatch {
            expected: Type::TyVar("Nat".to_string()),
            found: Type::TyVar("String".to_string()),
        },
    )
}

// ADR 29.9.26f AC7: the note prints in human mode and not in JSON mode.
#[test]
fn signature_collection_note_is_human_only() {
    use crate::driver::diagnostics::ErrorFormat;
    let errors = vec![undefined_var_error("foo")];
    let human = super::super::phases::signature_collection_note(&errors, false, ErrorFormat::Human)
        .expect("human mode prints the note");
    assert!(
        human.contains("Signature Collection global collection failed"),
        "{human}"
    );
    assert_eq!(
        super::super::phases::signature_collection_note(&errors, false, ErrorFormat::Json),
        None
    );
}

#[test]
fn annotate_adds_note_to_undefined_variable() {
    let mut errors = vec![undefined_var_error("foo")];
    super::super::phases::annotate_errors_for_signature_collection_failure(&mut errors);
    assert_eq!(errors[0].notes.len(), 1);
    assert!(errors[0].notes[0].message.contains("Signature Collection"));
}

#[test]
fn annotate_adds_note_to_module_not_found() {
    let mut errors = vec![module_not_found_error("elab::env::resolve")];
    super::super::phases::annotate_errors_for_signature_collection_failure(&mut errors);
    assert_eq!(errors[0].notes.len(), 1);
    assert!(errors[0].notes[0].message.contains("Signature Collection"));
}

#[test]
fn annotate_skips_non_resolution_errors() {
    let mut errors = vec![type_mismatch_error()];
    super::super::phases::annotate_errors_for_signature_collection_failure(&mut errors);
    assert!(errors[0].notes.is_empty());
}

#[test]
fn annotate_mixed_errors_only_annotates_resolution() {
    let mut errors = vec![
        undefined_var_error("foo"),
        type_mismatch_error(),
        module_not_found_error("bad::path"),
    ];
    super::super::phases::annotate_errors_for_signature_collection_failure(&mut errors);
    // E0001: annotated
    assert_eq!(errors[0].notes.len(), 1);
    // E0010: not annotated
    assert!(errors[1].notes.is_empty());
    // E0005: annotated
    assert_eq!(errors[2].notes.len(), 1);
}

#[test]
fn signature_collection_ok_defaults_to_true() {
    let acc = super::super::accumulator::ModuleTreeAccumulator::new();
    assert!(acc.signature_collection_ok);
}

#[test]
fn gating_annotates_when_signature_collection_failed() {
    // ok == false → the driver's map_err path must annotate.
    let mut errors = vec![undefined_var_error("foo")];
    super::super::phases::annotate_if_signature_collection_failed(false, &mut errors);
    assert_eq!(errors[0].notes.len(), 1);
    assert!(errors[0].notes[0].message.contains("Signature Collection"));
}

#[test]
fn gating_skips_when_signature_collection_succeeded() {
    // ok == true → no annotation (the `!` guard; deleting it would annotate here).
    let mut errors = vec![undefined_var_error("foo")];
    super::super::phases::annotate_if_signature_collection_failed(true, &mut errors);
    assert!(errors[0].notes.is_empty());
}

#[test]
fn annotated_error_display_includes_hint() {
    let mut errors = vec![undefined_var_error("missing_fn")];
    super::super::phases::annotate_errors_for_signature_collection_failure(&mut errors);
    let rendered = format!("{}", errors[0]);
    assert!(
        rendered.contains("note: Signature Collection global collection failed"),
        "rendered error should contain Signature Collection hint, got: {rendered}"
    );
    assert!(
        rendered.contains("albite doctor check module signature-collection"),
        "rendered error should contain remediation command, got: {rendered}"
    );
}

#[test]
fn annotated_module_not_found_display_includes_hint() {
    let mut errors = vec![module_not_found_error("elab::env::resolve")];
    super::super::phases::annotate_errors_for_signature_collection_failure(&mut errors);
    let rendered = format!("{}", errors[0]);
    assert!(
        rendered.contains("note:"),
        "rendered error should have a note line, got: {rendered}"
    );
    assert!(
        rendered.contains("bad import in another module"),
        "hint should mention bad import as root cause, got: {rendered}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// unreached_note — how much of the tree the walk never saw
// (ADR 7.8.26d retrospective; the coverage fix itself is ADR 14.8.26g)
// ─────────────────────────────────────────────────────────────────────────

use crate::driver::per_module::phases::unreached_note;

/// A complete walk must render exactly as it did before this note existed.
#[test]
fn a_complete_walk_says_nothing() {
    assert_eq!(unreached_note(12, 12), None);
}

#[test]
fn a_single_module_project_that_completed_says_nothing() {
    assert_eq!(unreached_note(1, 1), None);
}

/// The walk accumulates and reaches everything since ADR 14.8.26g, so this
/// note firing at all means an early exit crept back in — the note is a
/// tripwire, and its wording must not claim a mechanism that no longer exists.
#[test]
fn an_interrupted_walk_reports_the_remainder() {
    let note = unreached_note(12, 5).expect("7 modules were skipped");
    assert!(
        note.contains("7 of 12 modules were not elaborated"),
        "{note}"
    );
    assert!(note.contains("did not examine the whole project"), "{note}");
}

/// The count is the *difference*, not the walked figure — an off-by-one here
/// would read plausibly and be wrong in the direction that understates.
#[test]
fn the_reported_count_is_what_was_missed() {
    assert!(unreached_note(100, 99).unwrap().contains("1 of 100"));
    assert!(unreached_note(100, 1).unwrap().contains("99 of 100"));
}

#[test]
fn a_lone_module_is_singular() {
    assert!(unreached_note(1, 0)
        .unwrap()
        .contains("1 of 1 module was not elaborated"));
}

/// The noun agrees with the total and the verb with the *unreached* count, so
/// a one-module shortfall inside a larger tree still reads correctly.
#[test]
fn a_single_unreached_module_takes_a_singular_verb() {
    let note = unreached_note(3, 2).expect("1 module was skipped");
    assert!(note.contains("1 of 3 modules was not elaborated"), "{note}");
}

/// `walked` can never exceed `total`, but a saturating subtraction must not
/// invent a note if some future counter double-counts.
#[test]
fn walking_more_than_exists_says_nothing() {
    assert_eq!(unreached_note(3, 4), None);
}

#[test]
fn an_empty_tree_says_nothing() {
    assert_eq!(unreached_note(0, 0), None);
}

// ─────────────────────────────────────────────────────────────────────────
// ParsedModule::module_count — the denominator
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn a_leaf_module_counts_itself() {
    assert_eq!(super::make_parsed_module(vec![]).module_count(), 1);
}

#[test]
fn a_tree_counts_every_module_at_every_depth() {
    let mut root = super::make_parsed_module(vec![]);
    let mut child = super::make_parsed_module(vec![]);
    child.submodules.push(super::make_parsed_module(vec![]));
    child.submodules.push(super::make_parsed_module(vec![]));
    root.submodules.push(child);
    root.submodules.push(super::make_parsed_module(vec![]));
    // root + child + 2 grandchildren + sibling
    assert_eq!(root.module_count(), 5);
}

// ─────────────────────────────────────────────────────────────────────────
// the counter and the failure-path worker
// ─────────────────────────────────────────────────────────────────────────

use crate::driver::per_module::accumulator::ModuleTreeAccumulator;
use crate::driver::per_module::phases::finish_failed_body_elaboration;

#[test]
fn a_fresh_accumulator_has_walked_nothing() {
    assert_eq!(ModuleTreeAccumulator::new().modules_walked, 0);
}

#[test]
fn each_noted_module_advances_the_count_by_one() {
    let mut acc = ModuleTreeAccumulator::new();
    for expected in 1..=3 {
        acc.note_module_walked();
        assert_eq!(acc.modules_walked, expected);
    }
}

/// Parallel workers each count their own subtree, so the merge must *sum*
/// them — the field is meaningless if a merge overwrites or multiplies.
#[test]
fn merging_a_worker_sums_the_walked_counts() {
    let mut main_acc = ModuleTreeAccumulator::new();
    main_acc.note_module_walked();
    main_acc.note_module_walked();
    let mut worker = ModuleTreeAccumulator::new();
    worker.note_module_walked();
    main_acc.merge_worker(worker);
    assert_eq!(main_acc.modules_walked, 3);
}

#[test]
fn merging_an_untouched_worker_changes_nothing() {
    let mut main_acc = ModuleTreeAccumulator::new();
    main_acc.note_module_walked();
    main_acc.merge_worker(ModuleTreeAccumulator::new());
    assert_eq!(main_acc.modules_walked, 1);
}

/// A run that reached every module annotates but has nothing to report.
#[test]
fn finishing_a_complete_walk_reports_no_note() {
    let mut acc = ModuleTreeAccumulator::new();
    acc.note_module_walked();
    let tree = super::make_parsed_module(vec![]);
    let mut errors = [undefined_var_error("x")];
    assert_eq!(
        finish_failed_body_elaboration(&acc, &tree, &mut errors),
        None
    );
}

/// The two effects are independent: this asserts the note *and* that the
/// Signature-Collection annotation still ran alongside it.
#[test]
fn finishing_an_interrupted_walk_reports_and_annotates() {
    let mut root = super::make_parsed_module(vec![]);
    root.submodules.push(super::make_parsed_module(vec![]));
    let mut acc = ModuleTreeAccumulator::new(); // walked nothing of a 2-module tree
    acc.signature_collection_ok = false; // the precondition for annotating
    let mut errors = [undefined_var_error("x")];

    let note =
        finish_failed_body_elaboration(&acc, &root, &mut errors).expect("2 modules were unreached");

    assert!(
        note.contains("2 of 2 modules were not elaborated"),
        "{note}"
    );
    assert!(
        !errors[0].notes.is_empty(),
        "signature-collection annotation must still run"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// an imported non-positive type is reported once, from its own file
// (ADR 9.10.26h)
// ─────────────────────────────────────────────────────────────────────────

/// The `positivity_cross_module` golden's shape: `main.tg` imports `Bad`,
/// which `types.tg` defines. The header comment in `main.tg` is long enough
/// that `Bad`'s byte range in `types.tg` also lands inside `main.tg`.
const IMPORTED_NON_POSITIVE: [(&str, &str); 2] = [
    (
        "main.tg",
        "// header line one padding padding padding padding\n// header two\n\
         pub mod types;\n\nuse types::{Bad};\n\nfn take(b: Bad) -> Nat { 0 }\n\n\
         fn main() -> Nat { 0 }\n",
    ),
    (
        "types.tg",
        "// The violation lives here; `main.tg` only imports it.\n\
         pub type Bad = Mk(Bad -> Bad)\n",
    ),
];

fn positivity_files(errors: &[ElabError]) -> Vec<String> {
    errors
        .iter()
        .filter(|e| e.kind.code() == "E0061")
        .map(|e| {
            e.file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect()
}

// ADR 9.10.26h AC2: one violation, one raw report — the list
// `take_accumulated_errors_ordered` returns holds a single E0061, in types.tg.
#[test]
fn an_imported_non_positive_type_is_reported_once_from_its_own_file() {
    let errors = super::elaborate_tree_errors(&IMPORTED_NON_POSITIVE);
    assert_eq!(
        positivity_files(&errors),
        vec!["types.tg".to_string()],
        "{errors:?}"
    );
}

// ADR 9.10.26h AC2: both channels — the pre-walk copies carry types.tg's path,
// and no module group but types.tg's holds an E0061.
#[test]
fn each_positivity_channel_names_the_defining_file() {
    let groups = super::elaborate_tree_channels(&IMPORTED_NON_POSITIVE);
    let pre_walk = positivity_files(&groups.pre_walk);
    assert!(!pre_walk.is_empty(), "the global pass must report it");
    assert!(pre_walk.iter().all(|f| f == "types.tg"), "{pre_walk:?}");
    for (module, errors) in &groups.modules {
        let files = positivity_files(errors);
        if module == &["types".to_string()] {
            assert_eq!(files, vec!["types.tg".to_string()]);
        } else {
            assert!(files.is_empty(), "module {module:?} re-raised: {files:?}");
        }
    }
}

// ADR 9.10.26h D1: the global pass stamps the DEFINING file, not its
// `current_module`, which is whichever type it collected last — here
// `main.tg`'s `Local`, so the old stamp named the root file.
#[test]
fn the_global_pass_stamps_the_defining_file_not_the_last_registered() {
    let groups = super::elaborate_tree_channels(&[
        (
            "main.tg",
            "pub mod zlast;\npub mod types;\n\nuse types::{Bad};\n\n\
             type Local = MkLocal(Nat)\n\n\
             fn take(b: Bad) -> Nat { 0 }\n\nfn main() -> Nat { 0 }\n",
        ),
        ("types.tg", "pub type Bad = Mk(Bad -> Bad)\n"),
        ("zlast.tg", "pub type Fine = MkFine(Nat)\n"),
    ]);
    let pre_walk = positivity_files(&groups.pre_walk);
    assert_eq!(pre_walk, vec!["types.tg".to_string()], "{pre_walk:?}");
}
