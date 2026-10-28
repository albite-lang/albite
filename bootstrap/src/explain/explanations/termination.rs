//! Explanations for the termination gate's error codes (ADR 29.6.26e).

use crate::explain::error_catalogue::ErrorExplanation;

/// `albite explain error E0062` / `E0063`.
pub(super) fn termination(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "CannotProveTermination" => ErrorExplanation {
            name: "CannotProveTermination",
            code: "E0062",
            category: "Termination",
            summary: "recursion is not structurally decreasing",
            detail: "\
A non-terminating function inhabits every type, including the empty one, so a \
recursive definition is admitted only once the compiler can see that it stops. \
Phase 1 checks the structural rule: some parameter must get **strictly \
smaller** at every recursive call.\n\
\n\
\"Strictly smaller\" means bound by taking a value apart, not built back up. \
Matching `l` against `Cons(h, t)` proves `t` smaller than `l`; `Cons(0, t)` is \
not smaller than `l`, even though `t` is. Aliases and projections of a smaller \
value stay smaller; rebinding the name loses the fact.\n\
\n\
For mutual recursion the decrease is measured against the **caller's** \
parameter. It is not enough for each call to fill the callee's decreasing \
position — a group that passes its argument round unchanged occupies every \
position correctly and never makes progress.\n\
\n\
The recursion also has to be visible as a call. `let g = f; g(x)` recurses \
through a value, and there is no call site to inspect, so it is rejected \
rather than assumed fine.\n\
\n\
When more than one parameter decreases at every call, the compiler will not \
guess: write `#[decreasing(arg)]`. When the recursion genuinely is not \
structural — an accumulator, a measure, a division — Phase 1 cannot certify \
it. Mark it `#[partial]` and it is admitted as an opaque constant that proofs \
may not use (see PartialInProof).",
            example: "\
fn len(l: List) -> Nat {                     // fine: `t` is smaller than `l`\n\
    match l { Nil => 0, Cons(h, t) => 1 + len(t) }\n\
}\n\
\n\
fn spin(l: List) -> Nat { spin(l) }          // error: `l` never shrinks\n\
\n\
#[partial]\n\
fn collatz(n: Nat) -> Nat { … }              // opt out; unusable in proofs",
            see_also: &["PartialInProof", "NonStrictlyPositive"],
        },

        // `ForeignInProof` shares E0063 and this page (ADR 3.10.26c D3): a
        // second E0063 page could never be reached by code, since a code
        // resolves to the first kind carrying it.
        "PartialInProof" | "ForeignInProof" => ErrorExplanation {
            name: "PartialInProof",
            code: "E0063",
            category: "Termination",
            summary: "a proof depends on a partial definition",
            detail: "\
`#[partial]` buys executable code an exemption from termination checking. It \
cannot buy a proof one: a proof that may reference a possibly-diverging \
constant proves nothing.\n\
\n\
The restriction is transitive, which is the point. A wrapper that merely calls \
a partial function is itself partial, so routing a theorem through one more \
layer does not launder it. The message names the chain it travelled.\n\
\n\
A definition's **type** counts as well as its body — an equality statement \
embeds terms, so a theorem can reach a partial constant without its proof term \
mentioning one.\n\
\n\
The fix is to prove the recursion terminates (rewrite it structurally, or name \
the decreasing parameter), or to state the theorem about a definition that is \
already total. Executable code is unaffected: a partial constant is welcome \
anywhere outside a proof.\n\
\n\
A call to **foreign code** is treated the same way. The checker never runs an \
`extern \"C\"` function, so it would accept `g() == g()` by `refl` while the \
linked program returns two different values. A definition that calls an \
extern is therefore tainted exactly as a `#[partial]` one is, and so is \
everything that reaches it; the message names the extern beside the chain. \
The only exemption is the evaluator's `pure` externs, which the checker does \
run deterministically — `albite info eval externs` lists them as `[pure]`. \
There is no opt-out: state the theorem about something that does not reach \
the call.",
            example: "\
#[partial]\n\
fn search(k: Nat) -> Nat { … }\n\
\n\
fn wrapper(k: Nat) -> Nat { search(k) }   // fine — and now partial too\n\
\n\
theorem found(k: Nat) : Eq Nat (wrapper(k)) k = refl\n\
//      error: reaches `search` (marked #[partial]) through wrapper\n\
\n\
extern \"C\" fn tg_type_nat() -> Nat\n\
\n\
theorem handles_equal(): tg_type_nat() == tg_type_nat() { refl }\n\
//      error: depends on `tg_type_nat`, which reaches foreign code",
            see_also: &["CannotProveTermination", "ContainsSorry"],
        },

        // Affine resource handles (ADR 7.10.26t D7) — the self-host's E0720.
        "UsedAfterConsumed" => ErrorExplanation {
            name: "UsedAfterConsumed",
            code: "E0092",
            category: "Affine Handles",
            summary: "a `StringBuilder` or `Bytes` used after it was handed over",
            detail: "\
`StringBuilder` and `Bytes` are runtime handles: `push_str`, `bytes_set` and \
their kin hand the buffer to their result, and `to_string` / \
`bytes_into_string` hand it out as the `String` without copying. The old \
handle is dead after that, and a later call on it aborts the program. So \
each handle variable may be consumed once on every path through a function.\n\
\n\
Passing a handle to any function consumes it, except an observer \
(`string_builder_len`, `bytes_len`, `bytes_get`) before the first consuming \
use. `let b = a`, putting `a` in a tuple or constructor, and `match a` move \
it. The arms of an `if` or `match` are alternatives: one use in each is one \
use. Thread the successor instead — `let sb = push_str(sb, s);` shadows the \
dead name with the live one.\n\
\n\
The self-hosted compiler reports this as E0720.",
            example: "\
fn twice(sb: StringBuilder) -> String {\n\
    let a = push_str(sb, \"x\");\n\
    let b = push_str(sb, \"y\");   // error: `sb` was consumed above\n\
    to_string(a) ++ to_string(b)\n\
}",
            see_also: &["CapturedByClosure"],
        },

        "CapturedByClosure" => ErrorExplanation {
            name: "CapturedByClosure",
            code: "E0093",
            category: "Affine Handles",
            summary: "a closure mentions a `StringBuilder` or `Bytes` bound outside it",
            detail: "\
A closure can run any number of times, so one that mentions a handle bound \
outside it could consume that handle twice. A handle may not be captured: \
pass it to the closure as an argument instead, and thread the result.\n\
\n\
The self-hosted compiler reports this as E0721.",
            example: "\
fn deferred(sb: StringBuilder) -> String {\n\
    let finish = |s: String| to_string(push_str(sb, s));   // error: captures `sb`\n\
    finish(\"!\")\n\
}",
            see_also: &["UsedAfterConsumed"],
        },

        _ => return None,
    };
    Some(exp)
}

/// `albite explain error E0104` / `E0105` / `E0106`: the refusals of `let
/// mut` and assignment (ADR 13.10.26r D4). Kept beside the termination gate's
/// for room; the self-host's are E0740–E0742.
pub(super) fn mutable_bindings(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "NotMut" => ErrorExplanation {
            name: "NotMut",
            code: "E0104",
            category: "Mutable Bindings",
            summary: "an assignment to a binding that is not `let mut`",
            detail: "\
`x = e;` gives a `let mut` binding a new value, and is a statement, never an \
expression. It is refused when the innermost binding of `x` is a parameter, a \
plain `let`, a pattern variable or a global. A plain `let x` that shadows a \
`let mut x` makes `x` immutable from there on. E0104's other reason is \
`NotJoined`.\n\
\n\
`let mut` was added in B4.2; until then E0102 refused every `let mut`, and is \
retired. The self-hosted compiler reports this as E0740.",
            example: "\
fn f(p: Nat) -> Nat {\n\
    p = 1;      // error: `p` is a parameter\n\
    let mut x = p;\n\
    x = 2;      // fine\n\
    x\n\
}",
            see_also: &["NotJoined", "Captured", "InRegion"],
        },

        "NotJoined" => ErrorExplanation {
            name: "NotJoined",
            code: "E0104",
            category: "Mutable Bindings",
            summary: "an assignment no join hands back to later code",
            detail: "\
An assignment to a binding declared outside the block that holds it reaches \
later code only when that block is an arm of an `if`, `match` or `if let`, or \
a block, standing as a statement, a `let`'s value or a block's tail: the \
construct hands the arm's value back where the arms rejoin. A block passed as \
a call argument is not one, and neither is a `try` body or a `let ... else` \
block, so the assignment would be lost and is refused. An arm that assigns an \
outer binding also may not leave through `return` or `?`, since it could not \
hand the value back.\n\
\n\
The self-hosted compiler reports this as E0740.",
            example: "\
fn f(c: Bool) -> Nat {\n\
    let mut x = 1;\n\
    if c { x = 2; };            // fine: the `if` hands `x` back\n\
    let y = id({ x = 3; 4 });   // error: not joined\n\
    x + y\n\
}",
            see_also: &["NotMut"],
        },

        "Captured" => ErrorExplanation {
            name: "Captured",
            code: "E0105",
            category: "Mutable Bindings",
            summary: "a closure mentions a `let mut` binding declared outside it",
            detail: "\
A closure may not read or assign a `let mut` binding declared outside it: \
an assignment is a rebinding, so a closure would see a stale value, and \
there is no `&mut` in v2.0. A closure's own `let mut` is fine, and so is a \
closure that mentions an immutable `let` that shadows a `mut` one — copy the \
value into one first.\n\
\n\
The self-hosted compiler reports this as E0741.",
            example: "\
fn f() -> Nat {\n\
    let mut x = 1;\n\
    let g = |y: Nat| x + y;   // error: mentions `x`\n\
    g(1)\n\
}",
            see_also: &["NotMut", "CapturedByClosure"],
        },

        "InRegion" => ErrorExplanation {
            name: "InRegion",
            code: "E0106",
            category: "Mutable Bindings",
            summary: "a region body assigns a `let mut` binding declared outside it",
            detail: "\
A `region { ... }` frees what it allocated when it ends. A value assigned \
inside it to a binding declared outside it would outlive the region where \
the region's result check does not see it, so the assignment is refused. A \
binding declared inside the region may be assigned, and an outer one may be \
read.\n\
\n\
The self-hosted compiler reports this as E0742.",
            example: "\
fn f() -> Nat {\n\
    let mut x = 1;\n\
    region { x = 5; 1 }   // error\n\
}",
            see_also: &["NotMut", "NotFirstOrder"],
        },

        _ => return None,
    };
    Some(exp)
}

#[cfg(test)]
mod mutable_binding_tests {
    use crate::explain::error_catalogue;

    // 13.10.26r AC5: E0104–E0106 resolve by code and by name to their entries,
    // each naming its self-host counterpart; E0102 is retired.
    #[test]
    fn the_mutable_binding_codes_resolve_by_code_and_by_name() {
        for (kind, code, self_hosted) in [
            ("NotMut", "E0104", "E0740"),
            ("Captured", "E0105", "E0741"),
            ("InRegion", "E0106", "E0742"),
        ] {
            assert_eq!(error_catalogue::resolve_query_for_test(code), Some(kind));
            assert_eq!(error_catalogue::resolve_query_for_test(kind), Some(kind));
            let (detail, _, _) = error_catalogue::entry_body_for_test(kind).expect("an entry");
            assert!(detail.contains(self_hosted), "{detail}");
        }
        assert_eq!(
            error_catalogue::resolve_query_for_test("NotJoined"),
            Some("NotJoined")
        );
        assert_eq!(error_catalogue::resolve_query_for_test("E0102"), None);
    }
}
