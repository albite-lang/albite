//! Type errors (E0001-E0099), name resolution (E0100-E0199) and its package
//! codes (E0120-E0124).

use super::SelfHostedErrorEntry;

pub(super) const TYPING_AND_NAMES: &[SelfHostedErrorEntry] = &[
    // Type errors (E0001-E0099)
    SelfHostedErrorEntry {
        code: "E0001",
        name: "ErrTypeMismatch",
        category: "Type Errors",
        description: "Expected one type, found another. The elaborator inferred or expected a specific type but the expression produced a different one.",
    },
    SelfHostedErrorEntry {
        code: "E0002",
        name: "ErrArityMismatch",
        category: "Type Errors",
        description: "Wrong number of arguments. A function or constructor was called with the wrong number of arguments.",
    },
    SelfHostedErrorEntry {
        code: "E0003",
        name: "ErrNotAFunction",
        category: "Type Errors",
        description: "Expected a function type but found something else. An expression was used in a function call position but does not have a function type.",
    },
    SelfHostedErrorEntry {
        code: "E0004",
        name: "ErrNotAType",
        category: "Type Errors",
        description: "Expected a type but found a value or unknown name. A name was used in a type position but does not resolve to a type definition.",
    },
    SelfHostedErrorEntry {
        code: "E0005",
        name: "ErrNotAnAdt",
        category: "Type Errors",
        description: "Expected an algebraic data type (ADT) but found a different kind of type.",
    },
    SelfHostedErrorEntry {
        code: "E0006",
        name: "ErrNotASum",
        category: "Type Errors",
        description: "Expected a sum type but found something else. Match expressions require a sum-typed scrutinee.",
    },
    SelfHostedErrorEntry {
        code: "E0007",
        name: "ErrNotAPair",
        category: "Type Errors",
        description: "Expected a product/pair type but found something else. Tuple indexing requires a product type.",
    },
    SelfHostedErrorEntry {
        code: "E0008",
        name: "ErrCannotInfer",
        category: "Type Errors",
        description: "Cannot infer a type. A type annotation is needed because the elaborator cannot determine the type from context.",
    },
    SelfHostedErrorEntry {
        code: "E0009",
        name: "ErrIntLiteralOutOfRange",
        category: "Type Errors",
        description: "An integer literal outside the signed 64-bit range of `Int`.",
    },
    // E0010–E0014 stay free: E0010 is the bootstrap's TypeMismatch, and the
    // catalogue's tests use that block as their uncatalogued exemplars.
    SelfHostedErrorEntry {
        code: "E0015",
        name: "ErrSortUsedAsType",
        category: "Type Errors",
        description: "`Prop` or `Type` written where a type is expected. `Prop` is a sort: it classifies propositions such as `a == b`, and is not a type with values (ADR 4.10.26i).",
    },
    // Name resolution errors (E0100-E0199)
    SelfHostedErrorEntry {
        code: "E0100",
        name: "ErrUnresolvedType",
        category: "Name Resolution",
        description: "Type name not found in scope. The referenced type is not defined or not imported in the current module.",
    },
    SelfHostedErrorEntry {
        code: "E0101",
        name: "ErrUnresolvedValue",
        category: "Name Resolution",
        description: "Value name not found in scope. The referenced variable, function, or constructor is not defined or not imported.",
    },
    SelfHostedErrorEntry {
        code: "E0102",
        name: "ErrUnresolvedModule",
        category: "Name Resolution",
        description: "Module not found. The referenced module path does not resolve to any known module.",
    },
    SelfHostedErrorEntry {
        code: "E0103",
        name: "ErrDuplicateDef",
        category: "Name Resolution",
        description: "Name already defined. Two definitions in the same scope have the same name.",
    },
    SelfHostedErrorEntry {
        code: "E0104",
        name: "ErrPrivateAccess",
        category: "Name Resolution",
        description: "Accessing a private item. The item exists but is not visible from the current module.",
    },
    SelfHostedErrorEntry {
        code: "E0105",
        name: "ErrAmbiguousName",
        category: "Name Resolution",
        description: "Ambiguous bare value. A bare name the current module neither defines nor imports is looked up program-wide, and when two modules the caller may both reach and see define it, neither is chosen. Qualify the call (`x::helper`), or import one with `use`.",
    },
    SelfHostedErrorEntry {
        code: "E0106",
        name: "ErrDuplicateImport",
        category: "Name Resolution",
        description: "Same name imported twice. Two import statements bring the same name into scope.",
    },
    SelfHostedErrorEntry {
        code: "E0107",
        name: "ErrBuiltinTypeRedefined",
        category: "Name Resolution",
        description: "A `type` declared under the name of a builtin type such as `Int`.",
    },
    SelfHostedErrorEntry {
        code: "E0108",
        name: "ErrGlobConflict",
        category: "Name Resolution",
        description: "Two glob imports bring one name from two different definitions. Import the one you mean by name.",
    },
    // Package errors (E0120-E0124), numbered as the bootstrap's own so one
    // number means one refusal in both catalogues (ADR 6.10.26j D4).
    SelfHostedErrorEntry {
        code: "E0120",
        name: "ErrManifestInvalid",
        category: "Packages",
        description: "A `albite.toml` off the manifest schema: bad TOML, a missing or invalid `[package] name`, an unknown key, or a dependency that is not `{ path = \"…\" }`. Raised by the planner both compilers share.",
    },
    SelfHostedErrorEntry {
        code: "E0121",
        name: "ErrDependencyNotFound",
        category: "Packages",
        description: "A path dependency whose directory, `albite.toml` or `lib.tg` is missing.",
    },
    SelfHostedErrorEntry {
        code: "E0122",
        name: "ErrPackageNameConflict",
        category: "Packages",
        description: "A dependency key that disagrees with its package's name, names a second package already planned under that name, or takes the name of an application module.",
    },
    SelfHostedErrorEntry {
        code: "E0123",
        name: "ErrPackageExternCollision",
        category: "Packages",
        description: "Two packages declare the same C symbol in their externs. Declare it once, in one package, and import it from there.",
    },
    SelfHostedErrorEntry {
        code: "E0124",
        name: "ErrDependencyCycle",
        category: "Packages",
        description: "The dependency graph loops back on itself; the message names the chain from the application to the package that closes it.",
    },
];
