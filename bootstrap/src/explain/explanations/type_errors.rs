//! Type error explanations.
//!
//! Handles: `TypeMismatch`, `CannotInferType`, `CannotInferTypeArg`,
//! `ArityMismatch`, `ExpectedFunction`, `ExpectedType`.

use crate::explain::error_catalogue::ErrorExplanation;

pub(super) fn type_errors(name: &str) -> Option<ErrorExplanation> {
    inference_errors(name)
        .or_else(|| application_errors(name))
        .or_else(|| sort_errors(name))
}

/// Type inference errors: mismatch, cannot infer.
fn inference_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "TypeMismatch" => ErrorExplanation {
            name: "TypeMismatch",
            code: "E0010",
            category: "Type Errors",
            summary: "expected one type, found another",
            detail: "\
The compiler expected one type but found a different type. This is the \
most common error in Albite programs.\n\
\n\
Common causes:\n\
• Returning the wrong type from a function body\n\
• Passing an argument of the wrong type to a function\n\
• Match arms returning different types\n\
• Using a constructor with wrong field types\n\
\n\
Structural types in this error:\n\
Albite's Core IR uses structural encodings for ADTs. If you see \
a μ-type in a TypeMismatch error, use `albite explain type` to decode it:\n\
  $ albite explain type \"μα_List. (Unit + (Nat × α_List))\"",
            example: "\
fn greet() -> String {\n\
    42              // error: expected `String`, found `Nat`\n\
}",
            see_also: &["CannotInferType", "ExpectedType", "ExpectedFunction"],
        },

        "CannotInferType" => ErrorExplanation {
            name: "CannotInferType",
            code: "E0011",
            category: "Type Errors",
            summary: "type annotation needed",
            detail: "\
The compiler cannot determine the type of an expression from context alone. \
An explicit type annotation is needed.\n\
\n\
Common causes:\n\
• Variable declaration without type annotation in ambiguous context\n\
• Generic function call where type arguments can't be inferred",
            example: "\
fn id<T>(x: T) -> T { x }\n\
\n\
fn main() -> Nat {\n\
    let y = id(42);    // may need: let y: Nat = id(42)\n\
    y\n\
}",
            see_also: &["CannotInferTypeArg", "TypeMismatch"],
        },

        "CannotInferTypeArg" => ErrorExplanation {
            name: "CannotInferTypeArg",
            code: "E0015",
            category: "Type Errors",
            summary: "cannot infer type argument",
            detail: "\
The compiler cannot infer a type argument for a polymorphic (generic) function. \
Provide explicit type arguments.\n\
\n\
Common causes:\n\
• Calling a generic function in a context without enough type information\n\
• The result type doesn't constrain the type parameter",
            example: "\
fn empty_list<T>() -> List<T> { Nil }\n\
\n\
fn main() -> List<Nat> {\n\
    empty_list()    // error: cannot infer type argument `T`\n\
    // Fix: empty_list::<Nat>()\n\
}",
            see_also: &["CannotInferType"],
        },

        _ => return None,
    };
    Some(exp)
}

/// Application and arity errors.
fn application_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "ArityMismatch" => ErrorExplanation {
            name: "ArityMismatch",
            code: "E0012",
            category: "Type Errors",
            summary: "wrong number of arguments",
            detail: "\
A function was called with the wrong number of arguments.\n\
\n\
Common causes:\n\
• Forgetting an argument\n\
• Passing too many arguments\n\
• Confusing two functions with similar names but different arities",
            example: "\
fn add(x: Nat, y: Nat) -> Nat { x + y }\n\
\n\
fn main() -> Nat {\n\
    add(1, 2, 3)    // error: expected 2 arguments, found 3\n\
}",
            see_also: &["TypeMismatch", "ExpectedFunction"],
        },

        "ExpectedFunction" => ErrorExplanation {
            name: "ExpectedFunction",
            code: "E0013",
            category: "Type Errors",
            summary: "expected function, found other type",
            detail: "\
An expression was used as a function (called with arguments), but its type \
is not a function type.\n\
\n\
Common causes:\n\
• Calling a non-function value (e.g., a Nat or a constructor used wrong)\n\
• Typo causing a variable to shadow a function name\n\
• Missing parentheses in a chain of calls",
            example: "\
fn main() -> Nat {\n\
    let x: Nat = 42;\n\
    x(1)    // error: expected function, found `Nat`\n\
}",
            see_also: &["TypeMismatch", "ArityMismatch"],
        },

        "ExpectedType" => ErrorExplanation {
            name: "ExpectedType",
            code: "E0014",
            category: "Type Errors",
            summary: "expected a specific type",
            detail: "\
The compiler expected a specific type (like Bool for an if-condition) \
but found a different type.\n\
\n\
Common causes:\n\
• Using a non-Bool expression as an if-condition\n\
• Type annotation doesn't match the expression",
            example: "\
fn main() -> Nat {\n\
    if 42 { 1 } else { 0 }    // error: expected `Bool`, found `Nat`\n\
}",
            see_also: &["TypeMismatch"],
        },

        "ComparatorUnavailable" => ErrorExplanation {
            name: "ComparatorUnavailable",
            code: "E0080",
            category: "Type Errors",
            summary: "no comparator for this type",
            detail: "\
`__compare` — which the `assert_eq_*` test assertions desugar to — was \
applied at a type the compiler cannot synthesize a structural comparator \
for.\n\
\n\
Supported: primitives, tuples, sums/ADTs, records, and lists. Function \
types are the usual culprit: two functions have no decidable equality.\n\
\n\
`albite doctor check comparable <type> <file>` reports whether a type \
is comparable before you write assertions at it.",
            example: "\
fn f(x: Nat) -> Nat { x }\n\
fn g(x: Nat) -> Nat { x }\n\
\n\
fn test_fns() -> Bool {\n\
    __compare(f, g)    // error: no comparator for `Nat -> Nat`\n\
}",
            see_also: &["TypeMismatch"],
        },

        "IntLiteralOutOfRange" => ErrorExplanation {
            name: "IntLiteralOutOfRange",
            code: "E0090",
            category: "Type Errors",
            summary: "integer literal does not fit `Int`",
            detail: "\
A literal checked against `Int` is outside the signed 64-bit range \
(-9223372036854775808 to 9223372036854775807).\n\
\n\
Literals default to `Nat`; a literal becomes `Int` only where the \
expected type says so, or under a unary minus. `Nat` reaches \
18446744073709551615, so a value that fits `Nat` can still be too large \
for `Int` — `to_int(n)` traps at runtime for the same values.",
            example: "\
fn big() -> Int {\n\
    9223372036854775808    // error: out of range for `Int`\n\
}",
            see_also: &["TypeMismatch"],
        },

        "BuiltinTypeRedefined" => ErrorExplanation {
            name: "BuiltinTypeRedefined",
            code: "E0091",
            category: "Type Errors",
            summary: "a `type` under a builtin type's name",
            detail: "\
A `type` definition uses the name of a builtin type (`Int`, `Nat`, `Bool`, \
`Unit`, `Void`, `Prop`, `String`). Builtin lookup precedes user types, so \
the definition would never be reached — every use of the name would still \
resolve to the builtin, silently.\n\
\n\
Pick another name. For a C `int` return type the compiler's own FFI module \
uses `CInt`.",
            example: "\
type Int = Nat    // error: cannot redefine the builtin type `Int`",
            see_also: &["DuplicateDefinition"],
        },

        "ExistsBinderNotData" => ErrorExplanation {
            name: "ExistsBinderNotData",
            code: "E0065",
            category: "Type Errors",
            summary: "an `exists` binds a proof, not data",
            detail: "\
`exists x: A. B` says some value `x` of type `A` makes `B` hold, and its \
proof is that value paired with evidence. The binder must range over \
data — `Nat`, a data type — never over a proposition (an equality, \
`Void`, or a function ending in one) and never over `Prop` itself.\n\
\n\
To say two propositions both hold, use a tuple of their proofs. A dependent \
function type `(x: A) -> B` whose body uses `x` is held to the same rule.",
            example: "\
pub theorem bad() -> exists h: Eq<Nat, 0, 0>. Eq<Nat, 0, 0> {\n\
    (refl, refl)    // error: an `exists` must bind data, not a proof\n\
}",
            see_also: &["TypeMismatch"],
        },

        _ => return None,
    };
    Some(exp)
}

/// Sort errors: `Prop` or `Type` as a type, and a family's sort (ADR 4.10.26i).
fn sort_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "SortUsedAsType" => ErrorExplanation {
            name: "SortUsedAsType",
            code: "E0066",
            category: "Type Errors",
            summary: "`Prop` or `Type` used as a type",
            detail: "\
`Prop` is a sort: it classifies propositions such as `a == b`, and is not \
a type with values, so no parameter, binding or result can have type \
`Prop`. `Type` likewise names the sort of data, not a type. Both words are \
written only after a statement definition's parameters: \
`type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m`.\n\
\n\
To take a proof as an argument, give the parameter the proposition itself \
(`h: n == m`), not `Prop`.",
            example: "\
fn f(p: Prop) -> Nat { 0 }    // error: `Prop` is a sort",
            see_also: &["FamilySortMismatch", "ExistsBinderNotData"],
        },

        "FamilySortMismatch" => ErrorExplanation {
            name: "FamilySortMismatch",
            code: "E0067",
            category: "Type Errors",
            summary: "a data type declared a proposition",
            detail: "\
A definition `type Name(x: A, …): Prop = B` declares a statement, so its \
body `B` must be a proposition — an equality, `Void`, or a connective or \
`exists` over them — never data such as `Nat` or a two-constructor enum. \
Its parameters are the opposite: each ranges over data, as an `exists` \
binder does.",
            example: "\
type Bad(n: Nat): Prop = Nat    // error: the body `Nat` must be a proposition",
            see_also: &["SortUsedAsType"],
        },

        "TypeParameterApplied" => ErrorExplanation {
            name: "TypeParameterApplied",
            code: "E0068",
            category: "Type Errors",
            summary: "a type parameter applied to arguments",
            detail: "\
A type parameter stands for a type, not for a type constructor, so it \
takes no arguments: in `type Ap<F, T> = Mk(F<T>)` the field `F<T>` is \
refused. Higher-kinded type parameters are not supported; write the \
parameter bare (`Mk(F)`), or name the type you meant.\n\
\n\
The same code refuses a recursive use of the type being defined that does \
not pass its own parameters, such as `Nest<Nat>` inside \
`type Nest<T> = …`: that is non-uniform recursion, which a recursive \
type's encoding cannot express.",
            example: "\
type Ap<F, T> = Mk(F<T>)    // error: type parameter `F` takes no arguments",
            see_also: &["ArityMismatch"],
        },

        "TermTypeConfusion" => ErrorExplanation {
            name: "TermTypeConfusion",
            code: "E0023",
            category: "Type Errors",
            summary: "a type where a term belongs, or a term where a type belongs",
            detail: "\
A statement family's arguments are terms: `Le<n + 1, m>` and `Le<0, n>` \
pass expressions of each parameter's type, as an `Eq`'s two sides do. A \
type written there (`Le<Nat, m>`, `Le<List<Nat>, m>`) is refused, since a \
family's parameters range over values, not types.\n\
\n\
The converse is refused too: every other type application takes types, so \
a numeral or an expression there (`List<0>`, `Ref<n + 1>`) is not a type.",
            example: "\
type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n\
fn f(m: Nat) -> Le<Nat, m> { sorry }    // error: `Nat` is a type",
            see_also: &["FamilySortMismatch", "TypeMismatch"],
        },

        "PredicateKindRefused" => ErrorExplanation {
            name: "PredicateKindRefused",
            code: "E0069",
            category: "Type Errors",
            summary: "a type parameter's kind that is not a predicate over data",
            detail: "\
A `fn`, `theorem`, `lemma` or `axiom` may quantify over a predicate: \
`<P: Nat -> Prop>` binds `P`, applied as `P<n>`. The kind must list at \
least one argument type, each closed data (no type parameter, no \
proposition), and end in `Prop`. `P: Prop` alone would range over \
propositions, which needs impredicative `Prop`; a kind ending elsewhere \
(`F: Nat -> Nat`) is a type operator; and a type definition's parameters \
take no kind at all.",
            example: "\
theorem g<P: Prop>(h: P) -> P { h }    // error: `P: Prop` ranges over propositions",
            see_also: &["TypeParameterApplied", "SortUsedAsType"],
        },

        "SumIntroNotDisjunction" => ErrorExplanation {
            name: "SumIntroNotDisjunction",
            code: "E0024",
            category: "Type Errors",
            summary: "`inl` or `inr` against a type that is not a disjunction",
            detail: "\
`inl(e)` and `inr(e)` build a disjunction: a sum `A + B` whose two sides \
are propositions, such as `Eq<Nat, n, 0> + Le<1, n>`. A declared data type \
and a written sum are the same core type, so a sum of data (`Nat + Bool`, \
or a two-constructor enum) is built by its declared constructors instead. \
They are built in only where no value named `inl` or `inr` is in scope, \
and have no inferred type: the expected type says which sum is meant.",
            example: "\
fn f() -> Nat + Bool { inl(3) }    // error: `Nat + Bool` is not a disjunction",
            see_also: &["TypeMismatch", "CannotInferType"],
        },

        "DependentFunctionField" => ErrorExplanation {
            name: "DependentFunctionField",
            code: "E0025",
            category: "Type Errors",
            summary: "a dependent function type as a data type's field",
            detail: "\
A dependent function type `(x: A) -> B`, whose body mentions `x`, states a \
hypothesis: a function, a theorem's parameter or its statement may take \
one. A declared constructor payload or record field may not. A field whose \
binder is unused is a plain function type and is accepted, and a dependent \
function type that reaches a payload by instantiation, such as \
`Option<(n: Nat) -> Eq<Nat, n, n>>`, is accepted too.",
            example: "\
type Holder = { f: (n: Nat) -> Eq<Nat, n, n> }    // error: a field of a data type",
            see_also: &["ExistsBinderNotData", "TypeMismatch"],
        },

        _ => return None,
    };
    Some(exp)
}
