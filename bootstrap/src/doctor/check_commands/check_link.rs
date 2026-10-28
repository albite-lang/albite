//! `albite doctor check link` — link-level health checks (ADR 13.8.26c D5).
//!
//! Groups the two checks about *linking* — duplicate object-file symbols and
//! the properties of a produced binary — under one namespace. The grouping is
//! not cosmetic: `doctor check` was at 15 of 15 subcommands, and `cli-surface`
//! fires above the cap, so a new sibling needed a slot. Sub-namespacing is the
//! remedy `cli-surface`'s own message asks for; dropping a command is the move
//! it warns against.
//!
//! `doctor check link-collisions` and `doctor check link-health` remain as
//! hidden aliases in [`super::CheckCommands`], so no caller and no `make`
//! recipe breaks.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;

use crate::doctor::checks;

/// Link-level health check subcommands (ADR 13.8.26c D5).
///
/// Grouped under `albite doctor check link <subcommand>`.
#[derive(Subcommand)]
pub enum CheckLinkCommands {
    /// Check for symbol collisions across object files (ADR 6.5.26d §2.5)
    ///
    /// Runs `nm -g` on object files in a directory and reports duplicate
    /// defined text symbols that would cause linker errors.
    ///
    /// See also: `albite doctor check module name-collisions` (the same duplicate,
    /// one phase earlier and without needing a build).
    ///
    /// Examples:
    ///   albite doctor check link collisions /tmp/albite_codegen/
    #[cfg(feature = "codegen")]
    Collisions {
        /// Directory containing .o files to check
        dir: PathBuf,
    },

    /// Report `extern "C"` declarations no albite_core symbol provides (ADR 18.8.26b retrospective)
    ///
    /// An `extern "C"` DECLARATION type-checks on every target — the symbol it
    /// names is not looked for until something links. So a `.tg` binding with
    /// no Rust side gives a clean `check`, a clean `cargo test`, a clean
    /// `check-health`, and then an `undefined reference` minutes into a
    /// self-compile. This walks a module tree's declarations and a source root's
    /// `#[no_mangle] extern "C"` exports and reports the ones that resolve to
    /// nothing. Cost 2 (two source walks). Exit 2 on findings. A declaration of a
    /// code-generator intrinsic (`tg_bytes_*`, ADR 3.10.26n) is lowered inline
    /// and never linked, so it is listed on its own `ℹ` line, not as a finding.
    ///
    /// See also: `albite doctor check extern-coverage` — the same declarations,
    /// asked whether the EVALUATOR can execute them (a `run`/`test` question)
    /// rather than whether the linker can find them. The answers are
    /// independent: most of `main.tg`'s externs are unexecutable AND perfectly
    /// linkable.
    ///
    /// Examples:
    ///   albite doctor check link extern-symbols src/compiler/main.tg
    ///   albite doctor check link extern-symbols src/compiler/main.tg -v
    ExternSymbols {
        /// The root source file whose module tree is scanned for declarations
        file: PathBuf,

        /// Source root scanned for `#[no_mangle] extern "C"` exports
        #[arg(long, default_value = "albite_core/src")]
        core_root: PathBuf,
    },

    /// Report C names the built artefacts import that a program could capture (ADR 9.10.26f)
    ///
    /// A program's definition named like a C symbol the runtime imports is
    /// spelled apart by the elaborator, so the link cannot join it to the
    /// runtime's call — but only for names in `albite_core::link_names`.
    /// This runs `nm` over each file (an archive, an object or a linked
    /// binary), takes the names they import and none of them defines, and
    /// reports each one the table and its prefix rules do not reserve, naming
    /// the file. Cost 1 (no elaboration). Exit 2 on findings; exit 1 when the
    /// inputs import nothing, which checked nothing.
    ///
    /// See also: `make check-reserved-link-names` (builds the runtime archive
    /// and a probe program, then runs this over both, under both native-core
    /// settings); `albite doctor check link extern-symbols` (the opposite
    /// direction — a `.tg` declaration no runtime symbol provides).
    ///
    /// Examples:
    ///   albite doctor check link reserved-names target/release/libalbite_core.a
    ///   albite doctor check link reserved-names build/probe.o build/probe
    ReservedNames {
        /// Archives, objects or linked binaries to read with `nm`
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },

    /// Verify compiled binary link health (ADR 19.5.26d)
    ///
    /// Checks that a compiled Tungsten binary has the expected properties:
    /// a deep stack for the program body (macOS: the linked stack size; Linux:
    /// the `__albite_run_on_stack` runtime entry, ADR 19.10.26c) and
    /// executability. Cost 1 (no elaboration).
    ///
    /// Examples:
    ///   albite doctor check link health ./albite1
    ///   albite doctor check link health ./albite1 -v
    Health {
        /// Path to the compiled binary to check
        binary: PathBuf,
    },
}

/// Dispatch a link-level health check subcommand.
pub fn dispatch_check_link(cmd: CheckLinkCommands, verbose: bool) -> ExitCode {
    match cmd {
        #[cfg(feature = "codegen")]
        CheckLinkCommands::Collisions { dir } => {
            checks::check_link_collisions::cmd_check_link_collisions(&dir)
        }
        CheckLinkCommands::ExternSymbols { file, core_root } => {
            checks::check_extern_symbols::cmd_check_extern_symbols(
                &file,
                Path::new(&core_root),
                verbose,
            )
        }
        CheckLinkCommands::ReservedNames { files } => {
            checks::check_reserved_link_names::cmd_check_reserved_link_names(&files)
        }
        CheckLinkCommands::Health { binary } => {
            checks::check_link_health::cmd_check_link_health(&binary, verbose)
        }
    }
}
