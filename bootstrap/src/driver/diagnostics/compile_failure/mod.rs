//! `albite compile`'s structured failure (ADR 30.9.26j).
//!
//! Every back-end failure on `compile`'s path — a flag conflict, a codegen
//! error, a tool that would not run or exited non-zero, an unusable runtime
//! archive, an output that could not be written — is one [`CompileFailure`]
//! value, rendered in one place ([`report`]) as today's human text or as one
//! schema-1 `diagnostic` record.
//!
//! It lives here, in the LLVM-free library crate, rather than under
//! `compile/` (D1): that module is `#[cfg(feature = "codegen")]`, so a test
//! there needs LLVM, and the coverage and mutation gates cannot score it. The
//! `compile/` side only builds values and calls [`report`]; a codegen error
//! reaches the value as its `Display` string.
//!
//! The human text is **byte-identical** to what each site printed before this
//! module existed (D3): the `tests/golden/compile_error/` goldens were
//! snapshotted from the pre-change binary and are the oracle.

use std::io::Write;

use super::json::{self, DiagnosticRecord, Level, NoteRecord};
use super::{record_stream, write_record, ErrorFormat};
use crate::driver::PipelineError;

/// What went wrong in a back-end failure, one kind per catalogued code (D2, D7).
///
/// Fieldless on purpose: the code is the kind's, and everything a site knows
/// beyond the kind — its text, its notes, its exit code — is the
/// [`CompileFailure::Build`] payload's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildErrorKind {
    /// `--no-codegen` with `--emit-llvm`.
    IncompatibleFlags,
    /// `--only-unit` without `--emit-llvm`.
    MissingRequiredFlag,
    /// Declaring or compiling definitions, the main wrapper, per-module
    /// codegen, object emission.
    CodeGenerationFailed,
    /// `cc`, `llc` or `clang` could not be spawned.
    ToolUnavailable,
    /// The linker, `llc` or `clang -fsanitize` exited non-zero.
    ToolFailed,
    /// `libalbite_core.a` is missing or built for the other platform.
    RuntimeLibraryUnusable,
    /// An output, a temporary directory or a copied `.ll` could not be written.
    OutputNotWritable,
}

impl BuildErrorKind {
    /// The catalogued code. One `BuildErrorKind::X => "E014n",` arm per line:
    /// `explain`'s completeness oracle reads this match as text (D8).
    pub fn code(self) -> &'static str {
        match self {
            BuildErrorKind::IncompatibleFlags => "E0140",
            BuildErrorKind::MissingRequiredFlag => "E0141",
            BuildErrorKind::CodeGenerationFailed => "E0142",
            BuildErrorKind::ToolUnavailable => "E0143",
            BuildErrorKind::ToolFailed => "E0144",
            BuildErrorKind::RuntimeLibraryUnusable => "E0145",
            BuildErrorKind::OutputNotWritable => "E0146",
        }
    }
}

/// A failed `compile`, as a value (D2).
#[derive(Debug)]
pub enum CompileFailure {
    /// A back-end failure. The payload is per *site*, not per kind: the same
    /// kind can carry different text and, for `OutputNotWritable`, a different
    /// exit code (D3).
    Build {
        kind: BuildErrorKind,
        /// The `error: ` line's text, without the prefix.
        message: String,
        /// Continuation lines, printed indented under the `error: ` line.
        notes: Vec<String>,
        /// `help: ` lines, without the prefix.
        help: Vec<String>,
        /// A failed child tool's captured stderr then stdout lines — JSON mode
        /// only, so never part of the human text (D6).
        tool_output: Vec<String>,
        /// The process exit code this site has always returned.
        exit: u8,
    },
    /// The front end failed. Its diagnostics are already rendered, except for
    /// the pipeline's own I/O and module-structure failures (D5).
    Frontend(PipelineError),
}

/// The indent of a continuation line — the width of `error: `.
const CONTINUATION_INDENT: &str = "       ";

impl CompileFailure {
    /// A back-end failure with no notes or help, exiting 1.
    pub fn build(kind: BuildErrorKind, message: impl Into<String>) -> Self {
        CompileFailure::Build {
            kind,
            message: message.into(),
            notes: Vec::new(),
            help: Vec::new(),
            tool_output: Vec::new(),
            exit: 1,
        }
    }

    /// `--no-codegen` with `--emit-llvm` (E0140).
    pub fn incompatible_no_codegen_and_emit_llvm() -> Self {
        CompileFailure::build(
            BuildErrorKind::IncompatibleFlags,
            "--no-codegen and --emit-llvm are incompatible.",
        )
        .with_note("--no-codegen stops before LLVM IR emission.")
        .with_note("Use --no-codegen alone, or --emit-llvm alone.")
    }

    /// `--only-unit` without `--emit-llvm` (E0141, ADR 3.7.26b).
    pub fn only_unit_without_emit_llvm() -> Self {
        CompileFailure::build(
            BuildErrorKind::MissingRequiredFlag,
            "--only-unit requires --emit-llvm.",
        )
        .with_note("Unit isolation emits .ll files for the selected unit(s) only;")
        .with_note("a partial object set cannot be linked into a binary.")
    }

    /// A back-end failure from text another module pre-formatted as an
    /// `error: ` line followed by `help: ` lines — the static-library
    /// diagnoses (D3). Each line keeps every byte after its prefix, trailing
    /// spaces included, so [`Self::human_lines`] reprints it verbatim.
    pub fn preformatted(kind: BuildErrorKind, text: &str) -> Self {
        let mut lines = text.split('\n');
        let first = lines.next().unwrap_or_default();
        let message = first.strip_prefix("error: ").unwrap_or(first);
        let mut failure = CompileFailure::build(kind, message);
        for line in lines {
            match line.strip_prefix("help: ") {
                Some(help) => failure = failure.with_help(help),
                None => {
                    let note = line.strip_prefix(CONTINUATION_INDENT).unwrap_or(line);
                    failure = failure.with_note(note);
                }
            }
        }
        failure
    }

    /// Add a continuation line.
    #[must_use]
    pub fn with_note(mut self, line: impl Into<String>) -> Self {
        if let CompileFailure::Build { notes, .. } = &mut self {
            notes.push(line.into());
        }
        self
    }

    /// Add a `help: ` line.
    #[must_use]
    pub fn with_help(mut self, line: impl Into<String>) -> Self {
        if let CompileFailure::Build { help, .. } = &mut self {
            help.push(line.into());
        }
        self
    }

    /// Override the exit code (only `OutputNotWritable` at the `.ll` write
    /// returns anything but 1).
    #[must_use]
    pub fn with_exit(mut self, code: u8) -> Self {
        if let CompileFailure::Build { exit, .. } = &mut self {
            *exit = code;
        }
        self
    }

    /// Append a failed child tool's captured output: its stderr lines, then
    /// its stdout lines (D6).
    #[must_use]
    pub fn with_tool_output(mut self, stderr: &str, stdout: &str) -> Self {
        if let CompileFailure::Build { tool_output, .. } = &mut self {
            tool_output.extend(stderr.lines().chain(stdout.lines()).map(String::from));
        }
        self
    }

    /// The catalogued code, or `None` for a front-end failure.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            CompileFailure::Build { kind, .. } => Some(kind.code()),
            CompileFailure::Frontend(_) => None,
        }
    }

    /// The exit code the process returns.
    pub fn exit_code(&self) -> u8 {
        match self {
            CompileFailure::Build { exit, .. } => *exit,
            CompileFailure::Frontend(_) => 1,
        }
    }

    /// The human text, one `eprintln!` per element, exactly as the site
    /// printed it before this value existed (D3).
    pub fn human_lines(&self) -> Vec<String> {
        match self {
            CompileFailure::Build {
                message,
                notes,
                help,
                ..
            } => std::iter::once(format!("error: {message}"))
                .chain(notes.iter().map(|n| format!("{CONTINUATION_INDENT}{n}")))
                .chain(help.iter().map(|h| format!("help: {h}")))
                .collect(),
            CompileFailure::Frontend(error) => vec![format!("error: {error}")],
        }
    }

    /// The JSON record, or `None` when the failure restates records the front
    /// end already wrote: `compilation failed: elaboration errors` and
    /// `… parse errors` (D5).
    pub fn record(&self) -> Option<DiagnosticRecord> {
        match self {
            CompileFailure::Build {
                kind,
                message,
                notes,
                help,
                tool_output,
                ..
            } => Some(DiagnosticRecord {
                code: Some(kind.code().to_string()),
                notes: notes.iter().chain(tool_output).map(unlocated).collect(),
                help: help.clone(),
                ..json::failure_record(message)
            }),
            CompileFailure::Frontend(PipelineError::ElabFailed(_)) => None,
            CompileFailure::Frontend(error) => Some(json::failure_record(&error.to_string())),
        }
    }

    /// What [`report`] writes for this failure in `format`.
    pub fn output(&self, format: ErrorFormat) -> FailureOutput {
        match format {
            ErrorFormat::Human => FailureOutput::Lines(self.human_lines()),
            ErrorFormat::Json => self
                .record()
                .map_or(FailureOutput::Silent, FailureOutput::Record),
        }
    }
}

/// One failure's rendering, as a value the tests can compare.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // Reason: one short-lived value per failure; boxing touches every construction site (ADR 1.10.26j D5)
pub enum FailureOutput {
    /// Human lines for stderr.
    Lines(Vec<String>),
    /// One record for the record stream.
    Record(DiagnosticRecord),
    /// Nothing — the front end's records already said it.
    Silent,
}

/// The two streams a report writes to. The caller passes the process's own;
/// a test passes buffers, so the bytes each stream receives are assertable.
pub struct Sinks<'w> {
    pub stdout: &'w mut dyn Write,
    pub stderr: &'w mut dyn Write,
}

impl FailureOutput {
    /// Human lines go to stderr; a record goes to the selected record stream.
    fn write(self, sinks: &mut Sinks<'_>) {
        match self {
            FailureOutput::Lines(lines) => {
                for line in lines {
                    // A closed pipe is the consumer's choice; no one is left to tell.
                    let _ = writeln!(sinks.stderr, "{line}");
                }
            }
            FailureOutput::Record(record) => {
                write_record(&record, record_stream(), sinks.stdout, sinks.stderr);
            }
            FailureOutput::Silent => {}
        }
    }
}

/// Render `failure` in `format` and return its exit code — the one place a
/// `compile` failure becomes output.
pub fn report(failure: &CompileFailure, format: ErrorFormat, sinks: &mut Sinks<'_>) -> u8 {
    failure.output(format).write(sinks);
    failure.exit_code()
}

/// A `compile` warning on the default path (D9): `warning: <message>` in human
/// mode, an uncatalogued `warning` record in JSON mode.
pub fn warning_output(message: &str, format: ErrorFormat) -> FailureOutput {
    match format {
        ErrorFormat::Human => FailureOutput::Lines(vec![format!("warning: {message}")]),
        ErrorFormat::Json => FailureOutput::Record(DiagnosticRecord {
            level: Level::Warning,
            ..json::failure_record(message)
        }),
    }
}

/// Write [`warning_output`].
pub fn report_warning(message: &str, format: ErrorFormat, sinks: &mut Sinks<'_>) {
    warning_output(message, format).write(sinks);
}

/// A non-fatal note (ADR 20.10.26i D2): `note: <message>` in human mode, an
/// uncatalogued `note` record in JSON mode.
pub fn note_output(message: &str, format: ErrorFormat) -> FailureOutput {
    match format {
        ErrorFormat::Human => FailureOutput::Lines(vec![format!("note: {message}")]),
        ErrorFormat::Json => FailureOutput::Record(DiagnosticRecord {
            level: Level::Note,
            ..json::failure_record(message)
        }),
    }
}

/// Write [`note_output`].
pub fn report_note(message: &str, format: ErrorFormat, sinks: &mut Sinks<'_>) {
    note_output(message, format).write(sinks);
}

/// Whether a child tool's output is captured rather than inherited (D6): in
/// JSON mode its lines would be non-record noise on the contract stream.
pub fn capture_child_output(format: ErrorFormat) -> bool {
    format == ErrorFormat::Json
}

fn unlocated(message: &String) -> NoteRecord {
    NoteRecord {
        message: message.clone(),
        span: None,
    }
}

#[cfg(test)]
mod tests;
