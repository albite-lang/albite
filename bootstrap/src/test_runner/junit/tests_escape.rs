//! D7's escaping on hostile input (ADR 30.9.26k AC 2). Every escaped string
//! is also placed in a one-element document and parsed back by `roxmltree`,
//! so "renders" and "parses" are asserted together.

use super::escape::{escape_attr, escape_text};

/// The characters XML 1.0 forbids outright.
fn forbidden_chars() -> Vec<char> {
    let mut chars: Vec<char> = ('\u{0}'..='\u{8}').collect();
    chars.extend(['\u{b}', '\u{c}']);
    chars.extend('\u{e}'..='\u{1f}');
    chars.extend(['\u{fffe}', '\u{ffff}']);
    chars
}

/// The input with only the forbidden characters rewritten — what a parser
/// must hand back.
fn expected_round_trip(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if forbidden_chars().contains(&c) {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn parsed_text(input: &str) -> String {
    let xml = format!("<e>{}</e>", escape_text(input));
    let doc = roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{e}: {xml:?}"));
    doc.root_element().text().unwrap_or("").to_string()
}

fn parsed_attr(input: &str) -> String {
    let xml = format!("<e a=\"{}\"/>", escape_attr(input));
    let doc = roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{e}: {xml:?}"));
    doc.root_element().attribute("a").unwrap().to_string()
}

const HOSTILE_INPUTS: &[&str] = &[
    "plain",
    "a & b < c > d",
    "]]>",
    "<![CDATA[x]]>",
    "say \"hi\" & 'bye'",
    "line one\nline two",
    "crlf\r\nend",
    "tab\there",
    "\u{1b}[31mred\u{1b}[0m",
    "nul\u{0}byte",
    "non-ascii → λ ✓ 日本",
    "\u{fffe}\u{ffff}",
];

// 30.9.26k AC2: markup characters are entities in both contexts.
#[test]
fn markup_characters_become_entities() {
    assert_eq!(escape_text("a&b<c>d"), "a&amp;b&lt;c&gt;d");
    assert_eq!(escape_attr("a&b<c>d"), "a&amp;b&lt;c&gt;d");
}

#[test]
fn cdata_terminator_needs_no_special_case() {
    assert_eq!(escape_text("]]>"), "]]&gt;");
    assert_eq!(escape_attr("]]>"), "]]&gt;");
}

#[test]
fn carriage_return_is_a_reference_in_both_contexts() {
    assert_eq!(escape_text("a\r\nb"), "a&#13;\nb");
    assert_eq!(escape_attr("a\r\nb"), "a&#13;&#10;b");
}

#[test]
fn attribute_escapes_quote_tab_and_newline() {
    assert_eq!(escape_attr("\"\t\n"), "&quot;&#9;&#10;");
}

#[test]
fn text_passes_quote_tab_newline_and_non_ascii_through() {
    let input = "\"q\"\tt\nn → λ 日本";
    assert_eq!(escape_text(input), input);
}

#[test]
fn every_forbidden_character_renders_as_its_unicode_escape() {
    for c in forbidden_chars() {
        let expected = c.escape_unicode().to_string();
        assert_eq!(escape_text(&c.to_string()), expected, "{c:?}");
        assert_eq!(escape_attr(&c.to_string()), expected, "{c:?}");
    }
    assert_eq!(escape_text("\u{1b}"), "\\u{1b}");
}

#[test]
fn allowed_neighbours_of_the_forbidden_ranges_pass_through() {
    for c in [' ', '\u{7f}', '\u{fffd}', '\u{10000}'] {
        assert_eq!(escape_text(&c.to_string()), c.to_string(), "{c:?}");
    }
}

#[test]
fn every_hostile_input_round_trips_as_text() {
    for input in HOSTILE_INPUTS {
        assert_eq!(parsed_text(input), expected_round_trip(input), "{input:?}");
    }
}

#[test]
fn every_hostile_input_round_trips_as_an_attribute() {
    for input in HOSTILE_INPUTS {
        assert_eq!(parsed_attr(input), expected_round_trip(input), "{input:?}");
    }
}

#[test]
fn crlf_comes_back_as_crlf() {
    assert_eq!(parsed_text("a\r\nb"), "a\r\nb");
    assert_eq!(parsed_attr("a\r\nb"), "a\r\nb");
}

#[test]
fn every_forbidden_character_round_trips_through_a_parser() {
    for c in forbidden_chars() {
        let input = format!("x{c}y");
        assert_eq!(parsed_text(&input), expected_round_trip(&input), "{c:?}");
        assert_eq!(parsed_attr(&input), expected_round_trip(&input), "{c:?}");
    }
}
