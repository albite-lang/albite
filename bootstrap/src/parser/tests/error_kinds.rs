//! The kind a syntax error reports (ADR 29.9.26f AC1).
//!
//! The kind picks the catalogued code, so each test pins the kind the parser
//! raises for one condition rather than only that *some* error was raised.

use crate::error::ParseErrorKind;
use crate::parser::parse;

// 29.9.26f AC1 / D4: a lexical error is `Lexical`, with the lexer's message
// verbatim, not wrapped in ``unexpected token `…` ``.
#[test]
fn an_unterminated_string_is_lexical() {
    let (_, errors) = parse("fn g() -> Nat { \"abc }");
    let first = errors.first().expect("an unterminated string is an error");
    assert_eq!(
        first.kind,
        ParseErrorKind::Lexical("unterminated string literal".to_string())
    );
    assert_eq!(first.to_diagnostic().message, "unterminated string literal");
    assert_eq!(first.kind.code(), "E0117");
}

// 29.9.26f AC1 / D3: end of file reports `UnexpectedEof`, not an
// `UnexpectedToken("")` against an empty token.
#[test]
fn a_truncated_block_is_unexpected_eof() {
    let (_, errors) = parse("fn f() -> Nat {");
    let eof = errors
        .iter()
        .find(|e| e.kind == ParseErrorKind::UnexpectedEof)
        .unwrap_or_else(|| panic!("no UnexpectedEof among {errors:?}"));
    assert_eq!(
        eof.to_diagnostic().message,
        "expected `}`, found end of file"
    );
    assert!(
        !errors
            .iter()
            .any(|e| e.kind == ParseErrorKind::UnexpectedToken(String::new())),
        "end of file still reported as an empty token: {errors:?}"
    );
}

// The token path is not regressed: a wrong token mid-file stays
// `UnexpectedToken` naming what was found.
#[test]
fn a_wrong_token_mid_file_is_still_unexpected_token() {
    let (_, errors) = parse("fn f(x: Nat -> Nat { x }");
    assert!(
        errors
            .iter()
            .any(|e| matches!(&e.kind, ParseErrorKind::UnexpectedToken(t) if !t.is_empty())),
        "{errors:?}"
    );
    assert!(!errors
        .iter()
        .any(|e| e.kind == ParseErrorKind::UnexpectedEof));
}

// 29.9.26f AC1: a reserved keyword used as a name stays `ReservedKeyword`,
// from the lexer's fold as well as the parser's own sites.
#[test]
fn a_reserved_keyword_stays_reserved_keyword() {
    let (_, errors) = parse("fn const() -> Nat { 42 }");
    let reserved = errors
        .iter()
        .find(|e| matches!(e.kind, ParseErrorKind::ReservedKeyword(_)))
        .unwrap_or_else(|| panic!("no ReservedKeyword among {errors:?}"));
    assert_eq!(reserved.kind.code(), "E0116");
    assert!(!errors
        .iter()
        .any(|e| matches!(e.kind, ParseErrorKind::Lexical(_))));
}
