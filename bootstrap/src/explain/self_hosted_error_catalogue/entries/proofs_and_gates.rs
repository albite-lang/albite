//! Proofs (E0400-E0499), references (E0500-E0599), entry point and control
//! flow (E0600-E0699) and the soundness gates (E0700-E0799).

use super::SelfHostedErrorEntry;

pub(super) const PROOFS_AND_GATES: &[SelfHostedErrorEntry] = &[
    // Proof errors (E0400-E0499)
    SelfHostedErrorEntry {
        code: "E0400",
        name: "ErrProofRequired",
        category: "Proofs",
        description: "Expected a proof term.",
    },
    SelfHostedErrorEntry {
        code: "E0401",
        name: "ErrSorryInProd",
        category: "Proofs",
        description: "`sorry` used in production code. `sorry` is only allowed during development.",
    },
    SelfHostedErrorEntry {
        code: "E0402",
        name: "ErrInvalidRefl",
        category: "Proofs",
        description: "`refl` used on non-equal types. Reflexivity requires both sides of the equality to be the same type.",
    },
    SelfHostedErrorEntry {
        code: "E0403",
        name: "ErrSubstExpectedEquality",
        category: "Proofs",
        description: "`subst` given an argument that is not an equality proof.",
    },
    SelfHostedErrorEntry {
        code: "E0404",
        name: "ErrTransEndpointMismatch",
        category: "Proofs",
        description: "`trans` given two equalities whose middle endpoints differ.",
    },
    SelfHostedErrorEntry {
        code: "E0405",
        name: "ErrCongExpectedFunction",
        category: "Proofs",
        description: "`cong` given a first argument that is not a function.",
    },
    SelfHostedErrorEntry {
        code: "E0475",
        name: "ErrMotiveNotPredicate",
        category: "Proofs",
        description: "A `subst` motive that is not a predicate lambda.",
    },
    SelfHostedErrorEntry {
        code: "E0476",
        name: "ErrMotiveDomainMismatch",
        category: "Proofs",
        description: "A motive whose parameter type differs from the equality's base type.",
    },
    SelfHostedErrorEntry {
        code: "E0477",
        name: "ErrMotiveBodyNotType",
        category: "Proofs",
        description: "A motive whose body is a term rather than a type.",
    },
    // Reference/mutability errors (E0500-E0599)
    SelfHostedErrorEntry {
        code: "E0500",
        name: "ErrRefInPureContext",
        category: "References",
        description: "Reference used in a pure context. References are not allowed in pure functional code.",
    },
    SelfHostedErrorEntry {
        code: "E0501",
        name: "ErrCannotMutate",
        category: "References",
        description: "Trying to mutate an immutable binding.",
    },
    // Expression errors (E0600-E0699)
    SelfHostedErrorEntry {
        code: "E0600",
        name: "ErrNoReturnContext",
        category: "Control Flow",
        description: "`return` or `?` used outside a function body.",
    },
    SelfHostedErrorEntry {
        code: "E0601",
        name: "ErrNoMainFunction",
        category: "Entry Point",
        description: "No `main` function found. Executable files must define a `main` function.",
    },
    SelfHostedErrorEntry {
        code: "E0602",
        name: "ErrInvalidMainSig",
        category: "Entry Point",
        description: "`main` function has an invalid signature. It should take no arguments and return a supported type.",
    },
    SelfHostedErrorEntry {
        code: "E0603",
        name: "ErrReturnInTry",
        category: "Control Flow",
        description: "`return` inside a `try` block. Use `?` to propagate the error instead.",
    },
    // Soundness gates (E0700-E0799): positivity takes E0700-E0709, the
    // termination mirror starts at E0710 (ADR 18.8.26b D4).
    SelfHostedErrorEntry {
        code: "E0700",
        name: "ErrNonStrictlyPositive",
        category: "Soundness",
        description: "A type definition is not strictly positive: one of its own recursive group is reachable from a forbidden position -- left of an arrow at any depth, inside a `Ref`/`Ptr` cell, or in an `Eq` witness. Strict positivity is what makes `structural subterm` well-founded, so without it a diverging term exists at any type with no syntactic recursion for the termination checker to inspect. The rule is the same one the bootstrap raises E0061 for -- both compilers call one shared checker -- so the fix is the same: move the occurrence out of the argument position, storing the result of the function rather than the function itself. There is no annotation that admits it.",
    },
    SelfHostedErrorEntry {
        code: "E0710",
        name: "ErrCannotProveTermination",
        category: "Soundness",
        description: "A recursive definition could not be certified as terminating: no argument it passes to itself is a strict structural subterm of the caller's own decreasing parameter. Without termination a non-terminating definition inhabits every type -- including the empty one -- so every proof the compiler accepts would be contingent on the author not having written a loop. The rule is the same one the bootstrap raises E0062 for; both compilers call one shared checker, so the fix is the same: descend on a subterm of the argument rather than rebuilding it, name the parameter with `#[decreasing(arg)]` when the search is ambiguous, or opt the definition out with `#[partial]` -- which is fine in executable code and inadmissible in a proof (see E0711).",
    },
    SelfHostedErrorEntry {
        code: "E0711",
        name: "ErrPartialInProof",
        category: "Soundness",
        description: "A theorem, lemma or axiom reaches a `#[partial]` definition, directly or through a chain of ordinary ones. `#[partial]` opts a definition out of termination certification, and the opt-out taints transitively: a proof that depends on one is no longer a proof, because the partial definition may not terminate and so inhabits its type vacuously. Wrapping the partial definition in another one does not launder it -- admission rejects references to *tainted* constants, not only to annotated ones. The rule is the same one the bootstrap raises E0063 for. The fix is to remove the dependency, not to annotate the proof.",
    },
    SelfHostedErrorEntry {
        code: "E0720",
        name: "ErrHandleUsedAfterConsumed",
        category: "Soundness",
        description: "A `StringBuilder` or `Bytes` handle is used after it was handed over (ADR 7.10.26t). Threaders such as `push_str` and `bytes_set` hand the buffer to their result, and `to_string` / `bytes_into_string` hand it out as the `String`; the old handle is dead, and a later call on it aborts at run time. Each handle variable may be consumed once on every path: any call consumes it except an observer (`string_builder_len`, `bytes_len`, `bytes_get`) before the first consuming use, and the arms of an `if` or `match` are alternatives. The bootstrap raises E0092 for the same program, at the same use. The fix is to thread the successor: `let sb = push_str(sb, s);`.",
    },
    SelfHostedErrorEntry {
        code: "E0721",
        name: "ErrHandleCapturedByClosure",
        category: "Soundness",
        description: "A closure mentions a `StringBuilder` or `Bytes` handle bound outside it (ADR 7.10.26t). A closure can run any number of times, so it could consume the handle twice. The bootstrap raises E0093 for the same program, at the same use. The fix is to pass the handle to the closure as an argument and thread its result.",
    },
    SelfHostedErrorEntry {
        code: "E0730",
        name: "ErrRegionNotFirstOrder",
        category: "Soundness",
        description: "A `region { e }`'s value, or a `return` or `?` that leaves the region, has a type that is not first-order data (ADR 9.10.26o). A region frees everything allocated inside it when it ends, so only `Nat`, `Int`, `Bool`, `String`, `Unit` and tuples, sums and types built from them may leave it; a closure, a reference, a type variable or a proof could point into the freed memory. The bootstrap raises E0094 for the same program. The fix is to return data, not a function, from the region.",
    },
    SelfHostedErrorEntry {
        code: "E0731",
        name: "ErrRegionReachesOutside",
        category: "Soundness",
        description: "A region body names a `StringBuilder` or `Bytes` handle bound outside it, a function whose declared type mentions a handle, a handle constructor, or a variable bound outside it whose type holds a type parameter of the enclosing definition (ADR 9.10.26o). A handle's buffer lives outside every region, and an outer value of unknown representation cannot be reasoned about there. The bootstrap raises E0095 for the same program. The fix is to do the handle work outside the region.",
    },
    SelfHostedErrorEntry {
        code: "E0732",
        name: "ErrRegionInProof",
        category: "Soundness",
        description: "A `region` sits in a `theorem` or `lemma` body (ADR 9.10.26o). A proof is never run, so a memory instruction has no meaning in it. The bootstrap raises E0096 for the same program. The fix is to remove the region from the proof.",
    },
    SelfHostedErrorEntry {
        code: "E0740",
        name: "ErrAssignmentRefused",
        category: "Soundness",
        description: "An assignment `x = e;` is refused (ADR 13.10.26r): the innermost `x` is not a `let mut` binding (a parameter, a plain `let`, a pattern variable or a global), or the assignment targets a binding declared outside its block and that block is not an arm of an `if`, `match`, `if let` or block in a join position, or the arm can leave through `return` or `?`. Its effect would be lost to later code. The bootstrap raises E0104 for the same program, at the same assignment.",
    },
    SelfHostedErrorEntry {
        code: "E0741",
        name: "ErrMutableCaptured",
        category: "Soundness",
        description: "A closure reads or assigns a `let mut` binding declared outside it (ADR 13.10.26r, M3). An assignment is a rebinding, so a closure would see a stale value. The bootstrap raises E0105 for the same program, at the same mention. The fix is to copy the value into an immutable `let` first.",
    },
    SelfHostedErrorEntry {
        code: "E0742",
        name: "ErrMutableAssignedInRegion",
        category: "Soundness",
        description: "A `region` body assigns a `let mut` binding declared outside it (ADR 13.10.26r, M6). The value would outlive the region where the region's result check does not see it. The bootstrap raises E0106 for the same program. The fix is to return the value from the region and assign it outside.",
    },
];
