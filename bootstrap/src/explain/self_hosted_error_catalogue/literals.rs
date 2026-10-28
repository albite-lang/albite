//! Every `E….` code the self-host's elaborator spells resolves in this
//! catalogue (ADR 2.10.26i D9).
//!
//! The self-host's `ErrCoded` carries a code string straight through to a
//! record, so a typo — or a *bootstrap* number such as `E0010` — would render
//! without complaint. This scan reads the two places a code is spelled: a
//! string-literal argument of a `*_coded(` constructor or `ErrCoded(`, and an
//! `=> "E…"` arm (`codes.tg`). A literal that is not a code at all, such as the
//! cascade marker, is never a candidate.

use super::entries::SELF_HOSTED_ERRORS;

/// Each code literal in `source` that does not resolve in the catalogue, in
/// source order, duplicates kept.
pub(super) fn uncatalogued_code_literals(source: &str) -> Vec<String> {
    code_literals(&strip_line_comments(source))
        .into_iter()
        .filter(|code| !SELF_HOSTED_ERRORS.iter().any(|e| e.code == code))
        .collect()
}

/// Whether a string literal's content is an error code: `E` and four digits.
fn is_code(text: &str) -> bool {
    text.len() == 5 && text.starts_with('E') && text[1..].bytes().all(|b| b.is_ascii_digit())
}

/// Whether a callee's string-literal arguments are codes.
fn is_coded_callee(ident: &str) -> bool {
    ident.ends_with("_coded") || ident == "ErrCoded"
}

/// Every code literal: the arguments of a coded call, and `=> "E…"` arms.
fn code_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            // Skip over a string so a `(` or `=>` inside one is not read.
            i = string_end(bytes, i);
            continue;
        }
        if bytes[i] == b'(' && is_coded_callee(ident_before(source, i)) {
            let close = matching_close(bytes, i);
            out.extend(string_literals(&source[i + 1..close]).filter(|s| is_code(s)));
        } else if bytes[i..].starts_with(b"=>") {
            let rest = source[i + 2..].trim_start();
            if rest.starts_with('"') {
                out.extend(string_literals(rest).take(1).filter(|s| is_code(s)));
            }
        }
        i += 1;
    }
    out
}

/// The identifier that ends immediately before byte `at`.
fn ident_before(source: &str, at: usize) -> &str {
    let head = &source[..at];
    let start = head
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |p| p + 1);
    &head[start..]
}

/// The index of the quote closing the string literal that opens at `open`;
/// the end of the input when it is unterminated.
fn closing_quote(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The index just past the string literal that opens at `open`.
fn string_end(bytes: &[u8], open: usize) -> usize {
    closing_quote(bytes, open) + 1
}

/// The index of the `)` matching the `(` at `open`, skipping strings; the end
/// of the input when it is unbalanced.
fn matching_close(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = string_end(bytes, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// The contents of every string literal in `text`, escapes left as written.
fn string_literals(text: &str) -> impl Iterator<Item = String> + '_ {
    let bytes = text.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let close = closing_quote(bytes, i).min(bytes.len());
        let lit = text[i + 1..close].to_string();
        i = close + 1;
        Some(lit)
    })
}

/// `source` with every `//` comment removed, so prose that quotes a call is
/// not read as one.
fn strip_line_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut cut = line.len();
        while i < bytes.len() {
            if bytes[i] == b'"' {
                i = string_end(bytes, i);
                continue;
            }
            if bytes[i..].starts_with(b"//") {
                cut = i;
                break;
            }
            i += 1;
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}
