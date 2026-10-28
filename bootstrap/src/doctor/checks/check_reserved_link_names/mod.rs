//! `albite doctor check link reserved-names` — does every C name the
//! runtime, the native core and a compiled program import sit in the
//! reserved link-name table (ADR 9.10.26f D2)?
//!
//! A program's definition named like one of those imports is spelled apart by
//! the elaborator (`CoreNames`), so that the static link cannot join it to the
//! runtime's own call — but only if the name is in
//! `albite_core::link_names`. This reads what the built artefacts really
//! import and fails, naming the symbol and the file, on any the table and its
//! prefix rules lack. `make check-reserved-link-names` feeds it the runtime
//! archive, the native core object and a probe program's object and binary.
//!
//! The decision is a pure function over `nm` text ([`parse_nm`],
//! [`ReservedNamesCensus::from_tables`]); [`run_nm`] is the only effect.
//! Exit codes follow `extern-symbols`: 0 clean, 2 findings, 1 hard failure —
//! which includes inputs that import nothing at all, since a census over no
//! imports would otherwise read as clean.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use albite_core::link_names::is_reserved_link_name;

#[cfg(test)]
mod tests;

/// One input's global symbols, in their C spelling.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SymbolTable {
    /// Names the input references and does not define.
    pub undefined: BTreeSet<String>,
    /// Names the input defines with external linkage.
    pub defined: BTreeSet<String>,
}

/// Parse `nm`'s default output. A defined global is `<address> <TYPE>
/// <name>` with an uppercase type; an undefined one is `U <name>` (or a weak
/// `w`/`v`) with no address; a bare name is undefined too (`nm -u`). Archive
/// member headers (`x.o:`, `lib.a(x.o):`), blank lines and local symbols are
/// skipped. `strip_leading_underscore` is Mach-O's C-to-symbol prefix.
pub(crate) fn parse_nm(text: &str, strip_leading_underscore: bool) -> SymbolTable {
    let mut table = SymbolTable::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (undefined, raw) = match fields.as_slice() {
            [name] if !name.ends_with(':') => (true, *name),
            [kind, name] if matches!(*kind, "U" | "w" | "v") => (true, *name),
            [_, kind, name] if is_global_definition(kind) => (false, *name),
            _ => continue,
        };
        let name = c_spelling(raw, strip_leading_underscore);
        if name.is_empty() {
            continue;
        }
        if undefined {
            table.undefined.insert(name);
        } else {
            table.defined.insert(name);
        }
    }
    table
}

/// An `nm` type letter for a definition with external linkage.
fn is_global_definition(kind: &str) -> bool {
    kind.len() == 1 && kind != "U" && kind.chars().all(|c| c.is_ascii_uppercase())
}

/// The C identifier behind a symbol: Mach-O's leading `_` removed, and a
/// glibc `@VERSION` or Darwin `$VARIANT` suffix cut off.
fn c_spelling(raw: &str, strip_leading_underscore: bool) -> String {
    let unprefixed = if strip_leading_underscore {
        raw.strip_prefix('_').unwrap_or(raw)
    } else {
        raw
    };
    let end = unprefixed.find(['@', '$']).unwrap_or(unprefixed.len());
    unprefixed[..end].to_string()
}

/// What the inputs import from outside themselves, and which of those the
/// table does not reserve.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReservedNamesCensus {
    /// How many inputs were read.
    pub inputs: usize,
    /// How many distinct names the inputs import and none of them defines.
    pub imported: usize,
    /// Each unreserved import → the inputs that import it.
    pub unreserved: BTreeMap<String, Vec<String>>,
}

impl ReservedNamesCensus {
    /// The census over `(input label, table)` pairs. A name one input
    /// imports and another defines (a program unit's call into another, an
    /// archive member's into a sibling) is resolved inside the link and is
    /// not an import.
    pub(crate) fn from_tables(tables: &[(String, SymbolTable)]) -> Self {
        let defined: BTreeSet<&String> = tables.iter().flat_map(|(_, t)| &t.defined).collect();
        let mut imported: BTreeSet<&String> = BTreeSet::new();
        let mut unreserved: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (label, table) in tables {
            for name in table.undefined.iter().filter(|n| !defined.contains(n)) {
                imported.insert(name);
                if !is_reserved_link_name(name) {
                    unreserved
                        .entry(name.clone())
                        .or_default()
                        .push(label.clone());
                }
            }
        }
        Self {
            inputs: tables.len(),
            imported: imported.len(),
            unreserved,
        }
    }

    /// 0 clean, 2 findings, 1 when nothing was imported at all.
    pub(crate) fn exit_status(&self) -> u8 {
        if self.imported == 0 {
            1
        } else if self.unreserved.is_empty() {
            0
        } else {
            2
        }
    }

    /// The report, one line per unreserved name.
    pub(crate) fn render(&self) -> String {
        let reach = format!(
            "{} input(s), {} imported C name(s)",
            self.inputs, self.imported
        );
        match self.exit_status() {
            0 => format!("✓ reserved link names: {reach}, all reserved\n"),
            1 => format!(
                "error: reserved link names: {reach} — nothing imported, so nothing was checked\n"
            ),
            _ => {
                let mut findings = format!(
                    "✗ reserved link names: {reach}, {} not reserved\n",
                    self.unreserved.len()
                );
                for (name, inputs) in &self.unreserved {
                    findings.push_str(&format!("  {name} — imported by {}\n", inputs.join(", ")));
                }
                findings.push_str(
                    "  add each to RESERVED_LINK_NAMES in albite_core/src/link_names.rs\n",
                );
                findings
            }
        }
    }
}

/// The seam: `nm` over one file. Apple's `nm` must be told not to parse the
/// LLVM bitcode Rust embeds in its standard library objects.
fn run_nm(file: &Path) -> Result<String, String> {
    let mut command = Command::new("nm");
    if cfg!(target_os = "macos") {
        command.arg("--no-llvm-bc");
    }
    let output = command
        .arg(file)
        .output()
        .map_err(|e| format!("cannot run nm on {}: {e}", file.display()))?;
    if output.stdout.is_empty() && !output.status.success() {
        return Err(format!(
            "nm failed on {}: {}",
            file.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Entry point for `albite doctor check link reserved-names <file>…`.
pub fn cmd_check_reserved_link_names(files: &[PathBuf]) -> ExitCode {
    let mut tables = Vec::new();
    for file in files {
        match run_nm(file) {
            Ok(text) => tables.push((
                file.display().to_string(),
                parse_nm(&text, cfg!(target_os = "macos")),
            )),
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::FAILURE;
            }
        }
    }
    let census = ReservedNamesCensus::from_tables(&tables);
    let report = census.render();
    match census.exit_status() {
        0 => print!("{report}"),
        _ => eprint!("{report}"),
    }
    ExitCode::from(census.exit_status())
}
