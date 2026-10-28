//! Syntax (E0800-E0899) and other / internal errors (E0900-E0999).

use super::SelfHostedErrorEntry;

pub(super) const SYNTAX_AND_INTERNAL: &[SelfHostedErrorEntry] = &[
    // Syntax (E0800-E0899, ADR 30.9.26h D4). A fixed offset from the
    // bootstrap's E0110-E0117: the kind matching bootstrap E0110+k is
    // E0800+k. Only kinds the self-host raises are catalogued; E0804-E0805
    // (InvalidExpression, InvalidType) stay reserved.
    SelfHostedErrorEntry {
        code: "E0800",
        name: "SyntaxUnexpectedToken",
        category: "Syntax",
        description: "A token the grammar does not allow here. The parser expected one thing and found another; the message names both. Raised by every `expected X, found Y` parse error.",
    },
    SelfHostedErrorEntry {
        code: "E0801",
        name: "SyntaxUnexpectedEof",
        category: "Syntax",
        description: "The file ended mid-construct. The parser needed more input -- a closing brace, an expression -- and reached the end of the file first.",
    },
    SelfHostedErrorEntry {
        code: "E0802",
        name: "SyntaxExpected",
        category: "Syntax",
        description: "A required construct is missing. Raised where the parser can name the construct it needed but no single token it found, such as the left side of `==` in type position, which must be an identifier or literal.",
    },
    SelfHostedErrorEntry {
        code: "E0803",
        name: "SyntaxInvalidPattern",
        category: "Syntax",
        description: "A token that cannot start a pattern, such as the `=` in `let = 2`. A pattern is a name, a constructor, a literal, `_` or a parenthesised group.",
    },
    SelfHostedErrorEntry {
        code: "E0806",
        name: "SyntaxReservedKeyword",
        category: "Syntax",
        description: "A keyword used as a name. The lexer reserves words it does not yet give a meaning (`await`, `loop`, ...); `albite explain keywords` lists them.",
    },
    SelfHostedErrorEntry {
        code: "E0807",
        name: "SyntaxLexical",
        category: "Syntax",
        description: "A malformed token: an unterminated string, character literal or block comment, an invalid number, an empty character literal, an invalid escape, or a character the language does not use.",
    },
    // Other errors (E0900-E0999)
    SelfHostedErrorEntry {
        code: "E0900",
        name: "ErrRecursionLimitExceeded",
        category: "Limits",
        description: "Hit the elaboration recursion limit. The type or expression is too deeply nested.",
    },
    SelfHostedErrorEntry {
        code: "E0901",
        name: "ErrPhase1Violation",
        category: "Phases",
        description: "Violation of Phase 1 rules. An operation was attempted that is not allowed during the current elaboration phase.",
    },
    SelfHostedErrorEntry {
        code: "E0902",
        name: "ErrNotImplemented",
        category: "Internal",
        description: "Feature not yet implemented in the self-hosted compiler.",
    },
    SelfHostedErrorEntry {
        code: "E0999",
        name: "ErrOther",
        category: "Internal",
        description: "Internal fault: a compiler bug, or a refusal the bootstrap itself reports through its own catch-all E9999. User errors carry a catalogued code.",
    },
];
