//! Item and definition errors (E0200-E0299) and pattern errors (E0300-E0399).

use super::SelfHostedErrorEntry;

pub(super) const ITEMS_AND_PATTERNS: &[SelfHostedErrorEntry] = &[
    // Item/definition errors (E0200-E0299)
    SelfHostedErrorEntry {
        code: "E0200",
        name: "ErrMissingReturnType",
        category: "Items",
        description: "Function missing return type annotation. The function signature needs an explicit return type.",
    },
    SelfHostedErrorEntry {
        code: "E0201",
        name: "ErrInvalidExtern",
        category: "Items",
        description: "Invalid extern function declaration.",
    },
    SelfHostedErrorEntry {
        code: "E0202",
        name: "ErrImportNotFound",
        category: "Items",
        description: "Import path could not be resolved. The path in a `use` statement does not point to a valid module or item.",
    },
    SelfHostedErrorEntry {
        code: "E0203",
        name: "ErrGlobImportEmpty",
        category: "Items",
        description: "Glob import (`use foo::*`) resolved to zero items.",
    },
    SelfHostedErrorEntry {
        code: "E0204",
        name: "ErrCyclicDependency",
        category: "Items",
        description: "Cycle detected in type definitions. Types cannot reference each other in a way that creates an infinite structure without indirection.",
    },
    SelfHostedErrorEntry {
        code: "E0205",
        name: "ErrInvalidVisibility",
        category: "Items",
        description: "Visibility modifier on an invalid item. `pub` was used on an item that doesn't support visibility.",
    },
    SelfHostedErrorEntry {
        code: "E0206",
        name: "ErrMissingBody",
        category: "Items",
        description: "Function or theorem without a body. Non-extern functions must have an implementation.",
    },
    // Pattern matching errors (E0300-E0399)
    SelfHostedErrorEntry {
        code: "E0300",
        name: "ErrInexhaustiveMatch",
        category: "Patterns",
        description: "Non-exhaustive match. Not all possible values of the scrutinee type are covered by the match arms.",
    },
    SelfHostedErrorEntry {
        code: "E0301",
        name: "ErrUnreachablePattern",
        category: "Patterns",
        description: "Unreachable pattern. A match arm can never be reached because earlier arms already cover all its cases.",
    },
    SelfHostedErrorEntry {
        code: "E0302",
        name: "ErrConstructorNotFound",
        category: "Patterns",
        description: "Constructor not found for the given type. The pattern references a constructor that doesn't exist on the matched type.",
    },
    SelfHostedErrorEntry {
        code: "E0303",
        name: "ErrDuplicateField",
        category: "Patterns",
        description: "Field specified twice in a record pattern or constructor.",
    },
    SelfHostedErrorEntry {
        code: "E0304",
        name: "ErrMissingField",
        category: "Patterns",
        description: "Required field missing from record pattern or constructor.",
    },
    SelfHostedErrorEntry {
        code: "E0305",
        name: "ErrExtraField",
        category: "Patterns",
        description: "Unexpected field in record pattern or constructor.",
    },
    SelfHostedErrorEntry {
        code: "E0306",
        name: "ErrMultiArmMatch",
        category: "Patterns",
        description: "Match with too many arms. Only two-arm matches are supported for ADTs (one per constructor).",
    },
    SelfHostedErrorEntry {
        code: "E0307",
        name: "ErrPatternTooDeep",
        category: "Patterns",
        description: "Pattern nesting exceeds the depth limit.",
    },
    SelfHostedErrorEntry {
        code: "E0308",
        name: "ErrUnsupportedPattern",
        category: "Patterns",
        description: "A pattern the `match` cannot take, such as a shape its lowering does not plan, or a guard on a match over a user type.",
    },
];
