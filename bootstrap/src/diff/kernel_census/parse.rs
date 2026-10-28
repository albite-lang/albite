//! One saved `kernel-census --all-rows` report, parsed and proven complete
//! (ADR 4.10.26a D2).
//!
//! The grammar is the census's own (`check_kernel_census::render`): a header,
//! three count lines sharing the rows' two-space indent, a cause table
//! indented four spaces, then one row per definition. Anything the parser
//! cannot account for is a [`ReportError`], never a skipped line, because a
//! join over a partial report would print a plausible but false matrix.

use std::collections::BTreeMap;
use std::fmt;

use albite_bootstrap::elaborate::kernel::Cause;

/// One definition's census outcome as the report printed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportRow {
    /// The proof-mode outcome.
    pub primary: Cause,
    /// Executable mode's outcome, when the census computed one.
    pub secondary: Option<Cause>,
}

/// A whole report: every row, keyed by definition name.
pub type CensusReport = BTreeMap<String, ReportRow>;

/// Why a report was refused. Every arm names enough to find the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// No `kernel census: <file>` line.
    MissingHeader,
    /// A count line (`population`, `admitted`, `refused`) is absent.
    MissingCountLine(&'static str),
    /// A header or count line appears twice — two reports in one file.
    RepeatedLine { line: usize, keyword: &'static str },
    /// A count line whose number does not parse.
    MalformedCountLine { line: usize, text: String },
    /// A two-space-indented line that is neither a count line nor a row.
    MalformedRow { line: usize, text: String },
    /// A definition name that appears twice.
    DuplicateName(String),
    /// The header and counts are present but no row is.
    NoRows,
    /// Fewer or more rows than `admitted + refused + axioms` — a `-v` or a
    /// truncated report.
    RowCountMismatch { expected: usize, found: usize },
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReportError::MissingHeader => write!(f, "no `kernel census: <file>` header"),
            ReportError::MissingCountLine(keyword) => write!(f, "no `{keyword}:` count line"),
            ReportError::RepeatedLine { line, keyword } => {
                write!(
                    f,
                    "line {line}: a second `{keyword}` line (one report per file)"
                )
            }
            ReportError::MalformedCountLine { line, text } => {
                write!(f, "line {line}: count line does not parse: {text}")
            }
            ReportError::MalformedRow { line, text } => {
                write!(f, "line {line}: not a census row: {text}")
            }
            ReportError::DuplicateName(name) => write!(f, "definition `{name}` appears twice"),
            ReportError::NoRows => write!(f, "no rows — a census of nothing cannot be compared"),
            ReportError::RowCountMismatch { expected, found } => write!(
                f,
                "{found} rows, but the counts say {expected} — \
                 save the report with --all-rows, and in full"
            ),
        }
    }
}

/// The three counts the completeness check reads.
#[derive(Default)]
struct Counts {
    header: bool,
    axioms: Option<usize>,
    admitted: Option<usize>,
    refused: Option<usize>,
}

/// Parse a saved `--all-rows` report, refusing anything incomplete (D2).
///
/// # Errors
///
/// The first [`ReportError`] met, in line order; the completeness checks run
/// after every line has parsed.
pub fn parse_census_report(text: &str) -> Result<CensusReport, ReportError> {
    let mut counts = Counts::default();
    let mut rows = CensusReport::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.starts_with("kernel census: ") {
            if counts.header {
                return Err(ReportError::RepeatedLine {
                    line: number,
                    keyword: "kernel census",
                });
            }
            counts.header = true;
            continue;
        }
        let Some(body) = two_space_body(line) else {
            continue;
        };
        if read_count_line(body, number, &mut counts)? {
            continue;
        }
        let (name, row) = parse_row(body).ok_or_else(|| ReportError::MalformedRow {
            line: number,
            text: line.to_string(),
        })?;
        if rows.insert(name.to_string(), row).is_some() {
            return Err(ReportError::DuplicateName(name.to_string()));
        }
    }
    check_complete(&counts, rows.len())?;
    Ok(rows)
}

/// The text after exactly two spaces of indent; `None` for any other line
/// (unindented, or the cause table's four spaces).
fn two_space_body(line: &str) -> Option<&str> {
    let body = line.strip_prefix("  ")?;
    (!body.is_empty() && !body.starts_with(' ')).then_some(body)
}

/// Record `body` if it is a count line, by keyword, before any row parse.
/// `Ok(false)` means it is not one.
fn read_count_line(body: &str, line: usize, counts: &mut Counts) -> Result<bool, ReportError> {
    let (keyword, slot, parsed_count) = if let Some(rest) = body.strip_prefix("population: ") {
        ("population", &mut counts.axioms, axioms_of(rest))
    } else if let Some(rest) = body.strip_prefix("admitted: ") {
        ("admitted", &mut counts.admitted, first_number(rest))
    } else if let Some(rest) = body.strip_prefix("refused: ") {
        ("refused", &mut counts.refused, first_number(rest))
    } else {
        return Ok(false);
    };
    if slot.is_some() {
        return Err(ReportError::RepeatedLine { line, keyword });
    }
    let count = parsed_count.ok_or_else(|| ReportError::MalformedCountLine {
        line,
        text: body.to_string(),
    })?;
    *slot = Some(count);
    Ok(true)
}

/// The first whitespace-separated token, as a number.
fn first_number(rest: &str) -> Option<usize> {
    rest.split_whitespace().next()?.parse().ok()
}

/// The population line's axiom count: the number before `axioms not checked`.
fn axioms_of(rest: &str) -> Option<usize> {
    let (before, _) = rest.split_once(" axioms not checked")?;
    before.rsplit(' ').next()?.parse().ok()
}

/// `<name>: <primary> / <secondary>[ — <message>]`, where `<secondary>` is a
/// cause label or `—`. The message is not kept: the join reads causes only.
fn parse_row(body: &str) -> Option<(&str, ReportRow)> {
    let (name, outcome) = body.split_once(": ")?;
    let (primary, rest) = outcome.split_once(" / ")?;
    let secondary = rest.split_once(" — ").map_or(rest, |(label, _)| label);
    let secondary = match secondary {
        "—" => None,
        label => Some(cause_of(label)?),
    };
    let row = ReportRow {
        primary: cause_of(primary)?,
        secondary,
    };
    (!name.is_empty()).then_some((name, row))
}

/// The cause whose `Cause::label` is `label`.
#[must_use]
pub fn cause_of(label: &str) -> Option<Cause> {
    Cause::ALL.into_iter().find(|cause| cause.label() == label)
}

/// D2's completeness: the header, all three counts, exactly `admitted +
/// refused + axioms` rows, and at least one.
fn check_complete(counts: &Counts, found: usize) -> Result<(), ReportError> {
    if !counts.header {
        return Err(ReportError::MissingHeader);
    }
    let axioms = counts
        .axioms
        .ok_or(ReportError::MissingCountLine("population"))?;
    let admitted = counts
        .admitted
        .ok_or(ReportError::MissingCountLine("admitted"))?;
    let refused = counts
        .refused
        .ok_or(ReportError::MissingCountLine("refused"))?;
    // The mismatch first: a `-v` report of a census with nothing refused has
    // no rows either, and "save it with --all-rows" is the useful answer.
    let expected = admitted + refused + axioms;
    if found != expected {
        return Err(ReportError::RowCountMismatch { expected, found });
    }
    if found == 0 {
        return Err(ReportError::NoRows);
    }
    Ok(())
}
