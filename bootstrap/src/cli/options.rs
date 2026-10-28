//! Global CLI options: the top-level `Cli` parser and the value enums its
//! flags take.
//!
//! Split from `cli::mod` (which owns the `Commands` subcommand tree) on the
//! seam between *how a run is configured* and *what it does* — the two grow
//! independently, and `mod.rs` was one line from the file-size cap.

use clap::Parser;
use std::path::PathBuf;

use super::Commands;
use albite_bootstrap::driver::diagnostics::ErrorFormat;

/// Controls when ANSI color codes are emitted in test output.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ColorMode {
    /// Color when stdout is a TTY (default)
    Auto,
    /// Always emit color codes
    Always,
    /// Never emit color codes
    Never,
}

/// `check --error-format`'s values (ADR 28.9.26e D1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ErrorFormatArg {
    /// ariadne reports on stderr (default)
    Human,
    /// Schema-1 JSONL records on stdout
    Json,
}

/// `test --format`'s values (ADR 29.9.26i D1). It formats the whole stdout
/// stream — results, summary and diagnostics — so it is libtest's `--format`,
/// not `check`'s `--error-format`. `junit` (ADR 30.9.26k) renders the same
/// per-test records as one `JUnit` XML document for CI dashboards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum TestFormatArg {
    /// Per-test lines and a summary line (default)
    #[default]
    Human,
    /// Schema-1 JSONL records on stdout: diagnostics, one per test, a summary
    Json,
    /// One `JUnit` XML document on stdout; diagnostics stay on stderr
    Junit,
}

impl ErrorFormatArg {
    /// The library-side format, with `--json` as an alias for `json` (D2).
    /// clap already refuses `--json` beside an explicit `--error-format`.
    pub fn resolve(self, json_alias: bool) -> ErrorFormat {
        match (self, json_alias) {
            (_, true) | (ErrorFormatArg::Json, false) => ErrorFormat::Json,
            (ErrorFormatArg::Human, false) => ErrorFormat::Human,
        }
    }
}

// `name` and `bin_name` are fixed rather than taken from `argv[0]`, so usage
// lines say `albite` under the deprecated `tungsten` shim too (ADR 20.10.26d D1).
#[derive(Parser)]
#[command(name = "albite", bin_name = "albite")]
#[command(author, version, about = "The Albite proof language compiler")]
#[command(
    long_about = "Albite (formerly Tungsten by Spinning Cat Studios) is a proof language that \
                  combines programming and theorem proving.\n\
                  Former repository: https://github.com/Spinning-Cat-Studios/tungsten\n\n\
                  This is the bootstrap compiler, written in Rust. Once Albite is self-hosting,\n\
                  it will be replaced by a compiler written in Albite itself."
)]
#[command(
    after_help = "Core commands: check, run, test, compile, eval, repl, clean, cache\n\
                  Diagnostics:   info, explain, doctor, diff, commands\n\
                  Experience:    sidecar\n\n\
                  Run `albite <command> --help` for details on a specific command.\n\
                  Run `albite commands` for a flat listing of all commands.\n\
                  Run `albite info pipeline` for diagnostic flags and inspection tools."
)]
#[allow(clippy::struct_excessive_bools)] // Reason: clap global flags are independent bools (ADR 1.10.26j D5)
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Run a file directly (shorthand for `albite run <FILE>`)
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// Show verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Maximum number of errors to display (0 = no limit)
    #[arg(long, global = true, default_value = "20")]
    pub max_errors: usize,

    /// Dump elaborated type annotations to stderr (diagnostic)
    #[arg(long, global = true)]
    pub dump_types: bool,

    /// Always show diagnostic hints in error output (even in non-TTY contexts)
    #[arg(long, global = true)]
    pub hints: bool,

    /// Suppress diagnostic hints in error output
    #[arg(long, global = true)]
    pub no_hints: bool,

    /// Termination-checking enforcement: `all` | `proofs` | `report`
    /// (ADR 29.6.26e). Overrides `ALBITE_TERMINATION`. Global because the
    /// admission gate runs on `check`, `run`, `test` and `compile` alike.
    #[arg(long, global = true, value_name = "LEVEL")]
    pub termination: Option<String>,
}
