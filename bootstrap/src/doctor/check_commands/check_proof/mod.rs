//! `albite doctor check proof` — checks on proof terms through the kernel's
//! proof judgment (ADR 30.9.26f D2).
//!
//! A sub-namespace of its own because `doctor check type` checks type
//! definitions and encodings, and these check proof *terms* through a
//! different checker. B10's later proof checks land here without a later
//! regrouping.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

use crate::doctor::checks::check_kernel_census::{
    cmd_check_kernel_census, cmd_check_kernel_census_corpus, CensusRows,
};
use crate::doctor::checks::check_proof_term_size::{
    cmd_check_proof_term_size, Population, TermSizeRequest, Thresholds,
};

/// Proof-term health checks.
///
/// Grouped under `albite doctor check proof <subcommand>`.
#[derive(Subcommand)]
pub enum CheckProofCommands {
    /// Run every proof-relevant definition through the kernel's proof judgment and sort each refusal by cause (ADR 30.9.26f)
    ///
    /// Each theorem, lemma and axiom's elaborated Core — and each
    /// non-`#[partial]` `fn`'s whose type holds a proposition anywhere, the
    /// set the build's gate checks (ADR 13.10.26t) — goes to
    /// `albite_core::check_proof` exactly as the elaborator emitted it,
    /// under a context binding only the globals it mentions — `Admitted` when
    /// the termination report says `Total`, `Opaque` otherwise. Axioms are
    /// counted, not checked. Each refusal is the definition's FIRST one,
    /// classified into one of: hole, executable-construct, poison,
    /// global-not-admitted, missing-global-context, marker-ill-formed,
    /// named-type-reference, strict-equality, dependent-subst,
    /// needs-conversion, unexplained-mismatch, unclassified. When a hole,
    /// a `return`/non-structural `fix` or a poisoned type stops the
    /// derivation early, the term is also checked in executable mode and
    /// that outcome is the SECONDARY cause — what the rest of the proof
    /// would need.
    ///
    /// A census, not a gate: nothing any build accepts changes. Exit 0 when
    /// the census completes, whatever it counts. Exit 2 when it could not
    /// ask: the corpus fails to elaborate, or the population is empty — a
    /// census of nothing is refused rather than reported clean. Cost 3.
    ///
    /// Corpus form (ADR 2.10.26o): `--corpus <root>…` census-es every entry
    /// file under the roots — every `.tg` minus `<stem>_test.tg` companions
    /// and minus the files some candidate's `mod` tree reaches — each one
    /// cold, writing nothing, and reports each as census-ed, empty or does
    /// not elaborate. It then compares every term-mode theorem's verdict,
    /// decided by the build's own kernel gate, with the exemption register
    /// (`bootstrap/kernel_exemptions.toml`) and ends `register: matches`,
    /// `register: MISMATCH (…)` or `register: INCOMPLETE (…)`, naming each
    /// row that disagreed. Exit 0 means only that the census ran — whatever
    /// the register line says; exit 2 when a root is missing or unreadable
    /// or holds no entry file.
    ///
    /// `--proof-bearing` (ADR 3.10.26s), corpus-only, elaborates just the
    /// entries needed to cover every file that declares a theorem, lemma,
    /// axiom or proof-carrying `fn` (ADR 14.10.26b), decided from a parse of
    /// each entry's `mod` tree: an entry whose tree adds no uncovered proof
    /// file reads `skipped`, and a register row in a file only skipped
    /// entries reach is not examined (`skipped: no proof item recognised`),
    /// never matched. An entry whose tree does not parse or cannot be built
    /// is always elaborated. A `fn` is proof-carrying when its declared
    /// return type mentions `Eq<…>`, `a == b`, `Void` or a type that reaches
    /// one — a type is followed only through declarations in the same `mod`
    /// tree, so one imported from outside it is read as data; omit the flag
    /// to census such a file.
    ///
    /// See also: `albite doctor check type termination` — the admission
    /// report the context's markers come from. `albite doctor check
    /// sorry-sites` — who planted a hole the census reports. `albite info
    /// def <name> <file>` — the Core term of one refused definition.
    ///
    /// Examples:
    ///   albite doctor check proof kernel-census src/compiler/nat_arith.tg
    ///   albite doctor check proof kernel-census src/compiler/main.tg --include-total
    ///   albite doctor check proof kernel-census examples/proof.tg -v
    ///   albite doctor check proof kernel-census src/compiler/nat_arith.tg --all-rows
    ///   albite doctor check proof kernel-census --corpus tests/golden/check tests/golden/run tests/golden/test tests/golden/compile src/compiler examples
    ///   albite doctor check proof kernel-census --corpus tests/golden/check tests/golden/run tests/golden/test tests/golden/compile src/compiler examples --proof-bearing
    #[command(name = "kernel-census")]
    KernelCensus {
        /// The root source file to elaborate
        #[arg(required_unless_present = "corpus", conflicts_with = "corpus")]
        file: Option<PathBuf>,

        /// Census every entry file under these directories and diff the
        /// result against the kernel exemption register (ADR 2.10.26o)
        #[arg(long, num_args = 1.., value_name = "ROOT")]
        corpus: Vec<PathBuf>,

        /// With `--corpus`: skip every entry whose `mod` tree adds no
        /// uncovered theorem, lemma, axiom or proof-carrying `fn` (ADR
        /// 3.10.26s, 14.10.26b)
        #[arg(long, requires = "corpus", conflicts_with = "file")]
        proof_bearing: bool,

        /// Also check every definition the termination report admits `Total`,
        /// not only the proof-relevant ones
        #[arg(long, conflicts_with = "corpus")]
        include_total: bool,

        /// Print one line per definition, admitted and axiom rows included,
        /// not only the refused ones `-v` prints (ADR 1.10.26m)
        #[arg(long, conflicts_with = "corpus")]
        all_rows: bool,
    },

    /// List definitions beside the size of their Core term, ranked and compared against a saved run (ADR 13.10.26c, 15.10.26g)
    ///
    /// The population is `kernel-census`'s by default: theorems, lemmas and
    /// axioms, plus every definition the termination report admits `Total`
    /// under `--include-total`, and EVERY definition — `#[partial]` and
    /// unclassified included — under `--all-definitions`. Rows from imported
    /// modules are included, each naming its module. Per row: `Subst` nodes,
    /// `Refl` nodes, `Global` OCCURRENCES (a helper cited twice counts twice —
    /// not a distinct dependency count), the total number of `Term` nodes,
    /// and `case-arms`: each `AdtMatch`'s arm count plus 2 per `Case`, nested
    /// ones summed — match width, a definition's exposure to a padding change.
    /// Only the term is walked: the types it carries are not, and nothing is
    /// normalised. An axiom has no body; its row reads `axiom` (`"axiom":
    /// true` with null counts in `--json`). `--top <n>` prints the n largest
    /// rows by nodes; the totals still cover every row.
    ///
    /// The census reads the BOOTSTRAP's Core, elaborated cold: nothing is
    /// read from or written under `.albite/`, and a file that fails to
    /// elaborate prints only its first error — run `albite check` for the
    /// full diagnostics.
    ///
    /// `--baseline <report.json>` joins this run to a saved `--json` report
    /// (a census, or an earlier delta report) by `(module, name)` and ranks
    /// the rows by node growth, then ratio; it lists new and removed rows and
    /// the `largest now:` ranking. A row is flagged when it grew by at least
    /// `--min-delta` nodes AND by at least `--max-growth` times; a new row
    /// when it has at least `--min-delta` nodes. A baseline that is not a
    /// report of the same population, has a row missing a field, or names a
    /// definition twice is refused.
    ///
    /// A census, not a gate. Exit 0 when it completes; without `--baseline`
    /// never 1. With `--baseline`, exit 1 when a row is flagged — the one
    /// exception. Exit 2 when it could not ask: the file does not elaborate,
    /// the population is empty, or the baseline is refused; under `--json`
    /// that exit still prints one `{"error": …}` object. Cost 3.
    ///
    /// See also: `albite doctor check proof kernel-census` — what the
    /// kernel says about the same population. `albite info def <name>
    /// <file>` — the Core term of one definition. `make core-size-diff` —
    /// this delta over a batch or branch window.
    ///
    /// Examples:
    ///   albite doctor check proof term-size src/compiler/nat_arith.tg
    ///   albite doctor check proof term-size src/compiler/nat_arith.tg --json
    ///   albite doctor check proof term-size examples/proof.tg --include-total
    ///   albite doctor check proof term-size src/compiler/main.tg --all-definitions --top 20
    ///   albite doctor check proof term-size src/compiler/main.tg --all-definitions --baseline base.json
    #[command(name = "term-size")]
    TermSize {
        /// The root source file to elaborate
        file: PathBuf,

        /// Also count every definition the termination report admits `Total`,
        /// not only the proof-relevant ones
        #[arg(long, conflicts_with = "all_definitions")]
        include_total: bool,

        /// Count every definition, whatever its termination admission
        /// (ADR 15.10.26g D2)
        #[arg(long)]
        all_definitions: bool,

        /// Print the n largest rows by node count (default with
        /// `--baseline`: 20)
        #[arg(long, value_name = "N")]
        top: Option<usize>,

        /// Compare against a report this command wrote with `--json`
        #[arg(long, value_name = "REPORT_JSON")]
        baseline: Option<PathBuf>,

        /// Flag a row only when after/before nodes reaches this ratio
        #[arg(long, value_name = "RATIO", requires = "baseline")]
        max_growth: Option<f64>,

        /// Flag a row only when it grew by at least this many nodes; a new
        /// row when it has this many
        #[arg(long, value_name = "NODES", requires = "baseline")]
        min_delta: Option<usize>,

        /// Print one JSON object on stdout instead of the table
        #[arg(long)]
        json: bool,

        /// Also write the report's JSON object to this file, beside what
        /// stdout prints; not written when the census refuses
        #[arg(long, value_name = "PATH")]
        json_out: Option<PathBuf>,
    },
}

/// Dispatch `doctor check proof <subcommand>`.
pub fn dispatch_check_proof(command: CheckProofCommands, verbose: bool) -> ExitCode {
    match command {
        CheckProofCommands::KernelCensus {
            file: Some(file),
            include_total,
            all_rows,
            ..
        } => cmd_check_kernel_census(
            &file,
            include_total,
            CensusRows::from_flags(verbose, all_rows),
        ),
        CheckProofCommands::KernelCensus {
            corpus,
            proof_bearing,
            ..
        } => cmd_check_kernel_census_corpus(&corpus, proof_bearing, verbose),
        CheckProofCommands::TermSize {
            file,
            include_total,
            all_definitions,
            top,
            baseline,
            max_growth,
            min_delta,
            json,
            json_out,
        } => {
            let defaults = Thresholds::DEFAULT;
            let request = TermSizeRequest {
                population: Population::from_flags(include_total, all_definitions),
                json,
                top,
                baseline,
                thresholds: Thresholds {
                    max_growth: max_growth.unwrap_or(defaults.max_growth),
                    min_delta: min_delta.unwrap_or(defaults.min_delta),
                },
                json_out,
            };
            cmd_check_proof_term_size(&file, &request)
        }
    }
}
