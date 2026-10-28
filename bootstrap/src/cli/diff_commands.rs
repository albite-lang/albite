//! Diff CLI subcommand definitions — split from `subcommands.rs` when the
//! `kernel-census` variant took that file over its size threshold (ADR
//! 4.10.26a).

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum DiffCommands {
    /// Compare two LLVM IR files structurally (type defs + function signatures)
    Ir {
        /// Baseline IR file
        file_a: PathBuf,

        /// Candidate IR file
        file_b: PathBuf,

        /// Only compare type definitions
        #[arg(long)]
        types_only: bool,

        /// Only compare function signatures
        #[arg(long)]
        signatures_only: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Compare two Core IR dump files structurally (from --dump-ir output)
    Core {
        /// Baseline Core IR dump file
        file_a: PathBuf,

        /// Candidate Core IR dump file
        file_b: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Structural tree-diff of two type encodings
    ///
    /// Compares the cached encodings of two named types and shows
    /// an inline diff with +/- markers at divergence points.
    /// Returns exit code 0 if identical, nonzero if different.
    ///
    /// Examples:
    ///   albite diff types `TypeA` `TypeB` examples/list.tg
    ///   albite diff types Expr `TypeExpr` src/compiler/main.tg
    Types {
        /// First type name
        type_a: String,

        /// Second type name
        type_b: String,

        /// The source file containing both types
        file: PathBuf,
    },

    /// Compare ABI layout between bootstrap codegen and .tg emitter (ADR 13.5.26k)
    ///
    /// Elaborates the file, computes the bootstrap codegen layout for the
    /// named type, and compares it with the self-hosted emitter's ABI manifest.
    ///
    /// Examples:
    ///   albite diff abi Nat examples/hello.tg
    ///   albite diff abi Option src/compiler/main.tg
    #[cfg(feature = "codegen")]
    #[command(
        after_help = "See also: `albite info type adt` for ADT details, `albite info codegen abi` for IR-level ABI."
    )]
    Abi {
        /// Type name to compare (e.g., "Nat", "Option", "List")
        type_name: String,

        /// The source file to elaborate
        file: PathBuf,
    },

    /// Run a program on the evaluator AND natively, compare outputs (ADR 3.7.26d)
    ///
    /// Compiles the program natively, runs it, runs the same program under
    /// the bootstrap evaluator, and compares stdout (byte-for-byte after
    /// trailing-newline normalization; stderr is reported but not compared).
    /// This is the general detector for silent value miscompiles: binaries
    /// that compile cleanly, pass the LLVM verifier, and print garbage.
    ///
    /// Eligibility (v1): deterministic programs only — empty stdin, no argv,
    /// environment not passed to the native binary beyond PATH/TMPDIR.
    /// Programs reading time, randomness, filesystem state, or the
    /// environment can diverge without a compiler bug.
    ///
    /// Exit codes: 0 parity; 1 output divergence; 2 native runtime error
    /// with evaluator Ok; 3 compile error (neither side ran); 4 evaluator
    /// error with native Ok; 5 timeout on either side.
    ///
    /// Cost 5 (compile + two executions).
    ///
    /// See also: `albite doctor check ir sret-stores` (static sret-shape
    /// lint), `albite diff bootstrap-selfhost-check`.
    ///
    /// Examples:
    ///   albite diff exec `tests/dead_arm_letelse_run.tg`
    ///   albite diff exec examples/hello.tg --timeout 10
    #[cfg(feature = "codegen")]
    Exec {
        /// The source file to run on both sides
        file: PathBuf,

        /// Per-side timeout in seconds (compile, native run, evaluator run)
        #[arg(long, default_value_t = 60)]
        timeout: u64,
    },

    /// Cold-vs-warm cache parity check — the cache-poisoning canary (ADR 4.7.26d)
    ///
    /// Runs the program cold (a fresh, isolated cache dir) then warm (a second
    /// run reusing that dir) and compares exit status + stdout. The 4.7.26c bug
    /// was a silent cold-vs-warm divergence (warm reads a bodyless signature
    /// entry → "no tests found" / spurious E0030); this is the automated
    /// detector for that whole class, and the cold-vs-warm analog of
    /// `diff exec`'s native-vs-evaluator check.
    ///
    /// Runs on the evaluator — no LLVM, CI-friendly.
    ///
    /// Exit codes: 0 parity; 1 divergence (cache poisoning); 3 compile /
    /// elaboration error (neither side ran); 5 timeout.
    ///
    /// Cost 5 (two evaluator runs).
    ///
    /// See also: `albite cache inspect` (static per-module tier + hazard),
    /// `albite diff exec`, `albite cache clean`.
    ///
    /// Examples:
    ///   albite diff cache src/compiler/main.tg
    ///   albite diff cache tests/foo.tg --mode test --gate
    #[command(after_help = "See also: `albite cache inspect`, `albite diff exec`.")]
    Cache {
        /// The source file to run cold then warm
        file: PathBuf,

        /// Which evaluator mode to run on both sides (run|test)
        #[arg(long, default_value = "run")]
        mode: String,

        /// Per-side timeout in seconds
        #[arg(long, default_value_t = 60)]
        timeout: u64,

        /// CI gate form: non-zero exit only on a genuine divergence
        #[arg(long)]
        gate: bool,
    },

    /// Compare bootstrap and self-host check results on files or a directory (ADR 20.5.26a, 3.10.26a)
    ///
    /// Runs the bootstrap (this binary) and the self-hosted compiler
    /// (albite1, self-compiled) on the same source in check mode. Helps
    /// identify codegen regressions in the self-hosted compiler.
    ///
    /// One FILE: compares error counts and messages (exit 0 agree, 1 otherwise).
    ///
    /// Several paths, or any DIRECTORY: one row per `.tg` entry (a directory
    /// expands to every `.tg` under it, skipping dot-directories) with each
    /// compiler's verdict — `ok, N` definitions, its first error header, or a
    /// fault — and whether they agree. A directory's entries run with the
    /// directory as the working directory, so a bare entry finds its siblings:
    /// to check workspace entries, pass the directory, not its files. Exit 0
    /// every row agrees, 1 a row diverges, 2 bad input (missing path or
    /// binary, no entries), 3 a row faulted (a timeout, a signal, an
    /// unreadable verdict) — a fault outranks a divergence.
    ///
    /// Cost 3+3 per entry (two elaboration passes).
    ///
    /// Examples:
    ///   albite diff bootstrap-selfhost-check src/compiler/main.tg --selfhost-binary ./albite1
    ///   albite diff bootstrap-selfhost-check tests/conformance/workspace --selfhost-binary ./albite1
    ///
    /// In the container the bootstrap is $CARGO_TARGET_DIR/release/albite,
    /// never ./target/release/albite (the host's build):
    ///   make devcontainer-build
    ///   make devcontainer-self-compile-fast
    ///   devcontainer exec --workspace-folder . bash -c \
    ///     '$CARGO_TARGET_DIR/release/albite diff bootstrap-selfhost-check \
    ///     tests/conformance/workspace --selfhost-binary ./albite1'
    // Verbatim: the container recipe above is meant to be copied, and clap's
    // default reflow would join its lines into one paragraph.
    #[command(name = "bootstrap-selfhost-check", verbatim_doc_comment)]
    #[allow(clippy::doc_markdown)] // Reason: `--help` text, copied verbatim
    BootstrapSelfhostCheck {
        /// Source files or directories to check with both compilers
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Path to the albite1 (self-compiled) binary
        #[arg(long, default_value = "./albite1")]
        selfhost_binary: PathBuf,

        /// Per-run timeout in seconds, for each compiler on each entry
        #[arg(long, default_value_t = 60)]
        timeout: u64,
    },

    /// Compare one definition's Core TERM across both compilers (ADR 19.8.26d retrospective)
    ///
    /// The sibling above compares error counts, `exec` compares printed
    /// values, `cache` compares cold against warm. None of them can see the
    /// two compilers agreeing on the verdict while disagreeing about the
    /// *term* — the state ADR 19.8.26d had to establish by reading the
    /// lowering source and building a fixture pair, because no command asked.
    ///
    /// Both sides render through the same `Display for Term`, so a textual
    /// comparison is a structural one. Cost 3+3 (two elaborations).
    ///
    /// Three verdicts (ADR 4.10.26b). AGREE: byte-identical terms, exit 0.
    /// AGREE UP TO NUMERALS: identical once both sides' numerals are spelled
    /// one way — the bootstrap's `(succ (succ zero))` against the self-host's
    /// `2` — printing both raw terms and the normalised one, exit 0. DIVERGE:
    /// any other difference, exit 1. Exit 2 when the self-host could not be
    /// asked — its diagnostics stubbed out (the production default), no
    /// definition by that name, or a term it could not read back.
    ///
    /// The numeral tier fails closed: a term holding a string literal, or
    /// binding `zero` or `succ` as a name, is never normalised and compares
    /// byte for byte, so `(λzero:Nat. zero)` against `(λzero:Nat. 0)` diverges.
    ///
    /// Agreement on one definition is agreement on one definition. For the
    /// corpus-wide question, see `albite doctor check selfhost closed-terms`.
    ///
    /// On a divergence, read BOTH rendered terms, not the byte offset alone:
    /// mu-binders are still spelled differently (`μList` vs `μα_List`), so the
    /// first textual difference can be cosmetic while the structural one is
    /// further in.
    ///
    /// Examples:
    ///   albite diff selfhost-core main src/compiler/main.tg
    ///   albite diff selfhost-core len examples/list.tg --selfhost-binary ./albite1
    #[command(name = "selfhost-core")]
    SelfhostCore {
        /// The definition to compare
        definition: String,

        /// The source file both compilers elaborate
        file: PathBuf,

        /// Path to the albite1 (self-compiled) binary
        #[arg(long, default_value = "./albite1")]
        selfhost_binary: PathBuf,
    },

    /// Join two saved kernel-census reports by definition name (ADR 4.10.26a)
    ///
    /// Reads two files that `albite doctor check proof kernel-census <file>
    /// --all-rows` wrote — before and after a change — and prints how many
    /// definitions moved from each primary cause to each other cause, then
    /// which ones. Cost 1: it elaborates nothing, so the two runs can come
    /// from different commits.
    ///
    /// A move is a change of PRIMARY cause. A row whose primary held and whose
    /// secondary changed is listed under `secondary only`, never counted as a
    /// move. Names in one report only are listed apart.
    ///
    /// A report it cannot prove complete is refused, exit 2: a malformed row
    /// (by line number), a duplicate name, a missing header or count line, or
    /// a row count other than admitted + refused + axioms — which is what a
    /// `-v` report, printing refused rows only, looks like. Otherwise exit 0,
    /// however much moved: a census comparison, not a gate.
    ///
    /// Examples:
    ///   albite doctor check proof kernel-census src/compiler/main.tg --all-rows > before.txt
    ///   albite doctor check proof kernel-census src/compiler/main.tg --all-rows > after.txt
    ///   albite diff kernel-census before.txt after.txt
    ///   albite diff kernel-census before.txt after.txt --json
    #[command(
        name = "kernel-census",
        verbatim_doc_comment,
        after_help = "See also: `albite doctor check proof kernel-census` (writes the reports)."
    )]
    KernelCensus {
        /// The report saved before the change
        before: PathBuf,

        /// The report saved after the change
        after: PathBuf,

        /// Emit one JSON object instead of the text report
        #[arg(long)]
        json: bool,
    },

    /// Compare two saved Core-size censuses by definition (ADR 16.10.26w)
    ///
    /// Reads two reports that `albite doctor check proof term-size <file>
    /// --json` (or `--json-out`) wrote and prints exactly what `term-size
    /// --baseline` prints: matched rows by growth, new and removed rows,
    /// `largest now:` and the totals. Cost 1: it elaborates nothing, so both
    /// sides can be committed bases, or two binaries' runs over one tree.
    ///
    /// Either side may be a census or a delta report (whose "rows" are its own
    /// run's census). The output's "rows" are the after side's, so `--json`
    /// output is itself a baseline.
    ///
    /// Exit 0 when nothing is flagged, 1 when a row is flagged, 2 on a refusal
    /// — an unreadable path, a report that is not JSON, an error report, a
    /// missing field or population, a duplicate row (each naming its before
    /// or after path), or two different populations. Under `--json` a refusal
    /// prints one {"error": …} object. A usage error (an unknown flag, a
    /// non-numeric value) is clap's exit 2 with its usage text, not that
    /// object.
    ///
    /// Examples:
    ///   albite diff term-size target/core-size/before.json target/core-size/after.json
    ///   albite diff term-size before.json after.json --min-delta 500 --top 10
    ///   albite diff term-size before.json after.json --json > next-baseline.json
    #[command(
        name = "term-size",
        verbatim_doc_comment,
        after_help = "See also: `albite doctor check proof term-size` (writes the reports; `--baseline` compares against the tree)."
    )]
    TermSize {
        /// The report saved before the change
        before: PathBuf,

        /// The report saved after the change
        after: PathBuf,

        /// Flag a row only when after/before nodes reaches this ratio (default 1.5)
        #[arg(long, value_name = "RATIO")]
        max_growth: Option<f64>,

        /// Flag a row only when it grew by at least this many nodes; a new
        /// row when it has this many (default 2000)
        #[arg(long, value_name = "NODES")]
        min_delta: Option<usize>,

        /// Print at most this many matched, new, removed and `largest now:`
        /// rows (default 20); flagged rows and the totals are never cut
        #[arg(long, value_name = "N")]
        top: Option<usize>,

        /// Print one JSON object on stdout instead of the table
        #[arg(long)]
        json: bool,
    },
}
