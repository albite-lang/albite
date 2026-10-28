//! The `explain error` listing's category table — the grouped `(kind, summary)`
//! data [`super::error_catalogue`] renders, resolves and fuzzy-matches over.
//!
//! Split out of `error_catalogue.rs` so that file stays under the size cap: it
//! is the one part of the catalogue that is pure data, and it grows with every
//! new error kind.

/// All error kinds grouped by category, for listing.
pub(super) struct ErrorCategory {
    pub(super) name: &'static str,
    pub(super) entries: &'static [(&'static str, &'static str)], // (kind_name, summary)
}

pub(super) const CATEGORIES: &[ErrorCategory] = &[
    // `ParseErrorKind`s, not `ElabErrorKind`s (ADR 29.9.26f D6): the parser's
    // code block E0110–E0117. Listed first because a file that does not parse
    // reports nothing else.
    ErrorCategory {
        name: "Syntax",
        entries: &[
            ("UnexpectedToken", "a token the grammar does not allow here"),
            ("UnexpectedEof", "the file ended mid-construct"),
            ("Expected", "a required construct is missing"),
            ("InvalidPattern", "not a pattern"),
            ("InvalidExpression", "not an expression"),
            ("InvalidType", "not a type"),
            ("ReservedKeyword", "a keyword used as a name"),
            (
                "Lexical",
                "a malformed token (string, char, escape, number, comment)",
            ),
        ],
    },
    // `compile`'s back-end failures (ADR 30.9.26j D8): `BuildErrorKind`, the
    // build block E0140–E0146.
    ErrorCategory {
        name: "Build",
        entries: &[
            (
                "IncompatibleFlags",
                "two `compile` flags that cannot be combined",
            ),
            (
                "MissingRequiredFlag",
                "a `compile` flag that needs another flag",
            ),
            ("CodeGenerationFailed", "LLVM code generation failed"),
            (
                "ToolUnavailable",
                "an external build tool could not be started",
            ),
            ("ToolFailed", "an external build tool exited with an error"),
            (
                "RuntimeLibraryUnusable",
                "the runtime library cannot be linked",
            ),
            (
                "OutputNotWritable",
                "an output or temporary file could not be written",
            ),
        ],
    },
    ErrorCategory {
        name: "Name Resolution",
        entries: &[
            ("UndefinedVariable", "cannot find value in scope"),
            ("UndefinedType", "cannot find type in scope"),
            ("UndefinedConstructor", "cannot find constructor in scope"),
            ("DuplicateDefinition", "name defined multiple times"),
            ("ModuleNotFound", "cannot find referenced module"),
            ("ItemNotFoundInModule", "item not found in module"),
            ("DuplicateImport", "same name imported twice"),
            ("GlobConflict", "glob imports conflict on a name"),
            (
                "AmbiguousValue",
                "a bare name matches two visible definitions",
            ),
            ("UnresolvedImport", "cannot resolve import path"),
            ("PrivateModule", "module is private"),
            ("PrivateItem", "item is private"),
            ("PublicItemLeak", "public item exposes private type"),
        ],
    },
    ErrorCategory {
        name: "Type Errors",
        entries: &[
            ("TypeMismatch", "expected one type, found another"),
            ("CannotInferType", "type annotation needed"),
            ("CannotInferTypeArg", "cannot infer type argument"),
            ("ArityMismatch", "wrong number of arguments"),
            ("ExpectedFunction", "expected function, found other type"),
            ("ExpectedType", "expected a specific type"),
            ("ComparatorUnavailable", "no comparator for this type"),
            ("IntLiteralOutOfRange", "integer literal does not fit `Int`"),
            (
                "BuiltinTypeRedefined",
                "a `type` under a builtin type's name",
            ),
            ("ExistsBinderNotData", "an `exists` binds a proof, not data"),
            // ADR 4.10.26i D3, D5: nested in `SortErrorKind`.
            ("SortUsedAsType", "`Prop` or `Type` used as a type"),
            ("FamilySortMismatch", "a data type declared a proposition"),
            // ADR 6.10.26b D3.
            (
                "TypeParameterApplied",
                "a type parameter applied to arguments",
            ),
            // ADR 6.10.26d D3.
            (
                "TermTypeConfusion",
                "a type where a term belongs, or the reverse",
            ),
            // ADR 6.10.26h D2.
            (
                "PredicateKindRefused",
                "a type parameter's kind that is not a predicate over data",
            ),
            // ADR 7.10.26l D7.
            (
                "SumIntroNotDisjunction",
                "`inl` or `inr` against a type that is not a disjunction",
            ),
            // ADR 7.10.26n D8.
            (
                "DependentFunctionField",
                "a dependent function type as a data type's field",
            ),
        ],
    },
    ErrorCategory {
        name: "Phase 1 Restrictions",
        entries: &[("UnsupportedFeature", "feature not yet supported")],
    },
    ErrorCategory {
        name: "Pattern Matching",
        entries: &[
            ("NonExhaustiveMatch", "match does not cover all cases"),
            ("MatchScrutineeNotAdt", "match arms expect an ADT scrutinee"),
            ("UnreachableArm", "pattern is unreachable"),
            ("PatternTooDeep", "pattern nesting exceeds limit"),
            ("UnsupportedPattern", "pattern form not supported"),
        ],
    },
    ErrorCategory {
        name: "Entry Point",
        entries: &[
            ("NoMainFunction", "no main function found"),
            ("ContainsSorry", "file contains sorry (cannot compile)"),
        ],
    },
    ErrorCategory {
        name: "Control Flow",
        entries: &[
            ("DeadCodeAfterReturn", "unreachable code after return"),
            ("TryOnNonTryType", "? on non-Result/Option type"),
            ("TryReturnMismatch", "? return type mismatch"),
            ("TryOutsideReturnContext", "? outside function body"),
            ("ReturnOutsideFunction", "return with no function in scope"),
            ("ReturnInsideTryBlock", "return inside a try block"),
            (
                "TryBlockRequiresResultType",
                "try block has no Result type in scope",
            ),
            (
                "TryBlockExpectedSumEncoding",
                "try block's Result is not a sum",
            ),
            (
                "TryBlockMissingConstructor",
                "try block's Result lacks a constructor",
            ),
            ("LetElseNonDiverging", "let-else branch does not diverge"),
            ("LetElseIrrefutable", "irrefutable pattern in let-else"),
            ("IfLetIrrefutable", "irrefutable pattern in if let"),
        ],
    },
    ErrorCategory {
        name: "Type Definitions",
        entries: &[
            ("RecursiveAlias", "type alias references itself"),
            ("NonStrictlyPositive", "type is not strictly positive"),
            (
                "NestedRecursiveFamily",
                "recursion nested under a generic parameter",
            ),
        ],
    },
    ErrorCategory {
        name: "Termination",
        entries: &[
            (
                "CannotProveTermination",
                "recursion is not structurally decreasing",
            ),
            ("PartialInProof", "a proof depends on a partial definition"),
        ],
    },
    // ADR 7.10.26t D7: E0092–E0093 are `HandleErrorKind`'s.
    ErrorCategory {
        name: "Affine Handles",
        entries: &[
            (
                "UsedAfterConsumed",
                "a handle used after it was handed over",
            ),
            ("CapturedByClosure", "a closure captures a handle"),
        ],
    },
    // ADR 9.10.26o D4: E0094–E0096 are `RegionErrorKind`'s.
    ErrorCategory {
        name: "Regions",
        entries: &[
            ("NotFirstOrder", "a region's value is not first-order data"),
            ("ReachesOutside", "a region body reaches a handle"),
            ("RegionInProof", "a region in a proof"),
        ],
    },
    // ADR 13.10.26r D4: E0104–E0106 are `MutableErrorKind`'s; E0102 is retired.
    ErrorCategory {
        name: "Mutable Bindings",
        entries: &[
            ("NotMut", "an assignment to a binding that is not `let mut`"),
            (
                "NotJoined",
                "an assignment no join hands back to later code",
            ),
            ("Captured", "a closure mentions a `let mut` binding"),
            ("InRegion", "a region body assigns an outer `let mut`"),
        ],
    },
    // ADR 2.10.26c D8: E0081 is `ElabErrorKind`'s, E0082–E0083 `TacticErrorKind`'s,
    // and so is `rewrite`'s E0085 (ADR 2.10.26e D7).
    ErrorCategory {
        name: "Tactics and the Kernel",
        entries: &[
            ("KernelRefusedProof", "the kernel refused a proof"),
            ("TacticNotApplicable", "a tactic cannot act on the goal"),
            ("UnsolvedGoals", "a `by` block ends with goals open"),
            ("RewriteNoOccurrence", "a `rewrite` rule rewrites nothing"),
            ("SimpStepBound", "`simp` reached its step bound"),
            ("RingUnequal", "`ring` found the two sides unequal"),
            ("OmegaUnproved", "`omega` could not prove its goal"),
            (
                "KernelExemptionViolation",
                "a kernel exemption row the build contradicts",
            ),
        ],
    },
    ErrorCategory {
        name: "Equality Proofs",
        entries: &[
            (
                "ReflExpectedEquality",
                "refl checked against a non-equality type",
            ),
            (
                "InvalidRefl",
                "refl's two sides are not definitionally equal",
            ),
            (
                "SubstExpectedEquality",
                "subst's proof argument is not an equality",
            ),
            ("TransEndpointMismatch", "trans endpoints do not meet"),
            (
                "CongExpectedFunction",
                "cong's first argument is not a function",
            ),
            (
                "MotiveNotPredicate",
                "subst motive is not a predicate lambda",
            ),
            (
                "MotiveDomainMismatch",
                "motive parameter type differs from the base type",
            ),
            ("MotiveBodyNotType", "motive body is a term, not a type"),
            (
                "NatIndMotiveNotNat",
                "natind motive does not range over Nat",
            ),
        ],
    },
    // Local path dependencies (ADR 29.9.26g), all spanned into a `albite.toml`.
    ErrorCategory {
        name: "Packages",
        entries: &[
            (
                "ManifestInvalid",
                "`albite.toml` outside the supported schema",
            ),
            ("DependencyNotFound", "a path dependency cannot be found"),
            (
                "PackageNameConflict",
                "a dependency key clashes with a name",
            ),
            (
                "CrossPackageNameCollision",
                "two packages declare one extern C symbol",
            ),
            // ADR 3.10.26m D3: nested in `PackageErrorKind` with the four above.
            (
                "DependencyCycle",
                "the dependency graph loops back on itself",
            ),
        ],
    },
    ErrorCategory {
        name: "Named Records",
        entries: &[
            ("NotARecordType", "type is not a record"),
            ("MissingRecordField", "missing field in record constructor"),
            ("ExtraRecordField", "unknown field in record constructor"),
            ("DuplicateRecordField", "field specified twice"),
        ],
    },
];
