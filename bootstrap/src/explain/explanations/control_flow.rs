//! Error explanations for control flow errors (return, ?, let-else, region).

use crate::explain::error_catalogue::ErrorExplanation;
pub(super) fn control_flow(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "DeadCodeAfterReturn" => ErrorExplanation {
            name: "DeadCodeAfterReturn",
            code: "W0002",
            category: "Control Flow",
            summary: "unreachable code after return",
            detail: "\
Code after a `return` expression is unreachable and will never execute.\n\
\n\
The `return` expression exits the function immediately, so any \
statements or expressions after it in the same block are dead code.",
            example: "\
fn foo() -> Nat {\n\
    return 42;\n\
    100    // warning: unreachable code after `return`\n\
}",
            see_also: &["UnreachableArm"],
        },

        "TryOnNonTryType" => ErrorExplanation {
            name: "TryOnNonTryType",
            code: "E0040",
            category: "Control Flow",
            summary: "? on non-Result/Option type",
            detail: "\
The `?` operator can only be used on `Result<T, E>` or `Option<T>` types. \
It was applied to a type that is neither.\n\
\n\
`?` desugars to a match that unwraps the success case (`Ok` or `Some`) \
and early-returns the error case (`Err` or `None`).",
            example: "\
fn foo() -> Result<Nat, String> {\n\
    let x: Nat = 42;\n\
    x?    // error: ? requires Result or Option, found Nat\n\
}",
            see_also: &["TryReturnMismatch", "TryOutsideReturnContext"],
        },

        "TryReturnMismatch" => ErrorExplanation {
            name: "TryReturnMismatch",
            code: "E0041",
            category: "Control Flow",
            summary: "? return type mismatch",
            detail: "\
The `?` operator requires the enclosing function's return type to be \
compatible with the operand type:\n\
\n\
• `Result<T, E>?` requires the function to return `Result<_, E>` \
(error types must match).\n\
• `Option<T>?` requires the function to return `Option<_>`.\n\
\n\
This ensures the early-return value is type-safe.",
            example: "\
fn foo() -> Nat {\n\
    let x: Result<Nat, String> = Ok(42);\n\
    x?    // error: cannot use ? in function returning Nat\n\
}\n\
\n\
// Fix: return Result\n\
fn foo() -> Result<Nat, String> {\n\
    let x: Result<Nat, String> = Ok(42);\n\
    x?    // ok: function returns Result<_, String>\n\
}",
            see_also: &["TryOnNonTryType", "TryOutsideReturnContext"],
        },

        "TryOutsideReturnContext" => ErrorExplanation {
            name: "TryOutsideReturnContext",
            code: "E0042",
            category: "Control Flow",
            summary: "? outside function body",
            detail: "\
The `?` operator can only be used inside a function or closure body \
where the return type is known. It was used at module scope or in \
a context without a return type.\n\
\n\
`?` desugars to an early return, which requires an enclosing function.",
            example: "\
// At module scope:\n\
let x = some_result()?;    // error: ? outside function body\n\
\n\
// Fix: use inside a function\n\
fn process() -> Result<Nat, String> {\n\
    let x = some_result()?;    // ok\n\
    Ok(x)\n\
}",
            see_also: &["TryOnNonTryType", "TryReturnMismatch"],
        },

        "LetElseNonDiverging" => ErrorExplanation {
            name: "LetElseNonDiverging",
            code: "E0043",
            category: "Control Flow",
            summary: "let-else branch does not diverge",
            detail: "\
The `else` branch of a `let`-`else` statement must diverge — it must \
not return a value to the enclosing scope. Typically this means using \
`return` to exit the function early.\n\
\n\
The `else` branch runs when the pattern does not match, so it must \
exit the scope (e.g., via `return` or a diverging expression).",
            example: "\
fn foo(x: Option<Nat>) -> Nat {\n\
    let Some(v) = x else { 0 };    // error: else branch does not diverge\n\
\n\
    // Fix: use return\n\
    let Some(v) = x else { return 0 };\n\
    v\n\
}",
            see_also: &["LetElseIrrefutable"],
        },

        "LetElseIrrefutable" => ErrorExplanation {
            name: "LetElseIrrefutable",
            code: "W0003",
            category: "Control Flow",
            summary: "irrefutable pattern in let-else",
            detail: "\
The pattern in a `let`-`else` statement is irrefutable — it always \
matches, making the `else` branch unreachable.\n\
\n\
Use a plain `let` binding instead, since the pattern can never fail.",
            example: "\
fn foo(x: Nat) -> Nat {\n\
    let y = x else { return 0 };    // warning: irrefutable pattern\n\
\n\
    // Fix: use a plain let\n\
    let y = x;\n\
    y\n\
}",
            see_also: &["LetElseNonDiverging"],
        },

        "IfLetIrrefutable" => ErrorExplanation {
            name: "IfLetIrrefutable",
            code: "W0004",
            category: "Control Flow",
            summary: "irrefutable pattern in if let",
            detail: "\
The pattern in an `if let` expression is irrefutable — it always \
matches, so the condition is always true.\n\
\n\
Use a plain `let` binding and unconditional block instead.",
            example: "\
fn foo(x: Nat) -> Nat {\n\
    if let y = x { y }    // warning: irrefutable pattern\n\
    else { 0 }\n\
\n\
    // Fix: use a plain let\n\
    let y = x;\n\
    y\n\
}",
            see_also: &["LetElseIrrefutable"],
        },

        "ReturnOutsideFunction" => ErrorExplanation {
            name: "ReturnOutsideFunction",
            code: "E0048",
            category: "Control Flow",
            summary: "return with no function return type in scope",
            detail: "\
`return` needs an enclosing function whose return type it can check \
against, and no function body is in scope here.\n\
\n\
Common causes:\n\
• `return` in a theorem or lemma body — proof terms are expressions, \
not function bodies\n\
• `return` in any other non-function elaboration context",
            example: "\
theorem t : Nat = return 5    // error: no function to return from\n\
\n\
// Fix: a proof term is an expression — write the value itself\n\
theorem t : Nat = 5",
            see_also: &["TryOutsideReturnContext", "ReturnInsideTryBlock"],
        },

        "ReturnInsideTryBlock" => ErrorExplanation {
            name: "ReturnInsideTryBlock",
            code: "E0044",
            category: "Control Flow",
            summary: "return inside a try block",
            detail: "\
A `try` block evaluates to a `Result`; a `return` inside it would leave the \
block without producing one, and would return from the *enclosing function* \
rather than from the block — almost never what was meant.\n\
\n\
Propagate with `?` instead: it exits the block with the error, which is the \
behaviour `return` looks like it is asking for.",
            example: "\
let r = try {\n\
    let x = step()?;      // fine: `?` exits the block\n\
    return x;             // error: `return` would exit the function\n\
};",
            see_also: &["TryOutsideReturnContext", "TryReturnMismatch"],
        },

        "TryBlockRequiresResultType" => ErrorExplanation {
            name: "TryBlockRequiresResultType",
            code: "E0045",
            category: "Control Flow",
            summary: "try block has no Result type in scope",
            detail: "\
A `try` block produces a `Result<T, E>`, so a `Result` type must be resolvable \
where the block appears — and which `Result`, with which error type, cannot be \
inferred from the block alone.\n\
\n\
Add a type annotation on the binding, or import the `Result` the block should \
produce.",
            example: "\
let r: Result<Nat, Error> = try { step()? };   // fine\n\
let r = try { step()? };                       // error: which Result?",
            see_also: &["TryBlockExpectedSumEncoding", "CannotInferType"],
        },

        "TryBlockExpectedSumEncoding" => ErrorExplanation {
            name: "TryBlockExpectedSumEncoding",
            code: "E0046",
            category: "Control Flow",
            summary: "try block's Result type is not a two-constructor sum",
            detail: "\
The desugaring builds and matches the Ok/Err arms directly, so the annotated \
type must encode as a Sum — a two-constructor ADT. A record, an alias to a \
non-sum, or a type with the wrong constructor count cannot carry the block's \
result.\n\
\n\
Inspect the encoding with `albite info type adt <name> <file>` rather than \
guessing: the Sum shape is not visible in the source spelling.",
            example: "\
type Result<T, E> = Ok(T) | Err(E)    // fine: two constructors -> Sum\n\
type Result<T, E> = { ok: T }         // error: a record has no Sum encoding",
            see_also: &["TryBlockMissingConstructor", "TryOnNonTryType"],
        },

        "TryBlockMissingConstructor" => ErrorExplanation {
            name: "TryBlockMissingConstructor",
            code: "E0047",
            category: "Control Flow",
            summary: "try block's Result type lacks a required constructor",
            detail: "\
The desugaring names the constructors it builds — the message says which one \
is absent. A `Result` shaped correctly but spelled differently (`Success`/\
`Failure`) will not do: the names are what the generated code references.\n\
\n\
Confirm the constructors and their source order with `albite info type \
constructors <name> <file>`; order matters, since index 0 maps to left/inl.",
            example: "\
type Result<T, E> = Ok(T) | Err(E)          // fine\n\
type Result<T, E> = Good(T) | Bad(E)        // error: no `Ok` constructor",
            see_also: &["TryBlockExpectedSumEncoding", "UndefinedConstructor"],
        },

        // Regions (ADR 9.10.26o D4) — the self-host's E0730–E0732.
        "NotFirstOrder" => ErrorExplanation {
            name: "NotFirstOrder",
            code: "E0094",
            category: "Regions",
            summary: "a region's value, or an exit crossing it, is not first-order data",
            detail: "\
`region { e }` frees everything allocated inside it when it ends, so its value \
must be data that can be carried out: `Nat`, `Int`, `Bool`, `String`, `Unit`, \
or a tuple, sum or type built only from those. A closure, a reference, a type \
variable or a proof is refused, because it could point into the freed memory. \
A `return` or `?` inside the body that leaves the region carries a value out \
too, so the enclosing function's return type (or the `try` block's `Result`) \
is held to the same rule. A `return` inside a closure inside the region exits \
the closure, not the region.\n\
\n\
In this release the compiled program also needs the value to be a `Nat`, \
`Int`, `Bool` or `Unit`; `albite check` accepts the rest and `albite \
compile` refuses them until the copy-out lands.\n\
\n\
The self-hosted compiler reports this as E0730.",
            example: "\
fn f() -> Nat {\n\
    let g = region { |x: Nat| x };   // error: a closure is not first-order\n\
    g(1)\n\
}",
            see_also: &["ReachesOutside", "RegionInProof"],
        },

        "ReachesOutside" => ErrorExplanation {
            name: "ReachesOutside",
            code: "E0095",
            category: "Regions",
            summary: "a region body reaches a resource handle or an outer type variable",
            detail: "\
A region body may not name a `StringBuilder` or `Bytes` handle bound outside \
it, a function whose declared type mentions a handle (`push_str`, \
`bytes_new`, ...), or a handle constructor: a handle's buffer lives outside \
every region, and keeping handles out is defence in depth. Nor may it name a \
variable bound outside it whose type holds a type parameter of the enclosing \
definition, since that value's representation is unknown there. A generic \
top-level function such as `map` is fine: its type variables are \
instantiated at the use.\n\
\n\
The self-hosted compiler reports this as E0731.",
            example: "\
fn f(n: Nat) -> Nat {\n\
    region { bytes_len(bytes_new(n)) }   // error: reaches `bytes_len`\n\
}",
            see_also: &["NotFirstOrder", "CapturedByClosure"],
        },

        "RegionInProof" => ErrorExplanation {
            name: "RegionInProof",
            code: "E0096",
            category: "Regions",
            summary: "a `region` in a theorem or lemma body",
            detail: "\
A region is a run-time instruction about memory; a proof is never run, so a \
`region` in a `theorem` or `lemma` body has no meaning there and is refused. \
The kernel refuses one that reaches a proof term by any other route.\n\
\n\
The self-hosted compiler reports this as E0732.",
            example: "\
theorem t(): Eq<Nat, 1, 1> { region { refl } }   // error",
            see_also: &["NotFirstOrder"],
        },

        _ => return None,
    };
    Some(exp)
}

#[cfg(test)]
mod tests {
    use crate::explain::error_catalogue;

    // 9.10.26o AC3: E0094–E0096 resolve by code and by name to their entries,
    // each naming its self-host counterpart.
    #[test]
    fn the_region_codes_resolve_by_code_and_by_name() {
        for (kind, code, self_hosted) in [
            ("NotFirstOrder", "E0094", "E0730"),
            ("ReachesOutside", "E0095", "E0731"),
            ("RegionInProof", "E0096", "E0732"),
        ] {
            assert_eq!(error_catalogue::resolve_query_for_test(code), Some(kind));
            assert_eq!(error_catalogue::resolve_query_for_test(kind), Some(kind));
            let (detail, _, _) = error_catalogue::entry_body_for_test(kind).expect("an entry");
            assert!(detail.contains(self_hosted), "{detail}");
        }
    }
}
