//! Cache CLI subcommand definitions. The diff ones are in `diff_commands.rs`.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum CacheCommands {
    /// Show cache statistics (AST + elaboration)
    ///
    /// Reports the ROOT it inspected (ADR 5.8.26d D5). With no operand that is
    /// the current directory, which is NOT where a build writes: the writer
    /// resolves the root from the entry file's parent, so a run from the repo
    /// root can report `Elab entries: 0` about a project whose cache is
    /// elsewhere. Pass the entry file to inspect that project's cache.
    Stats {
        /// Entry source file whose project cache to inspect (default: current directory)
        file: Option<PathBuf>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Inspect per-module elaboration-cache tiers + run/test body hazard (ADR 4.7.26d)
    ///
    /// For each module in the project, reports the highest cache tier present
    /// (full-output / signature-only / uncached), its cached def count, and — for
    /// the selected mode — whether that entry would serve `CoreDef` bodies to a
    /// `run`/`test`. A signature-only entry answers "No": the ADR 4.7.26c hazard
    /// that `cache status`'s aggregate counts cannot surface.
    ///
    /// Cost 3 (parse + Signature Collection signature collection; no Body Elaboration / no codegen).
    ///
    /// See also: `albite cache status` (aggregate counts),
    /// `albite diff cache` (cold-vs-warm parity canary), `albite cache clean`.
    ///
    /// Examples:
    ///   albite cache inspect src/compiler/main.tg --mode run
    ///   albite cache inspect examples/hello.tg --json
    #[command(after_help = "See also: `albite diff cache`, `albite cache status`.")]
    Inspect {
        /// The entry source file of the project to inspect
        file: PathBuf,

        /// Which mode's body hazard to report (run|test|check)
        #[arg(long, default_value = "run")]
        mode: String,

        /// Output as JSON (one record per module)
        #[arg(long)]
        json: bool,
    },

    /// Show cache status summary (alias for stats, ADR 10.5.26l)
    ///
    /// Reports the root it inspected; pass an entry file to inspect that
    /// project's cache rather than the current directory's (ADR 5.8.26d D5).
    ///
    /// Examples:
    ///   albite cache status src/compiler/main.tg
    Status {
        /// Entry source file whose project cache to inspect (default: current directory)
        file: Option<PathBuf>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Prune cache to target size (removes least recently used entries)
    ///
    /// Reports the root it pruned; pass an entry file to prune that project's
    /// cache rather than the current directory's (ADR 5.8.26d D5).
    Prune {
        /// Entry source file whose project cache to prune (default: current directory)
        file: Option<PathBuf>,

        /// Target size in MB (defaults to configured `max_size_mb`)
        #[arg(long)]
        target_mb: Option<u64>,
    },

    /// Recursively find and remove ALL .albite (and .tungsten) cache directories (skips target/)
    ///
    /// Scope: every `.albite/` under the current directory. This is the broad
    /// one — the remedy after an AST-variant change or a suspected stale-type
    /// problem, and the one that cannot miss a project because you ran it from
    /// the wrong directory.
    ///
    /// NOT the same as `albite cache clean-project`, despite the shared verb:
    /// that one clears a SINGLE project's cache, resolved from the cwd or an
    /// entry-file operand (ADR 5.8.26d D5). Use `--dry-run` first if the blast
    /// radius matters.
    #[command(
        after_help = "See also: `albite cache clean-project [<file>]` (clears ONE project's cache), `albite cache status <file>`."
    )]
    Clean {
        /// Show what would be removed without actually deleting
        #[arg(long)]
        dry_run: bool,
    },

    /// Clear ONE project's build cache (ADR 19.8.26a; was `albite clean`)
    ///
    /// Scope: a single `.albite/` directory. Reports the root it cleared.
    /// With no operand that root is the current directory, which is NOT where a
    /// build writes — the writer resolves it from the entry file's parent, so a
    /// run from the repo root can clear nothing while the project's cache sits
    /// elsewhere (ADR 5.8.26d D5). Pass the entry file to clear that project's.
    ///
    /// NOT the same as its neighbour `albite cache clean`, which walks the
    /// tree and removes EVERY `.albite/` directory under the cwd. When in
    /// doubt after a compiler change, you almost certainly want that one — this
    /// one cannot reach a project you ran it from the wrong directory for.
    ///
    /// Spelled `clean-project` rather than `clean` because the two now sit
    /// adjacent: flat, the top-level `clean` took its meaning from being
    /// somewhere else; here the name has to carry the scope itself. The old
    /// `albite clean [<file>]` spelling remains as a hidden alias.
    #[command(name = "clean-project")]
    #[command(
        after_help = "See also: `albite cache clean` (removes ALL .albite/ dirs recursively), `albite cache status <file>`."
    )]
    CleanProject {
        /// Entry source file whose project cache to clear (default: current directory)
        file: Option<PathBuf>,
    },
}
