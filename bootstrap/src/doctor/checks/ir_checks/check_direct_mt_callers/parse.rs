//! Text-level parsing for `direct-mt-callers`: `define`/`declare` headers with
//! their ABI signature, and every `@…$direct_mt` reference on a line.

use super::DIRECT_MT_SUFFIX;
use crate::doctor::checks::ir_checks::textparse::split_top_level_commas;

/// Whether a header defines a body or only declares the symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderKind {
    Define,
    Declare,
}

/// One `define`/`declare` header of a `$direct_mt` symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectMtHeader {
    pub(super) kind: HeaderKind,
    pub(super) symbol: String,
    /// The ABI signature: return type, then each parameter's type with its
    /// attributes, value names dropped (ADR 3.10.26b D3).
    pub(super) signature: String,
}

/// Linkage and visibility words a `define` may carry and a `declare` of the
/// same symbol in another unit never does. They decide symbol visibility, not
/// the calling convention, so keeping them would make every foreign `declare`
/// of an `internal` definition a false mismatch.
const LINKAGE_WORDS: &[&str] = &["internal", "private", "external", "dso_local", "hidden"];

/// Parse a top-level `define`/`declare` line whose symbol is a `$direct_mt`.
/// `None` for any other line, including headers of other symbols.
pub(super) fn parse_direct_mt_header(line: &str) -> Option<DirectMtHeader> {
    let (kind, rest) = if let Some(rest) = line.strip_prefix("define ") {
        (HeaderKind::Define, rest)
    } else {
        (HeaderKind::Declare, line.strip_prefix("declare ")?)
    };
    let at = rest.find('@')?;
    let (symbol, symbol_len) = symbol_at(&rest[at + 1..])?;
    if !symbol.ends_with(DIRECT_MT_SUFFIX) {
        return None;
    }
    let return_type: Vec<&str> = rest[..at]
        .split_whitespace()
        .filter(|word| !LINKAGE_WORDS.contains(word))
        .collect();
    let params = parameter_list(&rest[at + 1 + symbol_len..])?;
    let normalised: Vec<String> = split_top_level_commas(params)
        .into_iter()
        .map(drop_value_name)
        .filter(|param| !param.is_empty())
        .collect();
    Some(DirectMtHeader {
        kind,
        symbol: symbol.to_string(),
        signature: format!("{} ({})", return_type.join(" "), normalised.join(", ")),
    })
}

/// The text inside the `( … )` that opens `after_symbol`, matched by depth so
/// a nested `sret({ i64, i64 })` does not close it early.
fn parameter_list(after_symbol: &str) -> Option<&str> {
    let inner = after_symbol.strip_prefix('(')?;
    let mut depth = 0i32;
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' if depth == 0 => return Some(&inner[..i]),
            ')' | '}' | ']' | '>' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// One parameter with its trailing `%name` dropped and whitespace collapsed:
/// `ptr noalias  %0` and `ptr noalias` are the same ABI slot.
fn drop_value_name(param: &str) -> String {
    let mut words: Vec<&str> = param.split_whitespace().collect();
    if words.last().is_some_and(|word| word.starts_with('%')) {
        words.pop();
    }
    words.join(" ")
}

/// The symbol named at the start of `after_at` (the text following an `@`),
/// with how many bytes its spelling took — quoted `"f$direct_mt"` or bare
/// `f$direct_mt`, since LLVM allows `$` unquoted.
pub(super) fn symbol_at(after_at: &str) -> Option<(&str, usize)> {
    if let Some(quoted) = after_at.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some((&quoted[..end], end + 2));
    }
    let end = after_at
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '$' | '.' | '_' | '-')))
        .unwrap_or(after_at.len());
    (end > 0).then(|| (&after_at[..end], end))
}

/// Every `$direct_mt` symbol referenced on `line`, in order of appearance — a
/// call's callee and any use of the address as a value alike.
pub(super) fn direct_mt_references(line: &str) -> Vec<&str> {
    line.match_indices('@')
        .filter_map(|(at, _)| symbol_at(&line[at + 1..]))
        .map(|(symbol, _)| symbol)
        .filter(|symbol| symbol.ends_with(DIRECT_MT_SUFFIX))
        .collect()
}
