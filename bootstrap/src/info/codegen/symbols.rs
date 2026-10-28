//! Rendering for `info codegen symbols --by-function` (ADR 5.8.26b retrospective).
//!
//! Lives here rather than beside `cmd_info_symbols` in `info/commands/mod.rs`
//! for a reason worth stating: this is `#[cfg(feature = "codegen")]` code, and
//! the LLVM-free mutation lane sweeps `bootstrap/**` with
//! `--no-default-features`. A codegen-gated function outside
//! `bootstrap/src/info/codegen/` therefore gets mutants generated for it that
//! **no test in that lane can ever kill**, because the function is not compiled
//! at all — it reports as an under-asserted survivor when the truth is that the
//! sweep cannot see it. `MUTANTS_BOOTSTRAP_EXCLUDES` already excludes this
//! directory; keeping codegen-gated code inside it keeps the signal honest.
//!
//! The decision logic is deliberately NOT here — it is
//! [`crate::info::symbol_names`], which is un-gated and therefore genuinely
//! swept. What remains below is formatting.

use std::fmt::Write as _;

use crate::info::symbol_names;

/// The Core names `query` selects among the elaborated definitions, each
/// paired with the source name `table` gives it (ADR 13.10.26a D1, D2).
pub(crate) fn resolve_query(
    query: &str,
    table: &albite_bootstrap::elaborate::CoreNames,
    defs: &[albite_bootstrap::driver::CoreDef],
) -> Vec<String> {
    let pairs: Vec<(&str, &str)> = defs
        .iter()
        .map(|def| (def.name.as_str(), table.source_name(&def.name)))
        .collect();
    symbol_names::resolve_by_function(query, &pairs)
}

/// `--by-function QUERY`: every symbol each resolved Core name compiles to,
/// one set per name (ADR 13.10.26a D3) — two when two modules share the
/// source name.
///
/// The three name-derived symbols always print for a name the program defines,
/// because their existence is a property of the emitter's naming rules rather
/// than of this module — a reader summing a profile needs to know to look for
/// `$direct_mt` whether or not this particular build emitted samples into it.
/// Lambdas are joined in from the emitted symbol map, where their presence IS
/// module-specific. `resolved` is never empty: the caller answers that case
/// with [`symbol_names::no_definition_line`] instead (D4).
pub(crate) fn render_function_symbol_sets(
    query: &str,
    resolved: &[String],
    symbols: &[albite_codegen::SymbolEntry],
) -> String {
    let rows: Vec<(Option<&str>, &str)> = symbols
        .iter()
        .map(|e| (e.source_name.as_deref(), e.ir_name.as_str()))
        .collect();
    let mut listing = String::new();
    for (position, core_name) in resolved.iter().enumerate() {
        if position > 0 {
            listing.push('\n');
        }
        let owned = symbol_names::owned_row_indices(&rows, core_name);
        render_one_set(
            &mut listing,
            query,
            core_name,
            owned.iter().map(|&i| &symbols[i]),
        );
    }
    if resolved.len() > 1 {
        let _ = writeln!(
            listing,
            "\n`{query}` is defined in {} modules; each set above is a separate function.",
            resolved.len()
        );
    }
    listing
}

fn render_one_set<'a>(
    listing: &mut String,
    query: &str,
    core_name: &str,
    lambdas: impl Iterator<Item = &'a albite_codegen::SymbolEntry>,
) {
    let _ = writeln!(
        listing,
        "{}",
        symbol_names::symbol_set_heading(query, core_name)
    );
    let _ = writeln!(listing);
    let _ = writeln!(listing, "{:<44} ROLE", "SYMBOL");
    let _ = writeln!(listing, "{}", "─".repeat(100));
    for sym in symbol_names::conventional_symbols(core_name) {
        let _ = writeln!(listing, "{:<44} {}", sym.symbol, sym.role);
    }
    let mut lambda_count = 0;
    for entry in lambdas {
        lambda_count += 1;
        let loc = match (&entry.file, entry.line) {
            (Some(f), Some(l)) => format!("closure body ({f}:{l})"),
            _ => "closure body".to_string(),
        };
        let _ = writeln!(listing, "{:<44} {}", entry.ir_name, loc);
    }
    let _ = writeln!(listing);
    let _ = writeln!(
        listing,
        "{} symbol(s): 3 name-derived + {lambda_count} lambda(s).",
        3 + lambda_count
    );
    let _ = writeln!(
        listing,
        "To attribute a `perf` profile to `{core_name}`, sum self time across ALL of them — \
         `<name>` and `<name>$direct` alone under-count."
    );
}
