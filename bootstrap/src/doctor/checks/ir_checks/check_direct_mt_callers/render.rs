//! The text and JSON forms of a `direct-mt-callers` report — values, so the
//! tests assert what is printed rather than that something was.

use std::path::Path;

use serde::Serialize;

use super::{DirectMtVerdict, FindingKind, Report};

impl FindingKind {
    /// The kebab-case label the text form prints, matching the JSON spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::ForeignCall => "foreign-call",
            Self::AddressEscape => "address-escape",
            Self::DeclareMismatch => "declare-mismatch",
            Self::UnresolvedDeclare => "unresolved-declare",
        }
    }
}

impl DirectMtVerdict {
    fn as_str(&self) -> &'static str {
        match self {
            Self::NotADirectory => "not-a-directory",
            Self::UnreadableCorpus => "unreadable-corpus",
            Self::NoDirectMt => "no-direct-mt",
            Self::Findings(_) => "findings",
            Self::Clean => "clean",
        }
    }
}

/// The non-vacuity line every run prints first (D3a).
pub(crate) fn summary_line(report: &Report) -> String {
    format!(
        "summary: files={} defined_direct_mt={} calls_examined={} foreign_declares={} findings={}",
        report.files,
        report.defined,
        report.calls_examined,
        report.foreign_declares,
        report.findings.len()
    )
}

/// The human form: summary, then one line per finding or the verdict.
pub(crate) fn render_text(
    report: &Report,
    verdict: &DirectMtVerdict,
    dir: &Path,
    unreadable: usize,
) -> Vec<String> {
    let mut lines = vec![summary_line(report)];
    for f in &report.findings {
        lines.push(format!(
            "  {}:{}: {} in {} → {} — {}",
            f.file,
            f.line_num,
            f.kind.label(),
            f.site,
            f.symbol,
            f.detail
        ));
    }
    lines.push(match verdict {
        DirectMtVerdict::Clean => format!(
            "✓ every $direct_mt in {} is unit-internal: called only by its own shim or itself, \
             never used as a value, every declare resolved and ABI-equal",
            dir.display()
        ),
        DirectMtVerdict::Findings(n) => format!(
            "✗ {n} $direct_mt unit-boundary finding(s) — its ABI is only sound unit-internally \
             (ADR 3.10.26b; codegen-pipeline.md § Codegen-Unit ABI Boundary)"
        ),
        DirectMtVerdict::NoDirectMt => {
            "✗ no $direct_mt is defined in this corpus — nothing was audited".to_string()
        }
        DirectMtVerdict::UnreadableCorpus => format!(
            "✗ {unreadable} unreadable .ll file(s), or none at all — nothing can be vouched for"
        ),
        DirectMtVerdict::NotADirectory => format!("✗ {} is not a directory", dir.display()),
    });
    lines
}

#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    verdict: &'static str,
    #[serde(flatten)]
    report: &'a Report,
}

/// The `--json` form: `schema_version: 1`, the verdict, then the report.
pub(crate) fn render_json(report: &Report, verdict: &DirectMtVerdict) -> String {
    let json = JsonReport {
        schema_version: 1,
        verdict: verdict.as_str(),
        report,
    };
    serde_json::to_string_pretty(&json).unwrap_or_default()
}
