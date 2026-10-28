//! Explanations for the parser's syntax-error codes, E0110–E0117
//! (ADR 29.9.26f D6). The codes come from `ParseErrorKind::code()`.

use crate::explain::error_catalogue::ErrorExplanation;

const CATEGORY: &str = "Syntax";

/// `albite explain error E0110` … `E0117`.
pub(in crate::explain) fn syntax(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "UnexpectedToken" => ErrorExplanation {
            name: "UnexpectedToken",
            code: "E0110",
            category: CATEGORY,
            summary: "a token the grammar does not allow here",
            detail: "\
The parser met a token that cannot continue the construct it was reading. \
The message names the token it found and, where the grammar allows only a \
few, the tokens it expected instead.\n\
\n\
The error points at the token where parsing stopped, which is often one \
token *after* the real mistake: a missing `)` or `,` is usually reported at \
whatever follows it. When the underlined token looks correct, check the one \
before it — an identifier spelled like a reserved word is a common cause \
(see ReservedKeyword).",
            example: "\
fn f(x: Nat -> Nat { x }   // error: expected `)`, found `{`",
            see_also: &["Expected", "UnexpectedEof", "ReservedKeyword"],
        },

        "UnexpectedEof" => ErrorExplanation {
            name: "UnexpectedEof",
            code: "E0111",
            category: CATEGORY,
            summary: "the file ended mid-construct",
            detail: "\
The file ended while the parser still needed a token — most often a closing \
`}`, `)` or `]` whose opening partner is further up the file. Find the \
unclosed bracket, not the last line.\n\
\n\
Not every early end reports this code: a construct that fails in another way \
at end of file keeps its own kind (for instance Expected), so group \"file \
ended early\" by the message as well as by E0111.",
            example: "\
fn f() -> Nat {\n\
    42\n\
// error: expected `}`, found end of file",
            see_also: &["UnexpectedToken", "Expected"],
        },

        "Expected" => ErrorExplanation {
            name: "Expected",
            code: "E0112",
            category: CATEGORY,
            summary: "a required construct is missing",
            detail: "\
The grammar requires a particular construct here — a name, a type, an \
expression, a block — and something else stands in its place. The message \
says which construct was required.\n\
\n\
Unlike UnexpectedToken, which is about one token, this is about a whole \
missing piece: a `use` with no path, an attribute with no closing `]`, a \
declaration with nothing where an item should start.",
            example: "\
use ;                      // error: expected identifier",
            see_also: &["UnexpectedToken", "InvalidType", "InvalidExpression"],
        },

        "InvalidPattern" => ErrorExplanation {
            name: "InvalidPattern",
            code: "E0113",
            category: CATEGORY,
            summary: "not a pattern",
            detail: "\
A `match` arm, a `let` binding or a parameter expected a pattern: a variable, \
`_`, a literal, a constructor with sub-patterns, or a tuple of patterns. The \
text here is none of those — an arbitrary expression cannot be matched \
against.\n\
\n\
A name that is a reserved word is the common surprise: `fn f(sym: T)` fails \
because `sym` is a proof keyword, and the error is reported at the token \
after it. `albite explain keywords` lists them.",
            example: "\
fn g() -> Nat { let = 2; 0 }   // error: invalid pattern",
            see_also: &["ReservedKeyword", "UnsupportedPattern"],
        },

        "InvalidExpression" => ErrorExplanation {
            name: "InvalidExpression",
            code: "E0114",
            category: CATEGORY,
            summary: "not an expression",
            detail: "\
The parser expected an expression — a value, a call, an operator application, \
a block — and found a token that cannot begin one, such as a stray operator \
or a closing bracket.",
            example: "\
fn f() -> Nat { * 2 }      // error: invalid expression",
            see_also: &["Expected", "UnexpectedToken"],
        },

        "InvalidType" => ErrorExplanation {
            name: "InvalidType",
            code: "E0115",
            category: CATEGORY,
            summary: "not a type",
            detail: "\
A type annotation was required — after `:` or `->`, or inside `<…>` — and the \
text there is not a type. A type is a name (optionally with arguments), a \
tuple of types, a function type, or an equality `Eq T a b`.",
            example: "\
fn f() -> Nat { let x: = 1; x }   // error: invalid type",
            see_also: &["Expected", "UndefinedType"],
        },

        "ReservedKeyword" => ErrorExplanation {
            name: "ReservedKeyword",
            code: "E0116",
            category: CATEGORY,
            summary: "a keyword used as a name",
            detail: "\
The name is a keyword — one the language uses, or one reserved for future \
use — so it cannot name a function, a parameter, a binding or a module. \
Rename it.\n\
\n\
`albite explain keywords [<name>]` answers from the lexer's own table \
whether a word is reserved, and why.",
            example: "\
fn const() -> Nat { 42 }   // error: `const` is a reserved keyword",
            see_also: &["InvalidPattern", "UnexpectedToken"],
        },

        "Lexical" => ErrorExplanation {
            name: "Lexical",
            code: "E0117",
            category: CATEGORY,
            summary: "a malformed token (string, char, escape, number, comment)",
            detail: "\
The text could not be split into tokens at all: a string or character literal \
with no closing quote, an unknown escape sequence, a malformed number, an \
unterminated block comment, a character the language does not use, or `--` \
written for a comment (Albite comments are `//` and `/* */`).\n\
\n\
The message is the lexer's own and names which of these it was. An \
unterminated literal or comment usually produces follow-on errors further \
down the file, because everything after it was swallowed — fix this one \
first.",
            example: "\
fn g() -> Nat { \"abc }     // error: unterminated string literal",
            see_also: &["UnexpectedToken"],
        },

        _ => return None,
    };
    Some(exp)
}
