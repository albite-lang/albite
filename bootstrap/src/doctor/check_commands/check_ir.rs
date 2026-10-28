//! `albite doctor check ir` — IR-related health checks (ADR 12.5.26h).
//!
//! Groups IR validation checks under `doctor check ir ...`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

use crate::doctor::checks;

/// IR-related health check subcommands (ADR 12.5.26h).
///
/// Grouped under `albite doctor check ir <subcommand>`.
#[derive(Subcommand)]
pub enum CheckIrCommands {
    /// Check store/load type-width consistency in LLVM IR (ADR 21.4.26b)
    ///
    /// Parses an emitted .ll file and reports store/load instructions
    /// where the value type width disagrees with the pointer target type.
    ///
    /// Examples:
    ///   albite doctor check ir layout output.ll
    Layout {
        /// The LLVM IR (.ll) file to check
        file: PathBuf,

        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },

    /// Validate per-unit declaration hygiene in emitted LLVM IR (ADR 10.5.26b)
    ///
    /// Scans the `define … { … }` bodies of `.ll` files in a directory and
    /// reports any direct `call @symbol` target that lacks a matching `declare`
    /// or `define` in the same file. Module-scope globals are skipped: the
    /// self-hosted compiler emits LLVM IR *as string literals*, and scanning
    /// them produced five false findings (ADR 28.7.26e).
    ///
    /// Examples:
    ///   albite doctor check ir declares target/ll/
    ///   albite doctor check ir declares target/ll/ --strict
    ///
    /// See also: `albite compile --emit-llvm`, `albite info codegen units`
    Declares {
        /// Directory containing `.ll` files to scan
        dir: Option<PathBuf>,

        /// Deprecated alias for the positional <DIR>. This audit took a flag
        /// where its five siblings take a positional, which is part of why ADR
        /// 28.7.26e's own first pass failed to enumerate it (§1.2). Kept working
        /// so existing scripts and the `doctor check declares` legacy alias do
        /// not break.
        #[arg(long, hide = true)]
        from_existing_ir: Option<PathBuf>,

        /// Fail (exit 2) on a vacuous pass: call targets seen but none resolved
        #[arg(long)]
        strict: bool,
    },
    /// Scan for null function pointer calls in emitted LLVM IR (ADR 10.5.26d)
    ///
    /// Searches `.ll` files for a call whose **callee position** is the `null`
    /// literal, indicating unresolved monomorphization — a mono instance was
    /// expected but the function pointer was never filled in. A named callee is
    /// never one, however its name ends: the pre-28.7.26e substring heuristic
    /// reported 15 false findings on `@cstring_is_null(`.
    ///
    /// Examples:
    ///   albite doctor check ir null-calls target/ll/
    ///   albite doctor check ir null-calls target/ll/ --strict
    NullCalls {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Fail (exit 2) on a vacuous pass: calls seen but no callee resolved
        #[arg(long)]
        strict: bool,
    },

    /// Audit Class-P indirect-param buffer discipline in LLVM IR — the standing
    /// `noalias` oracle (ADRs 1.7.26e, 17.7.26e).
    ///
    /// Callee arm: for each `$direct_mt` with a self-`musttail`, verifies its
    /// buffer/sret/env pointers are *forwarded* (never a fresh per-iteration
    /// `alloca`, R10), that the buffer slots of the tail edge are pairwise
    /// distinct (I5 — one pointer in two slots would alias two `noalias` params
    /// on the next activation), and that a buffer pointer never escapes
    /// (stored-as-value or returned, R4).
    /// Shim arm: for each `$direct` shim calling a `$direct_mt` with buffer
    /// slots, verifies its `sret_buf`/`indirect_buf.*` allocas are used only for
    /// {alloca, address derivation, lifetime/debug/memory intrinsics, the fill
    /// store, the `$direct_mt` argument, the sret read-back} (I4).
    /// Run `albite compile --emit-llvm` first, then point this at the output dir.
    ///
    /// See also: `albite info codegen indirect-abi`, `albite doctor check
    /// tco-coverage`.
    ///
    /// Prints a machine-readable `summary:` line with per-arm candidate/tracked
    /// counts. With `--strict` (CI), a vacuous pass — an arm's candidate
    /// functions exist but zero buffers were tracked (parser/format drift,
    /// ADR 2.7.26b T5a; a `shim.rs` buffer rename, ADR 17.7.26e) — is a failure.
    ///
    /// Examples:
    ///   albite doctor check ir indirect-buffers target/ll/
    ///   albite doctor check ir indirect-buffers target/ll/ --strict
    #[command(name = "indirect-buffers")]
    IndirectBuffers {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Fail (exit ≠ 0) on a vacuous pass: candidates > 0 but zero buffers tracked
        #[arg(long)]
        strict: bool,
    },

    /// Lint sret-return shapes in emitted LLVM IR (ADR 3.7.26d).
    ///
    /// Canonical-shape lint, not a general sret ABI verifier: in every
    /// function with an `sret(T)` parameter, each `ret void` must be
    /// immediately preceded by a store through the sret parameter or a
    /// `musttail call` forwarding it. A bare `call …; ret void` — the ADR
    /// 3.7.26a defect-2 silent-miscompile shape (result computed, never
    /// stored to the out-pointer) — is reported as a suspicious bare-return
    /// shape. Headers that mention `sret(` but cannot be parsed fail closed.
    /// Run `albite compile --emit-llvm` first, then point this at the
    /// output dir.
    ///
    /// See also: `albite diff exec` (dynamic evaluator-vs-native parity),
    /// `albite doctor check ir indirect-buffers`.
    ///
    /// Examples:
    ///   albite doctor check ir sret-stores target/ll/
    ///   albite doctor check ir sret-stores target/ll/ --strict
    #[command(name = "sret-stores")]
    SretStores {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Fail (exit 2) on a vacuous pass: sret functions found but no
        /// `ret void` site classified
        #[arg(long)]
        strict: bool,
    },

    /// Flag truncating merge memcpys in emitted LLVM IR (ADR 2.7.26b T6).
    ///
    /// Scans `.ll` files for a `memcpy` whose copy size is SMALLER than the
    /// aggregate alloca it reads/writes, with the loaded result feeding a
    /// `phi` — the 1.7.26e §6.6 phi-poisoning signature (a merge typed from a
    /// dead musttail arm's dummy, truncating real sret results through a
    /// 1-byte memcpy). Heuristic text backstop behind the `cast_to_type`
    /// shrinking-aggregate hard error; only direct-alloca memcpy operands are
    /// matched (documented reach). Exit codes: 0 clean, 1 matches, 2 bad input.
    /// Run `albite compile --emit-llvm` first, then point this at the output
    /// dir.
    ///
    /// See also: `albite doctor check ir indirect-buffers`, `albite doctor
    /// check tco-coverage`.
    ///
    /// Examples:
    ///   albite doctor check ir merge-truncation target/ll/
    ///   albite doctor check ir merge-truncation target/ll/ --strict
    #[command(name = "merge-truncation")]
    MergeTruncation {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Fail (exit 2) on a vacuous pass: functions scanned but no
        /// alloca/load/phi fact bound
        #[arg(long)]
        strict: bool,
    },

    /// Flag depot-instance wrapper self-calls in emitted LLVM IR (ADR 23.7.26c).
    ///
    /// A monomorphized generic instance's saturated self-call must ride its
    /// `$direct` entry; if its own `$direct`/`$direct_mt`/wrapper body `call`s
    /// its closure-returning wrapper `@<W>`, each recursion step heap-allocates
    /// an environment (the self-compile-scale OOM this ADR fixed). Precise: a
    /// legitimate higher-order call to a *different* instance's wrapper is not
    /// flagged, and neither is an arity-1 instance's saturated self-call, whose
    /// return type is its result aggregate rather than a closure pair
    /// (ADR 28.7.26e §2.3). Run `albite compile --emit-llvm` first, then
    /// point this at the output dir.
    ///
    /// See also: `albite doctor check ir null-calls`, `albite doctor check
    /// tco-coverage`.
    ///
    /// Examples:
    ///   albite doctor check ir wrapper-self-calls target/ll/
    ///   albite doctor check ir wrapper-self-calls target/ll/ --strict
    #[command(name = "wrapper-self-calls")]
    WrapperSelfCalls {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Fail (exit 2) on a vacuous pass: instance bodies entered but no
        /// call line examined
        #[arg(long)]
        strict: bool,
    },

    /// Prove every `$direct_mt` entry stays inside its codegen unit (ADR 3.10.26b).
    ///
    /// A `$direct_mt` carries the sret, indirect-buffer and word ABIs, which are
    /// sound only while nothing outside its own unit (= its `.ll` file) calls
    /// it. Clause 1 — calls: every `call`/`musttail call`/`invoke` of
    /// `<B>$direct_mt` comes from `<B>$direct` or `<B>$direct_mt` itself, in
    /// the file that defines it (`foreign-call`); and the address never becomes
    /// a value (`address-escape`). Clause 2 — declares: every foreign `declare`
    /// resolves to exactly one `define` (`unresolved-declare`) and matches its
    /// ABI signature, attributes included (`declare-mismatch`). A matching
    /// foreign declare is counted, not flagged. A corpus defining no
    /// `$direct_mt` exits 2 — it proves nothing. Exit codes: 0 clean, 1
    /// findings, 2 bad input. Run `albite compile --emit-llvm` first.
    ///
    /// See also: `albite doctor check ir indirect-buffers`, `albite info
    /// codegen symbols`.
    ///
    /// Examples:
    ///   albite doctor check ir direct-mt-callers target/ll/
    ///   albite doctor check ir direct-mt-callers target/ll/ --json
    #[command(name = "direct-mt-callers")]
    DirectMtCallers {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Output the stable `schema_version: 1` JSON form
        #[arg(long)]
        json: bool,
    },

    /// Census where small sums still cost in emitted LLVM IR (ADR 28.9.26f).
    ///
    /// Counts, per `{ i32, [N x i8] }` blob size: by-value params/returns,
    /// `sret` returns and indirect-buffer params (memory-buffer sites), payload
    /// constructions, heap `fold` sites and how many store an empty
    /// constructor, and `Option`-of-pointer constructions. A fixed rule turns
    /// the counts into B19's scope (`rescope` or `close`). A census, not a
    /// gate: never exits 1, and not part of `make check-ir-audits`.
    ///
    /// See also: `make ir-small-adt-census`, `albite info type lowering`,
    /// `albite doctor check ir indirect-buffers`.
    ///
    /// Examples:
    ///   albite doctor check ir small-adts target/ll/
    ///   albite doctor check ir small-adts target/ll/ --json --strict --emit-log emit.log
    #[command(name = "small-adts")]
    SmallAdts {
        /// Directory containing `.ll` files to scan
        dir: PathBuf,

        /// Output the stable `schema_version: 1` JSON form
        #[arg(long)]
        json: bool,

        /// Fail (exit 2) on a vacuous pass: blob or allocation candidates
        /// seen but nothing counted
        #[arg(long)]
        strict: bool,

        /// The emit's captured stderr, to count `ptr` fallback warnings
        #[arg(long)]
        emit_log: Option<PathBuf>,
    },
}

/// Dispatch an IR-related health check subcommand.
pub fn dispatch_check_ir(cmd: CheckIrCommands) -> ExitCode {
    match cmd {
        CheckIrCommands::Layout { file, json } => {
            checks::check_ir_layout::cmd_check_ir_layout(&file, json)
        }
        CheckIrCommands::Declares {
            dir,
            from_existing_ir,
            strict,
        } => declares_corpus_dir(dir, from_existing_ir).map_or_else(missing_declares_dir, |dir| {
            checks::check_declares::cmd_check_declares(&dir, strict)
        }),
        CheckIrCommands::NullCalls { dir, strict } => {
            checks::check_null_calls::cmd_check_null_calls(&dir, strict)
        }
        CheckIrCommands::IndirectBuffers { dir, strict } => {
            checks::check_indirect_buffers::cmd_check_indirect_buffers(&dir, strict)
        }
        CheckIrCommands::SretStores { dir, strict } => {
            checks::check_sret_stores::cmd_check_sret_stores(&dir, strict)
        }
        CheckIrCommands::MergeTruncation { dir, strict } => {
            checks::check_merge_truncation::cmd_check_merge_truncation(&dir, strict)
        }
        CheckIrCommands::WrapperSelfCalls { dir, strict } => {
            checks::check_wrapper_self_calls::cmd_check_wrapper_self_calls(&dir, strict)
        }
        CheckIrCommands::DirectMtCallers { dir, json } => {
            checks::ir_checks::check_direct_mt_callers::cmd_check_direct_mt_callers(&dir, json)
        }
        CheckIrCommands::SmallAdts {
            dir,
            json,
            strict,
            emit_log,
        } => {
            checks::check_small_adts::cmd_census_small_adts(&dir, json, strict, emit_log.as_deref())
        }
    }
}

/// The corpus directory for `declares`: the positional `<DIR>`, else its
/// deprecated `--from-existing-ir` alias.
///
/// A value rather than a nested `match` in the dispatcher, for two reasons: the
/// dispatcher is a flat one-`match` arm table and a second level trips the
/// match-depth gate, and the precedence rule ("positional wins") is a decision
/// worth asserting rather than reading.
fn declares_corpus_dir(
    positional: Option<PathBuf>,
    deprecated_flag: Option<PathBuf>,
) -> Option<PathBuf> {
    positional.or(deprecated_flag)
}

/// Report the missing corpus directory. Exit 2 = bad input, matching the audits'
/// own contract (0 clean / 1 findings / 2 bad input).
fn missing_declares_dir() -> ExitCode {
    eprintln!(
        "error: missing <DIR> — the directory of `.ll` files to scan\n\
         usage: albite doctor check ir declares <DIR> [--strict]"
    );
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `declares` accepts a positional `<DIR>` like its five siblings, and still
    /// honours the deprecated `--from-existing-ir` flag it shipped with — the
    /// odd-one-out spelling that helped ADR 28.7.26e's first pass overlook the
    /// audit entirely (§1.2).
    #[test]
    fn the_positional_dir_wins_over_the_deprecated_flag() {
        let positional = PathBuf::from("positional/ll");
        let flag = PathBuf::from("flag/ll");

        assert_eq!(
            declares_corpus_dir(Some(positional.clone()), None),
            Some(positional.clone()),
            "the positional form is the documented one"
        );
        assert_eq!(
            declares_corpus_dir(None, Some(flag.clone())),
            Some(flag.clone()),
            "the deprecated flag still works, so scripts do not break"
        );
        assert_eq!(
            declares_corpus_dir(Some(positional.clone()), Some(flag)),
            Some(positional),
            "given both, the positional wins — one rule, not an error"
        );
        assert_eq!(
            declares_corpus_dir(None, None),
            None,
            "neither is the caller's mistake, reported as bad input"
        );
    }

    /// Missing input is bad input (exit 2), not a clean run and not a finding.
    #[test]
    fn a_missing_corpus_dir_exits_as_bad_input() {
        let rendered = format!("{:?}", missing_declares_dir());
        assert_eq!(rendered, format!("{:?}", ExitCode::from(2)));
        assert_ne!(rendered, format!("{:?}", ExitCode::SUCCESS));
        assert_ne!(
            rendered,
            format!("{:?}", ExitCode::FAILURE),
            "bad input is distinct from a finding"
        );
    }
}
