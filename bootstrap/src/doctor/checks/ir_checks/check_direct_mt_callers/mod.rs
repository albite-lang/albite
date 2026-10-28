//! `albite doctor check ir direct-mt-callers` — ADR 3.10.26b.
//!
//! Every `$direct_mt` entry carries an ABI only its own unit agrees on — sret,
//! indirect buffers and (since 2.10.26h) words — and is sound only because it
//! is **unit-internal** (1.7.26e §6.4, 2.7.26b T8, 2.10.26h D5). Under
//! `--emit-llvm` a file is a codegen unit, so the invariant is three clauses
//! over a corpus of `.ll` files:
//!
//! 1. every *call* of `<B>$direct_mt` comes from `<B>$direct` or `<B>$direct_mt`
//!    itself, in the file that defines `<B>$direct_mt` (else `foreign-call`);
//! 2. the address never becomes a value — a reference anywhere but a call's
//!    callee position is `address-escape`, because an indirect call through it
//!    would bypass clause 1;
//! 3. every `declare` resolves to exactly one `define` (else
//!    `unresolved-declare`) and matches its ABI signature, attributes included
//!    (else `declare-mismatch`).
//!
//! A foreign `declare` that resolves and matches is **not** a finding:
//! 2.10.26h measured 2731 of them, all type-equal and none called.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

use crate::doctor::checks::ir_checks::corpus::scan_ll_corpus;
use crate::doctor::checks::ir_checks::textparse::{callee_of, parse_define_name};
use parse::{direct_mt_references, parse_direct_mt_header, symbol_at, HeaderKind};

mod parse;
mod render;
#[cfg(test)]
mod skip_tests;
#[cfg(test)]
mod tests;

/// The `$direct_mt` decomposed-entry suffix (mirrors
/// `albite_codegen::codegen::exec::direct_calls::decompose::DIRECT_MT_SUFFIX`).
pub(crate) const DIRECT_MT_SUFFIX: &str = "$direct_mt";
/// The `$direct` shim suffix — the one foreign-named caller D2 accepts.
const DIRECT_SUFFIX: &str = "$direct";

/// The four ways the invariant can break (ADR 3.10.26b D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    ForeignCall,
    AddressEscape,
    DeclareMismatch,
    UnresolvedDeclare,
}

/// One breach of the invariant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectMtFinding {
    pub kind: FindingKind,
    /// The `.ll` file — the codegen unit — holding the offending line.
    pub file: String,
    /// The function whose body holds the reference; for a declare finding,
    /// the declared symbol itself.
    pub site: String,
    /// The `$direct_mt` symbol referenced or declared.
    pub symbol: String,
    /// 1-indexed line of the offending reference or `declare`.
    pub line_num: usize,
    /// Why: where the symbol is defined, how many defines, or both signatures.
    pub detail: String,
}

/// The whole scan: the non-vacuity counts (D3a) and every finding.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub files: usize,
    pub defined: usize,
    pub calls_examined: usize,
    pub foreign_declares: usize,
    pub findings: Vec<DirectMtFinding>,
}

/// Scan a corpus of `(path, .ll text)` pairs. Pure: the tests drive it with
/// hand-written IR, the command with the files on disk.
pub fn scan(files: &[(PathBuf, String)]) -> Report {
    let mut report = Report {
        files: files.len(),
        ..Report::default()
    };
    // symbol → (defining file, signature) for every define in the corpus.
    let mut defines: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut declares = Vec::new();
    for (path, text) in files {
        let file = path.display().to_string();
        for (i, line) in text.lines().enumerate() {
            let Some(header) = parse_direct_mt_header(line.trim_start()) else {
                continue;
            };
            match header.kind {
                HeaderKind::Define => defines
                    .entry(header.symbol)
                    .or_default()
                    .push((file.clone(), header.signature)),
                HeaderKind::Declare => declares.push((file.clone(), i + 1, header)),
            }
        }
    }
    report.defined = defines.values().map(Vec::len).sum();
    for (path, text) in files {
        scan_bodies(&path.display().to_string(), text, &mut report);
    }
    for (file, line_num, header) in declares {
        report.foreign_declares += 1;
        if let Some(finding) = check_declare(&defines, file, line_num, header) {
            report.findings.push(finding);
        }
    }
    report
}

/// Clause 3 for one `declare`: resolved to exactly one define, and equal to it.
fn check_declare(
    defines: &BTreeMap<String, Vec<(String, String)>>,
    file: String,
    line_num: usize,
    header: parse::DirectMtHeader,
) -> Option<DirectMtFinding> {
    let candidates = defines.get(&header.symbol).map_or(&[][..], Vec::as_slice);
    let (kind, detail) = match candidates {
        [(_, signature)] if *signature == header.signature => return None,
        [(define_file, signature)] => (
            FindingKind::DeclareMismatch,
            format!(
                "declared `{}`, defined in {define_file} as `{signature}`",
                header.signature
            ),
        ),
        [] => (
            FindingKind::UnresolvedDeclare,
            "no define in the corpus".to_string(),
        ),
        many => (
            FindingKind::UnresolvedDeclare,
            format!("{} defines in the corpus", many.len()),
        ),
    };
    Some(DirectMtFinding {
        kind,
        file,
        site: header.symbol.clone(),
        symbol: header.symbol,
        line_num,
        detail,
    })
}

/// Clauses 1 and 2 over every function body in one file.
fn scan_bodies(file: &str, text: &str, report: &mut Report) {
    let defined_here: BTreeSet<String> = text
        .lines()
        .filter_map(|line| parse_direct_mt_header(line.trim_start()))
        .filter(|header| header.kind == HeaderKind::Define)
        .map(|header| header.symbol)
        .collect();
    let mut current: Option<String> = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("define ") {
            current = Some(parse_define_name(trimmed));
            continue;
        }
        if trimmed == "}" {
            current = None;
            continue;
        }
        let Some(function) = &current else {
            continue;
        };
        let mut references = direct_mt_references(line);
        let mut breaches = Vec::new();
        if let Some(callee) = called_direct_mt(line) {
            report.calls_examined += 1;
            if let Some(at) = references.iter().position(|symbol| *symbol == callee) {
                references.remove(at);
            }
            if !is_internal_call(function, callee, &defined_here) {
                breaches.push((FindingKind::ForeignCall, callee));
            }
        }
        breaches.extend(
            references
                .into_iter()
                .map(|symbol| (FindingKind::AddressEscape, symbol)),
        );
        for (kind, symbol) in breaches {
            let detail = match kind {
                FindingKind::ForeignCall => "called from outside its own shim/self pair",
                _ => "address used as a value, not as a callee",
            };
            report.findings.push(DirectMtFinding {
                kind,
                file: file.to_string(),
                site: function.clone(),
                symbol: symbol.to_string(),
                line_num: i + 1,
                detail: detail.to_string(),
            });
        }
    }
}

/// The `$direct_mt` symbol in this line's callee position, if any.
fn called_direct_mt(line: &str) -> Option<&str> {
    let callee = callee_of(line)?.strip_prefix('@')?;
    let (symbol, _) = symbol_at(callee)?;
    symbol.ends_with(DIRECT_MT_SUFFIX).then_some(symbol)
}

/// D2: the caller is the callee's own shim or the callee itself, **and** this
/// file is the one that defines the callee — a same-named caller elsewhere is
/// foreign.
#[allow(clippy::similar_names)] // Reason: caller/callee is the domain's own pair
fn is_internal_call(caller: &str, callee: &str, defined_here: &BTreeSet<String>) -> bool {
    let Some(base) = callee.strip_suffix(DIRECT_MT_SUFFIX) else {
        return false;
    };
    let own_pair = caller == callee || caller.strip_suffix(DIRECT_SUFFIX) == Some(base);
    own_pair && defined_here.contains(callee)
}

/// What auditing a directory concluded; [`DirectMtVerdict::exit`] is the only
/// place it becomes a process status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectMtVerdict {
    NotADirectory,
    /// No `.ll` files, or some could not be read: the audit cannot vouch for
    /// what it did not see.
    UnreadableCorpus,
    /// `.ll` files but no `$direct_mt` define — a corpus that proves nothing
    /// must not render as green (D3a).
    NoDirectMt,
    Findings(usize),
    Clean,
}

impl DirectMtVerdict {
    pub(crate) fn classify(report: &Report, unreadable: usize) -> Self {
        if report.files == 0 || unreadable > 0 {
            Self::UnreadableCorpus
        } else if report.defined == 0 {
            Self::NoDirectMt
        } else if report.findings.is_empty() {
            Self::Clean
        } else {
            Self::Findings(report.findings.len())
        }
    }

    /// 0 clean / 1 findings / 2 bad input.
    pub(crate) fn exit(&self) -> ExitCode {
        match self {
            Self::Clean => ExitCode::SUCCESS,
            Self::Findings(_) => ExitCode::FAILURE,
            Self::NotADirectory | Self::UnreadableCorpus | Self::NoDirectMt => ExitCode::from(2),
        }
    }
}

/// Audit a directory of `.ll` files and print the report.
pub fn cmd_check_direct_mt_callers(dir: &Path, json: bool) -> ExitCode {
    audit_directory(dir, json).exit()
}

/// Read, scan, print; the testable half of [`cmd_check_direct_mt_callers`].
pub(crate) fn audit_directory(dir: &Path, json: bool) -> DirectMtVerdict {
    if !dir.is_dir() {
        eprintln!("error: {} is not a directory", dir.display());
        return DirectMtVerdict::NotADirectory;
    }
    let output = audit_output(dir, json);
    for line in &output.stdout {
        println!("{line}");
    }
    for line in &output.stderr {
        eprintln!("{line}");
    }
    output.verdict
}

/// What one run decided and printed, per stream — so a test can parse exactly
/// what reaches stdout.
pub(crate) struct AuditOutput {
    pub(crate) verdict: DirectMtVerdict,
    pub(crate) stdout: Vec<String>,
    pub(crate) stderr: Vec<String>,
}

/// Scan a directory and render its report. The `note: skipped` line follows
/// the `summary:` line in text mode; in `--json` mode it goes to stderr, so
/// stdout stays one parseable document (ADR 28.10.26e D2, as `small-adts`
/// already does for its drift lines).
pub(crate) fn audit_output(dir: &Path, json: bool) -> AuditOutput {
    let mut files = Vec::new();
    let corpus = scan_ll_corpus(dir, |path, text| {
        files.push((path.to_path_buf(), text.to_string()));
    });
    let report = scan(&files);
    let verdict = DirectMtVerdict::classify(&report, corpus.unreadable.len());
    if json {
        return AuditOutput {
            stdout: vec![render::render_json(&report, &verdict)],
            stderr: corpus.skip_report(),
            verdict,
        };
    }
    let mut stdout = render::render_text(&report, &verdict, dir, corpus.unreadable.len());
    // `render_text` opens with the summary line; the note goes directly after.
    stdout.splice(1..1, corpus.skip_report());
    AuditOutput {
        verdict,
        stdout,
        stderr: Vec::new(),
    }
}
