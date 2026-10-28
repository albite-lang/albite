//! `direct-mt-callers` end to end over a corpus holding the native core's
//! optimised unit (ADR 28.10.26e): the unit is skipped, and the skip is
//! reported without breaking `--json`.

use super::tests::F_UNIT;
use super::*;
use crate::doctor::checks::ir_checks::corpus::fixtures::corpus_at;

/// A unit that calls `f$direct_mt` from outside its shim/self pair — the
/// shape O2's inlining leaves in the core.
const FOREIGN_CALLER: &str = "declare void @\"f$direct_mt\"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16), i64)\n\n\
define i64 @\"g$direct\"(ptr %0) {\nentry:\n  call void @\"f$direct_mt\"(ptr %buf, i64 1)\n  ret i64 0\n}\n";

// 28.10.26e AC6 — the same IR is a finding in a program file and skipped in
// the core's unit.
#[test]
fn a_foreign_call_in_the_core_unit_is_skipped_and_elsewhere_is_a_finding() {
    let in_core = corpus_at(&[
        ("f.ll", F_UNIT),
        ("__native_core/native_core.ll", FOREIGN_CALLER),
    ]);
    let verdict = audit_directory(in_core.path(), false);
    assert_eq!(verdict, DirectMtVerdict::Clean);
    assert_eq!(
        format!("{:?}", verdict.exit()),
        format!("{:?}", ExitCode::SUCCESS)
    );

    let in_program = corpus_at(&[("f.ll", F_UNIT), ("m1.ll", FOREIGN_CALLER)]);
    let verdict = audit_directory(in_program.path(), false);
    assert_eq!(verdict, DirectMtVerdict::Findings(1));
    assert_eq!(
        format!("{:?}", verdict.exit()),
        format!("{:?}", ExitCode::FAILURE)
    );
}

#[test]
fn the_text_form_puts_the_note_directly_after_the_summary() {
    let dir = corpus_at(&[
        ("f.ll", F_UNIT),
        ("__native_core/native_core.ll", FOREIGN_CALLER),
    ]);
    let output = audit_output(dir.path(), false);
    assert!(
        output.stdout[0].starts_with("summary: files=1 "),
        "{:?}",
        output.stdout
    );
    assert!(
        output.stdout[1]
            .starts_with("note: skipped 1 optimised unit file(s) (__native_core/native_core.ll)"),
        "{:?}",
        output.stdout
    );
    assert!(output.stderr.is_empty());
}

// 28.10.26e AC5 — D2's JSON arm: the note goes to stderr, stdout stays one
// document.
#[test]
fn the_json_form_stays_one_parseable_document_with_the_note_on_stderr() {
    let dir = corpus_at(&[
        ("f.ll", F_UNIT),
        ("__native_core/native_core.ll", FOREIGN_CALLER),
    ]);
    let output = audit_output(dir.path(), true);
    assert_eq!(output.verdict, DirectMtVerdict::Clean);
    assert_eq!(output.stdout.len(), 1, "{:?}", output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&output.stdout[0]).expect("json");
    assert!(parsed.is_object(), "{parsed}");
    assert_eq!(output.stderr.len(), 1, "{:?}", output.stderr);
    assert!(
        output.stderr[0].starts_with("note: skipped 1 "),
        "{:?}",
        output.stderr
    );
}

#[test]
fn nothing_is_noted_when_nothing_is_skipped() {
    let dir = corpus_at(&[("f.ll", F_UNIT)]);
    for json in [false, true] {
        let output = audit_output(dir.path(), json);
        assert!(output.stderr.is_empty(), "json={json}");
        assert!(
            output
                .stdout
                .iter()
                .all(|line| !line.starts_with("note: skipped")),
            "json={json}: {:?}",
            output.stdout
        );
    }
}
