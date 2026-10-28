//! `albite info ffi` — the runtime's C exports (ADR 4.10.26n D9).
//!
//! `exports` lists every `#[no_mangle] extern "C"` export in `albite_core`
//! and `albite_runtime`, with its file and `cfg`, read by `doctor check link
//! extern-symbols`' recogniser unchanged. It also lists each row of the native
//! core's export table as **written in Tungsten**, with the Rust symbol it
//! shadows — the fact the list exists to carry: which exports have moved.
//! Cost 2: a source walk, no elaboration, no LLVM.
//!
//! The listing is a pure function of the scan's result and the table, so it
//! is tested against injected inputs; the walk is the only effect.
//!
//! `--classify` adds each export's trust class and its basis (ADR 5.10.26f):
//! 1.10.26k's kernel / runtime / pure rule, read from each body with `syn`,
//! plus **unknown** for a body that could not be read. The recogniser still
//! finds the exports; the classifier parses the crates they sit in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use albite_bootstrap::doctor::checks::check_extern_symbols::{scan_exports, ExportedSymbol};
use albite_bootstrap::driver::native_core::elaborate_native_core;
use albite_core::native_core::{NativeCoreExport, NATIVE_CORE_EXPORTS};
use clap::Subcommand;

use classify::extract::{CrateIndex, SourceFile};
use classify::reach::Reach;
use classify::walk::parse_crate;
use classify::{classify_export, Verdict};
use native_reach::{row_verdicts, RowVerdict};

mod classify;
mod native_reach;

/// The source roots scanned when `--root` is not given.
const DEFAULT_ROOTS: [&str; 2] = ["albite_core/src", "albite_runtime/src"];

/// `albite info ffi <subcommand>`.
#[derive(Subcommand)]
pub enum InfoFfiCommands {
    /// List the runtime's C exports, and which are written in Tungsten (cost 2)
    ///
    /// Every `#[no_mangle] extern "C"` export under the scanned roots, with
    /// its file and any `#[cfg]` gate, then each export the native core
    /// defines in Tungsten (`src/native_core/`) with the Rust export it
    /// shadows. A root that does not exist is an error, never an empty list.
    ///
    /// Examples:
    ///   albite info ffi exports
    ///   albite info ffi exports --root albite_core/src
    ///   albite info ffi exports --classify
    #[command(
        after_help = "See also: `albite doctor check link extern-symbols <file>` asks which of a \nfile's `extern \"C\"` declarations these exports resolve, and `albite info \neval externs` which externs the evaluator can execute."
    )]
    #[allow(clippy::doc_markdown)] // Reason: `--help` text, copied verbatim
    Exports {
        /// Source root to scan (repeatable; default: albite_core/src and
        /// albite_runtime/src)
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
        /// Add each export's trust class (kernel, runtime, pure or unknown)
        /// and the call or state that decided it (ADR 5.10.26f)
        #[arg(long)]
        classify: bool,
    },
}

/// Dispatch `albite info ffi`.
pub fn dispatch_ffi_info(cmd: InfoFfiCommands) -> ExitCode {
    match cmd {
        InfoFfiCommands::Exports { roots, classify } => cmd_info_ffi_exports(&roots, classify),
    }
}

/// One scanned root and what the scan found in it.
pub(crate) struct ScannedRoot {
    /// The root as given.
    pub(crate) root: String,
    /// Its exports.
    pub(crate) exports: Vec<ExportedSymbol>,
}

fn cmd_info_ffi_exports(roots: &[PathBuf], classify: bool) -> ExitCode {
    let roots: Vec<PathBuf> = if roots.is_empty() {
        DEFAULT_ROOTS.iter().map(PathBuf::from).collect()
    } else {
        roots.to_vec()
    };
    let mut scanned = Vec::new();
    for root in &roots {
        if !root.is_dir() {
            eprintln!(
                "error: {} is not a directory — run from the repository root, or pass --root",
                root.display()
            );
            return ExitCode::from(2);
        }
        match scan_exports(root) {
            Ok(exports) => scanned.push(ScannedRoot {
                root: root.display().to_string(),
                exports,
            }),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(2);
            }
        }
    }
    if !classify {
        print!("{}", render_exports(&scanned, NATIVE_CORE_EXPORTS, None));
        return ExitCode::SUCCESS;
    }
    let parsed = match parse_roots(&roots) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let (verdicts, errors) = classify_scanned(&scanned, &parsed);
    let native = native_core_verdicts();
    let classified = Classified {
        rust: &verdicts,
        native: &native,
    };
    print!(
        "{}",
        render_exports(&scanned, NATIVE_CORE_EXPORTS, Some(&classified))
    );
    for error in &errors {
        eprintln!("error: {error}");
    }
    // ADR 9.10.26c D4: an unclassed Tungsten row exits as an unclassed Rust
    // export does. Asked per row, so a row missing from the map is unclassed
    // too, as `render_exports` prints it.
    let rows_classed = NATIVE_CORE_EXPORTS
        .iter()
        .all(|row| matches!(native.get(row.definition), Some(Ok(_))));
    if errors.is_empty() && rows_classed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

/// Each root parsed whole, named by its crate directory (`albite_core/src`
/// is `albite_core`).
pub(crate) fn parse_roots(roots: &[PathBuf]) -> Result<Vec<(String, Vec<SourceFile>)>, String> {
    roots
        .iter()
        .map(|root| Ok((crate_name_of(root), parse_crate(root)?)))
        .collect()
}

fn crate_name_of(root: &Path) -> String {
    let parent = root.parent().and_then(Path::file_name);
    parent.map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A verdict per scanned export, and an error per export that resolves to no
/// item, to an ambiguous set, or to an item outside the recogniser's file.
pub(crate) fn classify_scanned(
    scanned: &[ScannedRoot],
    parsed: &[(String, Vec<SourceFile>)],
) -> (HashMap<String, Verdict>, Vec<String>) {
    let crates: Vec<(&str, &[SourceFile])> = parsed
        .iter()
        .map(|(name, files)| (name.as_str(), files.as_slice()))
        .collect();
    let index = CrateIndex::build(&crates);
    let reach = Reach::new(&index);
    let mut verdicts = HashMap::new();
    let mut errors = Vec::new();
    for export in scanned.iter().flat_map(|root| &root.exports) {
        if verdicts.contains_key(&export.symbol) {
            continue;
        }
        if !reach
            .export_files(&export.symbol)
            .contains(&export.file.as_str())
        {
            errors.push(format!(
                "`{}`: the recogniser found it in {}, the parse did not",
                export.symbol, export.file
            ));
            continue;
        }
        match reach.export_facts(&export.symbol) {
            Ok(facts) => {
                verdicts.insert(export.symbol.clone(), classify_export(&facts));
            }
            Err(error) => errors.push(error),
        }
    }
    (verdicts, errors)
}

/// Each Tungsten row's verdict, read from the elaborated embedded core (ADR
/// 9.10.26c D2). A core that does not elaborate is every row's reason (D4).
fn native_core_verdicts() -> HashMap<String, Result<RowVerdict, String>> {
    let definitions = NATIVE_CORE_EXPORTS.iter().map(|row| row.definition);
    let elaborated = tempfile::tempdir()
        .map_err(|error| format!("no scratch directory for the core: {error}"))
        .and_then(|dir| {
            elaborate_native_core(dir.path())
                .map_err(|error| format!("the core does not elaborate: {error}"))
        });
    match &elaborated {
        Ok(project) => row_verdicts(definitions, Ok(&project.defs)),
        Err(reason) => row_verdicts(definitions, Err(reason)),
    }
}

/// A pure Tungsten row's basis (5.10.26f D6): it reaches only byte intrinsics.
pub(crate) const NATIVE_CORE_BASIS: &str = "native core: byte intrinsics only";

/// `--classify`'s verdicts: each Rust export's, by symbol, and each Tungsten
/// row's, by definition — or the reason it is not classed (ADR 9.10.26c D4).
pub(crate) struct Classified<'a> {
    pub(crate) rust: &'a HashMap<String, Verdict>,
    pub(crate) native: &'a HashMap<String, Result<RowVerdict, String>>,
}

/// The listing: each root's exports in symbol order, then the table's rows.
/// Classed, a Tungsten row with no verdict prints as in the plain listing,
/// and each distinct reason follows as a `native core: not classed:` line.
pub(crate) fn render_exports(
    scanned: &[ScannedRoot],
    written: &[NativeCoreExport],
    classified: Option<&Classified>,
) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let mut total = 0;
    for root in scanned {
        let mut exports: Vec<&ExportedSymbol> = root.exports.iter().collect();
        exports.sort_by(|a, b| (&a.symbol, &a.file).cmp(&(&b.symbol, &b.file)));
        let _ = writeln!(out, "{} — {} export(s) in Rust:", root.root, exports.len());
        for export in exports {
            let gate = export
                .cfg
                .as_deref()
                .map_or_else(String::new, |cfg| format!("  #[cfg({cfg})]"));
            let class = classified.map_or_else(String::new, |classified| {
                classified.rust.get(&export.symbol).map_or_else(
                    || "  error (see below)".to_string(),
                    |verdict| format!("  {}  {}", verdict.class, verdict.basis()),
                )
            });
            let _ = writeln!(out, "  {}  {}{gate}{class}", export.symbol, export.file);
        }
        out.push('\n');
        total += root.exports.len();
    }
    let _ = writeln!(
        out,
        "Written in Albite (src/native_core/) — {}:",
        written.len()
    );
    let mut not_classed: Vec<String> = Vec::new();
    for row in written {
        let verdict = classified.map(|classified| {
            classified
                .native
                .get(row.definition)
                .cloned()
                .unwrap_or_else(|| Err(format!("`{}` was not walked", row.definition)))
        });
        let class = match verdict {
            Some(Ok(verdict)) => format!("  {}  {}", verdict.class, verdict.basis),
            Some(Err(reason)) => {
                if !not_classed.contains(&reason) {
                    not_classed.push(reason);
                }
                String::new()
            }
            None => String::new(),
        };
        // A row off by default is called only under `all` (ADR 6.10.26n D5).
        let setting = if row.on_by_default {
            "linked under ALBITE_NATIVE_CORE=on"
        } else {
            "called only under ALBITE_NATIVE_CORE=all"
        };
        let _ = writeln!(
            out,
            "  {}  ← {}  shadows {} ({setting}){class}",
            row.symbol, row.definition, row.shadows
        );
    }
    let _ = writeln!(
        out,
        "\n{total} Rust export(s) in {} root(s); {} written in Albite.",
        scanned.len(),
        written.len()
    );
    for reason in &not_classed {
        let _ = writeln!(out, "native core: not classed: {reason}");
    }
    out
}

#[cfg(test)]
mod tests;
