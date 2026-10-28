//! Tests for `direct-mt-callers` (ADR 3.10.26b): one `.ll` string per file,
//! scanned whole, asserted as whole reports where the shape is the point.

use super::parse::{direct_mt_references, parse_direct_mt_header};
use super::render::{render_json, render_text, summary_line};
use super::*;
use crate::doctor::checks::ir_checks::corpus::fixtures::{corpus_at, corpus_of};

/// `f`'s unit: the shim calls the entry, the entry musttails itself.
pub(super) const F_UNIT: &str = r#"
define void @"f$direct_mt"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16) %0, i64 %1) {
entry:
  musttail call void @"f$direct_mt"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16) %0, i64 %1)
  ret void
}

define { i64, i64 } @"f$direct"(ptr %0, i64 %1) {
entry:
  %sret_buf = alloca { i64, i64 }, align 8
  call void @"f$direct_mt"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16) %sret_buf, i64 %1)
  %r = load { i64, i64 }, ptr %sret_buf, align 8
  ret { i64, i64 } %r
}
"#;

/// The declaration a foreign unit carries for `f$direct_mt`, value names gone.
const F_DECLARE: &str =
    "declare void @\"f$direct_mt\"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16), i64)";

fn corpus(files: &[(&str, &str)]) -> Vec<(PathBuf, String)> {
    files
        .iter()
        .map(|(name, text)| (PathBuf::from(name), (*text).to_string()))
        .collect()
}

fn kinds(report: &Report) -> Vec<FindingKind> {
    report.findings.iter().map(|f| f.kind).collect()
}

// 3.10.26b AC1
#[test]
fn a_shim_and_a_self_edge_are_internal() {
    let report = scan(&corpus(&[("f.ll", F_UNIT)]));
    assert_eq!(
        report,
        Report {
            files: 1,
            defined: 1,
            calls_examined: 2,
            foreign_declares: 0,
            findings: vec![],
        }
    );
}

// 3.10.26b AC1
#[test]
fn a_foreign_call_is_a_finding() {
    let g_unit = format!(
        "{F_DECLARE}\n\ndefine i64 @\"g$direct\"(ptr %0) {{\nentry:\n  \
         call void @\"f$direct_mt\"(ptr %buf, i64 1)\n  ret i64 0\n}}\n"
    );
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", &g_unit)]));
    assert_eq!(report.foreign_declares, 1);
    assert_eq!(report.calls_examined, 3);
    assert_eq!(
        report.findings,
        vec![DirectMtFinding {
            kind: FindingKind::ForeignCall,
            file: "g.ll".to_string(),
            site: "g$direct".to_string(),
            symbol: "f$direct_mt".to_string(),
            line_num: 5,
            detail: "called from outside its own shim/self pair".to_string(),
        }]
    );
}

// 3.10.26b AC1 — D2: the file is the unit, so the name alone is not enough.
#[test]
fn a_same_named_caller_in_another_file_is_foreign() {
    let other = "define i64 @\"f$direct\"(ptr %0) {\nentry:\n  \
                 call void @\"f$direct_mt\"(ptr %b, i64 1)\n  ret i64 0\n}\n";
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("other.ll", other)]));
    assert_eq!(kinds(&report), vec![FindingKind::ForeignCall]);
    assert_eq!(report.findings[0].file, "other.ll");
    assert_eq!(report.findings[0].site, "f$direct");
}

// 3.10.26b AC1
#[test]
fn an_invoke_is_a_call() {
    let g_unit = "define void @g(ptr %0) {\nentry:\n  \
             invoke void @\"f$direct_mt\"(ptr %b, i64 1) to label %ok unwind label %bad\n}\n";
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", g_unit)]));
    assert_eq!(kinds(&report), vec![FindingKind::ForeignCall]);
    assert_eq!(report.findings[0].site, "g");
}

/// A bare (unquoted) callee is still the symbol — LLVM allows `$` unquoted.
#[test]
fn a_bare_symbol_spelling_is_recognised() {
    let g_unit =
        "define void @g() {\nentry:\n  call void @f$direct_mt(ptr %b, i64 1)\n  ret void\n}\n";
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", g_unit)]));
    assert_eq!(kinds(&report), vec![FindingKind::ForeignCall]);
}

/// A shim calling some OTHER base's entry is foreign even in that entry's file.
#[test]
fn a_shim_of_another_base_is_foreign() {
    let unit = format!(
        "{F_UNIT}\ndefine void @\"h$direct\"() {{\nentry:\n  \
         call void @\"f$direct_mt\"(ptr %b, i64 1)\n  ret void\n}}\n"
    );
    let report = scan(&corpus(&[("f.ll", &unit)]));
    assert_eq!(kinds(&report), vec![FindingKind::ForeignCall]);
    assert_eq!(report.findings[0].site, "h$direct");
}

// 3.10.26b AC2
#[test]
fn a_stored_address_escapes() {
    let g_unit = "define void @g(ptr %slot) {\nentry:\n  store ptr @\"f$direct_mt\", ptr %slot, align 8\n  ret void\n}\n";
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", g_unit)]));
    assert_eq!(kinds(&report), vec![FindingKind::AddressEscape]);
    assert_eq!(report.findings[0].line_num, 3);
    assert_eq!(report.calls_examined, 2, "a store is not a call");
}

// 3.10.26b AC2 — the callee is internal; the argument is the escape.
#[test]
fn an_address_passed_as_an_argument_escapes() {
    let unit = F_UNIT.replace(
        "call void @\"f$direct_mt\"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16) %sret_buf, i64 %1)",
        "call void @\"f$direct_mt\"(ptr noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16) %sret_buf, ptr @\"f$direct_mt\")",
    );
    let report = scan(&corpus(&[("f.ll", &unit)]));
    assert_eq!(kinds(&report), vec![FindingKind::AddressEscape]);
    assert_eq!(report.findings[0].site, "f$direct");
}

/// References outside any function body (module-scope globals) are data.
#[test]
fn a_module_scope_reference_is_not_examined() {
    let g_unit = "@table = global ptr @\"f$direct_mt\"\n";
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", g_unit)]));
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

// 3.10.26b AC2a
#[test]
fn a_type_equal_foreign_declare_is_counted_not_flagged() {
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", F_DECLARE)]));
    assert_eq!(report.foreign_declares, 1);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

// 3.10.26b AC2a
#[test]
fn a_mismatched_declare_is_a_finding() {
    let declare = F_DECLARE.replace(", i64)", ", i32)");
    let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", &declare)]));
    assert_eq!(kinds(&report), vec![FindingKind::DeclareMismatch]);
    let finding = &report.findings[0];
    assert_eq!((finding.file.as_str(), finding.line_num), ("g.ll", 1));
    assert!(
        finding.detail.contains("defined in f.ll"),
        "{}",
        finding.detail
    );
}

// 3.10.26b AC2a — attributes are compared, not stripped.
#[test]
fn an_sret_mismatch_is_a_finding() {
    let wrong_type = F_DECLARE.replace("sret({ i64, i64 })", "sret({ i64, i32 })");
    let moved_slot = "declare void @\"f$direct_mt\"(ptr, i64 noalias nonnull sret({ i64, i64 }) align 8 dereferenceable(16))";
    for declare in [wrong_type.as_str(), moved_slot] {
        let report = scan(&corpus(&[("f.ll", F_UNIT), ("g.ll", declare)]));
        assert_eq!(
            kinds(&report),
            vec![FindingKind::DeclareMismatch],
            "{declare}"
        );
    }
}

// 3.10.26b AC2a
#[test]
fn an_unresolved_declare_is_a_finding() {
    let report = scan(&corpus(&[
        ("g.ll", F_DECLARE),
        ("h.ll", F_UNIT.replace("f$", "h$").as_str()),
    ]));
    assert_eq!(kinds(&report), vec![FindingKind::UnresolvedDeclare]);
    assert_eq!(report.findings[0].detail, "no define in the corpus");

    let twice = scan(&corpus(&[
        ("f.ll", F_UNIT),
        ("f2.ll", F_UNIT),
        ("g.ll", F_DECLARE),
    ]));
    assert_eq!(kinds(&twice), vec![FindingKind::UnresolvedDeclare]);
    assert_eq!(twice.findings[0].detail, "2 defines in the corpus");
    assert_eq!(twice.defined, 2);
}

/// Linkage words a `define` may carry are not part of the ABI signature.
#[test]
fn linkage_does_not_make_a_mismatch() {
    let internal = F_UNIT.replace(
        "define void @\"f$direct_mt\"",
        "define internal void @\"f$direct_mt\"",
    );
    let report = scan(&corpus(&[("f.ll", &internal), ("g.ll", F_DECLARE)]));
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn headers_normalise_value_names_and_whitespace() {
    let define = parse_direct_mt_header(
        "define ptr @\"k$direct_mt\"(ptr  noalias %0, { i64, i64 } %pair) {",
    )
    .expect("a $direct_mt define");
    let declare = parse_direct_mt_header("declare ptr @\"k$direct_mt\"(ptr noalias, { i64, i64 })")
        .expect("a $direct_mt declare");
    assert_eq!(define.signature, "ptr (ptr noalias, { i64, i64 })");
    assert_eq!(define.signature, declare.signature);
    assert_eq!(define.symbol, "k$direct_mt");
    assert_eq!(
        parse_direct_mt_header("declare void @\"k$direct_mt\"()").map(|h| h.signature),
        Some("void ()".to_string())
    );
    assert!(parse_direct_mt_header("define ptr @\"k$direct\"(ptr %0) {").is_none());
    assert!(parse_direct_mt_header("  call void @\"k$direct_mt\"()").is_none());
    assert!(parse_direct_mt_header("declare void @\"k$direct_mt\"(ptr").is_none());
}

#[test]
fn references_are_every_direct_mt_symbol_on_the_line() {
    assert_eq!(
        direct_mt_references(
            "call void @\"a$direct_mt\"(ptr @b$direct_mt, ptr @c, ptr @\"d$direct\")"
        ),
        vec!["a$direct_mt", "b$direct_mt"]
    );
    assert!(direct_mt_references("ret void").is_empty());
}

// 3.10.26b AC3
#[test]
fn a_corpus_with_no_direct_mt_is_refused() {
    let dir = corpus_of(&["define void @main() {\nentry:\n  ret void\n}\n"]);
    let verdict = audit_directory(dir.path(), false);
    assert_eq!(verdict, DirectMtVerdict::NoDirectMt);
    assert_eq!(
        format!("{:?}", verdict.exit()),
        format!("{:?}", ExitCode::from(2))
    );
}

#[test]
fn the_verdict_maps_each_outcome_to_its_exit_code() {
    let clean = scan(&corpus(&[("f.ll", F_UNIT)]));
    assert_eq!(DirectMtVerdict::classify(&clean, 0), DirectMtVerdict::Clean);
    assert_eq!(
        DirectMtVerdict::classify(&clean, 1),
        DirectMtVerdict::UnreadableCorpus
    );
    assert_eq!(
        DirectMtVerdict::classify(&Report::default(), 0),
        DirectMtVerdict::UnreadableCorpus,
        "no files at all is bad input"
    );
    let dirty = scan(&corpus(&[
        ("f.ll", F_UNIT),
        ("g.ll", &F_DECLARE.replace("i64)", "i8)")),
    ]));
    assert_eq!(
        DirectMtVerdict::classify(&dirty, 0),
        DirectMtVerdict::Findings(1)
    );

    let code = |v: DirectMtVerdict| format!("{:?}", v.exit());
    assert_eq!(
        code(DirectMtVerdict::Clean),
        format!("{:?}", ExitCode::SUCCESS)
    );
    assert_eq!(
        code(DirectMtVerdict::Findings(1)),
        format!("{:?}", ExitCode::FAILURE)
    );
    for bad in [
        DirectMtVerdict::NotADirectory,
        DirectMtVerdict::UnreadableCorpus,
        DirectMtVerdict::NoDirectMt,
    ] {
        assert_eq!(code(bad), format!("{:?}", ExitCode::from(2)));
    }
}

#[test]
fn audit_directory_reads_the_corpus_from_disk() {
    let dir = corpus_at(&[("f.ll", F_UNIT), ("sub/g.ll", F_DECLARE)]);
    assert_eq!(audit_directory(dir.path(), false), DirectMtVerdict::Clean);
    assert_eq!(audit_directory(dir.path(), true), DirectMtVerdict::Clean);
    let missing = dir.path().join("absent");
    assert_eq!(
        audit_directory(&missing, false),
        DirectMtVerdict::NotADirectory
    );
}

#[test]
fn the_text_form_carries_the_counts_and_each_finding() {
    let report = scan(&corpus(&[
        ("f.ll", F_UNIT),
        ("g.ll", "declare void @\"x$direct_mt\"()"),
    ]));
    assert_eq!(
        summary_line(&report),
        "summary: files=2 defined_direct_mt=1 calls_examined=2 foreign_declares=1 findings=1"
    );
    let verdict = DirectMtVerdict::classify(&report, 0);
    let lines = render_text(&report, &verdict, Path::new("ll"), 0);
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[1],
        "  g.ll:1: unresolved-declare in x$direct_mt → x$direct_mt — no define in the corpus"
    );
    assert!(
        lines[2].starts_with("✗ 1 $direct_mt unit-boundary finding(s)"),
        "{}",
        lines[2]
    );

    let clean = scan(&corpus(&[("f.ll", F_UNIT)]));
    let clean_lines = render_text(&clean, &DirectMtVerdict::Clean, Path::new("ll"), 0);
    assert!(
        clean_lines[1].starts_with("✓ every $direct_mt in ll"),
        "{}",
        clean_lines[1]
    );
    for (verdict, prefix) in [
        (DirectMtVerdict::NoDirectMt, "✗ no $direct_mt"),
        (DirectMtVerdict::UnreadableCorpus, "✗ 3 unreadable"),
        (DirectMtVerdict::NotADirectory, "✗ ll is not"),
    ] {
        let last = render_text(&Report::default(), &verdict, Path::new("ll"), 3)
            .pop()
            .unwrap();
        assert!(last.starts_with(prefix), "{last}");
    }
}

#[test]
fn the_json_form_is_versioned_and_kebab_cased() {
    let report = scan(&corpus(&[
        ("f.ll", F_UNIT),
        ("g.ll", "declare void @\"x$direct_mt\"()"),
    ]));
    let parsed_report: serde_json::Value =
        serde_json::from_str(&render_json(&report, &DirectMtVerdict::Findings(1))).expect("json");
    assert_eq!(parsed_report["schema_version"], 1);
    assert_eq!(parsed_report["verdict"], "findings");
    assert_eq!(parsed_report["defined"], 1);
    assert_eq!(parsed_report["foreign_declares"], 1);
    assert_eq!(parsed_report["findings"][0]["kind"], "unresolved-declare");
    for (verdict, label) in [
        (DirectMtVerdict::Clean, "clean"),
        (DirectMtVerdict::NoDirectMt, "no-direct-mt"),
        (DirectMtVerdict::UnreadableCorpus, "unreadable-corpus"),
        (DirectMtVerdict::NotADirectory, "not-a-directory"),
    ] {
        let parsed_verdict: serde_json::Value =
            serde_json::from_str(&render_json(&report, &verdict)).unwrap();
        assert_eq!(parsed_verdict["verdict"], label);
    }
    for (kind, label) in [
        (FindingKind::ForeignCall, "foreign-call"),
        (FindingKind::AddressEscape, "address-escape"),
        (FindingKind::DeclareMismatch, "declare-mismatch"),
        (FindingKind::UnresolvedDeclare, "unresolved-declare"),
    ] {
        assert_eq!(kind.label(), label);
        assert_eq!(serde_json::to_value(kind).unwrap(), label);
    }
}
