//! XML 1.0 escaping for the `JUnit` document (ADR 30.9.26k D7).
//!
//! A test body's captured output is arbitrary text, and one unescaped byte
//! makes the whole report unparseable — a dashboard then shows no tests at
//! all. So every string the renderer writes goes through one of these two
//! functions, and no CDATA section is ever written: `]]>` needs no special
//! case once `>` is an entity.

/// Escape `text` for an element's character content.
///
/// `&` `<` `>` become entities; `\r` becomes `&#13;`, because a parser
/// normalises a literal carriage return to a newline and a captured `\r\n`
/// would otherwise lose its `\r`. Tab and newline pass through.
pub(super) fn escape_text(text: &str) -> String {
    escape(text, false)
}

/// Escape `value` for a double-quoted attribute.
///
/// Everything [`escape_text`] does, plus `"` → `&quot;` and tab / newline as
/// character references, so attribute-value normalisation cannot fold a
/// multi-line message onto one line.
pub(super) fn escape_attr(value: &str) -> String {
    escape(value, true)
}

/// A character XML 1.0 cannot carry at all — not even as a character
/// reference. Written as `char::escape_unicode`'s visible text instead, so an
/// ANSI colour escape reads as `\u{1b}[31m` rather than vanishing.
fn is_forbidden_in_xml(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'
    )
}

fn escape(input: &str, in_attribute: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            '"' if in_attribute => out.push_str("&quot;"),
            '\t' if in_attribute => out.push_str("&#9;"),
            '\n' if in_attribute => out.push_str("&#10;"),
            c if is_forbidden_in_xml(c) => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    out
}
