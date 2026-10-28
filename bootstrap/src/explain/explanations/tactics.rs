//! Explanations for the kernel boundary and tactic blocks, E0081–E0083
//! (ADR 2.10.26c D8), the kernel exemption register's E0084 (ADR 2.10.26d),
//! `rewrite`'s E0085 (ADR 2.10.26e D7), `simp`'s E0086 (ADR 4.10.26e D3) and
//! `ring`'s E0087 (ADR 4.10.26g D6) and `omega`'s E0088 (ADR 6.10.26f D7).

use crate::explain::error_catalogue::ErrorExplanation;

/// `albite explain error E0081` / `E0082` / `E0083` / `E0084`.
pub(super) fn tactics(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "KernelRefusedProof" => ErrorExplanation {
            name: "KernelRefusedProof",
            code: "E0081",
            category: "Tactics and the Kernel",
            summary: "the kernel refused a proof",
            detail: "\
Every proof a `by` block builds is handed to the kernel's proof judgment, \
`check_proof`, once the whole project's termination verdicts are known. The \
kernel is the small trusted checker: `intro` and `apply` assemble Core the \
elaborator never checked as a whole, so the kernel's verdict is the one that \
counts, and a refusal is always an error, never a warning.\n\
\n\
The message names the tactic whose output was refused — the last step whose \
own piece the kernel rejected — and the kernel's reason. When no single step \
is at fault, it names the `by` block.\n\
\n\
Common causes:\n\
• A `refl` that holds only by unfolding a definition. The kernel does not \
unfold definitions yet (δ), so `two() == 2 by { refl }` is refused even \
though the elaborator accepts it; that waits on kernel δ.\n\
• Citing a `#[partial]` or rejected definition, which a proof may not use.\n\
• A step whose piece is ill-formed in proof mode (`return`, an unrestricted \
`fix`, a negative recursive type).\n\
\n\
A `sorry` in an `exact` is not a refusal: that goal is set aside as a \
hypothesis and the rest of the proof is still checked.\n\
\n\
A term-mode `theorem` or `lemma` is checked too (ADR 2.10.26d): its message \
names the refusal's class (`needs-delta`, `named-type`, `self-reference`) or \
the census's cause, then the kernel's error. Within this repository a refused \
theorem may be listed in the shrink-only exemption register, \
`bootstrap/kernel_exemptions.toml`; `albite check` then reports it as \
kernel-exempt instead. No command-line switch turns the check off.\n\
\n\
The cause after the colon is the kernel's own error — read it there. \
`albite doctor check proof kernel-census <file>` sorts kernel refusals by \
cause, but only over a file that elaborates: it runs the same boundary, so on \
a file with an E0081 it stops at this error too.",
            example: "\
fn two() -> Nat { 2 }\n\
\n\
theorem two_is_two(): two() == 2 by { refl }   // E0081: needs δ in the kernel",
            see_also: &["TacticNotApplicable", "UnsolvedGoals", "PartialInProof"],
        },

        "TacticNotApplicable" => tactic_not_applicable(),

        "UnsolvedGoals" => ErrorExplanation {
            name: "UnsolvedGoals",
            code: "E0083",
            category: "Tactics and the Kernel",
            summary: "a `by` block ends with goals open",
            detail: "\
A `by` block must close every goal it opens. `intro` replaces a goal with its \
body, and `apply` replaces it with one goal per premise; the block's tactics \
must close each of those in turn, first goal first.\n\
\n\
The message lists every open goal as `locals ⊢ target`.",
            example: "\
theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> by { intro h }   // E0083\n\
\n\
// Fix: close the goal `h : Eq<Nat, n, 0> ⊢ Eq<Nat, n, 0>`\n\
theorem t(n: Nat) -> Eq<Nat, n, 0> -> Eq<Nat, n, 0> by { intro h; exact h }",
            see_also: &["TacticNotApplicable"],
        },

        "KernelExemptionViolation" => ErrorExplanation {
            name: "KernelExemptionViolation",
            code: "E0084",
            category: "Tactics and the Kernel",
            summary: "a kernel exemption row the build contradicts",
            detail: "\
A few theorems in this repository are refused by the kernel for a reason it \
cannot yet handle, and are listed by name in its exemption register, \
`bootstrap/kernel_exemptions.toml`, each with its class and the ADR that will \
remove the row. The register only shrinks, and it must say exactly what the \
build observes. This error is a row that does not:\n\
\n\
• The kernel now admits a theorem whose row says `needs-delta`: the row is \
stale. Remove it and lower `KERNEL_EXEMPTION_CEILING` in the same change.\n\
• The kernel refuses a theorem for a different class than its row says.\n\
• An admitted theorem cites an exempt one: the kernel never checked what it \
leans on, so it needs a `cites-exempt` row listing exactly those names — or a \
`cites-exempt` row lists names that are no longer exempt.\n\
\n\
Only files in this repository can match a row, so a user's project never \
meets this error; a theorem there that the kernel refuses is E0081.",
            example: "\
# kernel_exemptions.toml — the kernel admits `two_plain`, so this row is stale\n\
[[exemption]]\n\
file = \"tests/golden/check/conversion_unfolds_admitted_globals.tg\"\n\
theorem = \"two_plain\"\n\
cause = \"needs-delta\"\n\
removed_by = \"2.10.26l\"",
            see_also: &["KernelRefusedProof"],
        },

        _ => return None,
    };
    Some(exp)
}

/// E0082, a sibling of [`tactics`] so each tactic's sentence has room: one
/// bullet per tactic that can refuse a goal, `cases` included (ADR 3.10.26f
/// D5).
fn tactic_not_applicable() -> ErrorExplanation {
    ErrorExplanation {
        name: "TacticNotApplicable",
        code: "E0082",
        category: "Tactics and the Kernel",
        summary: "a tactic cannot act on the goal",
        detail: "\
A tactic was used on a goal it cannot act on, or after every goal was closed.\n\
\n\
• `intro x` needs a function goal (`A -> B`), and a name not already bound — \
shadowing a parameter is refused rather than renamed.\n\
• `apply e` matches `e`'s conclusion against the goal syntactically: no \
computation, so `0 + n` never matches `n * 1`. Every `Π` variable must be \
fixed by that match, each remaining premise must be a proposition (it \
becomes a new goal), and the lemma may not be generic. A `sorry` inside the \
applied term is refused.\n\
• `rewrite [r]` needs an equation goal. A bare lemma name as a rule has each \
`Π` variable fixed by matching its side against the goal, so every variable \
must occur in that side, the side may not be a bare variable, and the lemma \
may have no premise and no type parameters; pass explicit arguments instead.\n\
• `simp [r, …]` refuses a goal no rule in its list occurs in, and a bare \
lemma whose sides only swap its variables (`add_comm`): it would rewrite \
forever, so give its arguments or use `rewrite`. Each rule is also refused \
as `rewrite [r]` refuses one.\n\
• `induction x` needs `x` to be a `Nat` variable in scope, and no other \
hypothesis may mention `x`.\n\
• `cases x { … }` needs `x` to be a local of a user data type — not a \
global, an expression, a `Nat` (split it by `induction`), a `Bool` or a \
record — and one arm per constructor, each naming it once with one binder \
per declared field; a binder may not reuse a name already bound.\n\
• `ring` needs an equation over `Nat` and the `nat_arith` library in scope \
(`mod nat_arith;`), whose lemmas it cites.\n\
• `omega` needs a `Le`-shaped goal (`exists k: Nat. s + k == t`) or an \
equation over `Nat`, and `mod nat_arith;` in scope.\n\
• Any tactic after the last goal is closed has nothing to act on.",
        example: "\
theorem t(n: Nat) -> Eq<Nat, n, n> by { intro h; refl }   // E0082: not a function goal\n\
\n\
// Fix: the goal is already an equation\n\
theorem t(n: Nat) -> Eq<Nat, n, n> by { refl }",
        see_also: &["UnsolvedGoals", "KernelRefusedProof"],
    }
}

/// `albite explain error E0085` — `rewrite`'s own code (ADR 2.10.26e D7).
pub(super) fn rewrite(name: &str) -> Option<ErrorExplanation> {
    (name == "RewriteNoOccurrence").then_some(ErrorExplanation {
        name: "RewriteNoOccurrence",
        code: "E0085",
        category: "Tactics and the Kernel",
        summary: "a `rewrite` rule rewrites nothing",
        detail: "\
`rewrite [e]` with `e : Eq<T, a, b>` replaces every `a` in the goal by `b`; \
`rewrite [<- e]` replaces every `b` by `a`. This error is a rule whose side \
does not occur in the goal, so it would rewrite nothing.\n\
\n\
The search is syntactic: no computation, and nothing under a binder (a \
lambda, a `let`, a `match` arm), so `0 + n` is not found in `n`. A bare lemma \
name is instantiated at its first match, outermost and leftmost first.\n\
\n\
When the rule's other side occurs, the message asks whether you meant the \
other direction — add or drop the `<-`. A rule whose two sides are the same \
term is refused here too: it can only transport the goal to itself.",
        example: "\
theorem t(n: Nat, j: Nat, ih: Eq<Nat, n + j, j + n>) -> Eq<Nat, j + n, j + n> by {\n\
    rewrite [ih];   // E0085: `n + j` does not occur; did you mean `<-`?\n\
    refl\n\
}\n\
\n\
// Fix: rewrite right to left\n\
    rewrite [<- ih];",
        see_also: &["TacticNotApplicable", "SubstExpectedEquality"],
    })
}

/// `albite explain error E0086` — `simp`'s own code (ADR 4.10.26e D3).
pub(super) fn simp(name: &str) -> Option<ErrorExplanation> {
    (name == "SimpStepBound").then_some(ErrorExplanation {
        name: "SimpStepBound",
        code: "E0086",
        category: "Tactics and the Kernel",
        summary: "`simp` reached its step bound",
        detail: "\
`simp [r₁, …, rₙ]` rewrites by the first rule that occurs in the goal, again \
and again, until none does, then closes the goal by `refl` if it can. Each \
rewrite is one step, and one call may take at most 64. This error is a call \
that has taken 64 steps with a rule still applying.\n\
\n\
The usual cause is two rules that undo each other — `add_comm(a, b)` beside \
`add_comm(b, a)` — so the goal cycles. The message lists the last rules \
applied, by their position in the list, which is where the cycle shows. \
Drop one of the pair, or write the steps out with `rewrite`.\n\
\n\
A bare lemma whose sides only swap its variables, such as `add_comm`, is \
refused before any step, with E0082, for the same reason.",
        example: "\
theorem t(a: Nat, b: Nat, c: Nat) -> Eq<Nat, a + b, c> by {\n\
    simp [add_comm(a, b), add_comm(b, a)]   // E0086: rules 1, 2, 1, 2 …\n\
}",
        see_also: &["TacticNotApplicable", "RewriteNoOccurrence"],
    })
}

/// `albite explain error E0087` — `ring`'s own code (ADR 4.10.26g D6).
pub(super) fn ring(name: &str) -> Option<ErrorExplanation> {
    (name == "RingUnequal").then_some(ErrorExplanation {
        name: "RingUnequal",
        code: "E0087",
        category: "Tactics and the Kernel",
        summary: "`ring` found the two sides unequal",
        detail: "\
`ring` proves an `Eq<Nat, a, b>` goal that holds in every commutative \
semiring: it reads each side as a polynomial over its atoms — the variables, \
and every subterm that is not `+`, `*` or a numeral — and rewrites both sides \
into one normal form, citing a `nat_arith` lemma per step. This error is a \
goal whose two sides normalise to different polynomials; the message prints \
both.\n\
\n\
Atoms are compared as written, so `ring` cannot see inside `n - 1` or `f(x)`: \
`(n - 1) + 1 = n` is refused, rightly, since it is false at `0`. `ring` uses \
no hypothesis — that is `omega`'s job — and refuses `pow`.\n\
\n\
`ring` needs `mod nat_arith;` in scope, and a goal that is not an equation \
over `Nat` is refused with E0082.",
        example: "\
theorem t(n: Nat, m: Nat) -> Eq<Nat, n + m, n * m> by {\n\
    ring   // E0087: `n + m` and `n * m`\n\
}",
        see_also: &["TacticNotApplicable", "KernelRefusedProof"],
    })
}

/// `albite explain error E0088` — `omega`'s own code (ADR 6.10.26f D7).
pub(super) fn omega(name: &str) -> Option<ErrorExplanation> {
    (name == "OmegaUnproved").then_some(ErrorExplanation {
        name: "OmegaUnproved",
        code: "E0088",
        category: "Tactics and the Kernel",
        summary: "`omega` could not prove its goal",
        detail: "\
`omega` proves a bounded linear goal over `Nat`: a `Le`-shaped goal \
`exists k: Nat. s + k == t`, or `Eq<Nat, s, t>`. It reads every hypothesis of \
either shape, solves each equation for a bare variable where it can, allows \
one cancellation round (`a + c == b + c` gives `a == b`, and `a + k == 0` \
gives `a == 0`), then computes `t - s` and closes the rest by `ring`. This \
error has three messages:\n\
\n\
• the goal holds a term `ring` cannot read, such as `pow`;\n\
• no witness: `t - s` has a negative term after every hypothesis was used — \
the goal is false, or needs a product, which is outside the fragment;\n\
• no witness over the oriented hypotheses: some hypothesis neither solved \
for a variable nor cancelled. The message names each one; `omega` does not \
combine them, which is Presburger arithmetic's job.\n\
\n\
`omega` needs `mod nat_arith;` in scope, and a goal of any other shape is \
refused with E0082.",
        example: "\
theorem t(n: Nat) -> Le<n + 1, n> by {\n\
    omega   // E0088: no witness: `t - s` is `-1`\n\
}",
        see_also: &["TacticNotApplicable", "RingUnequal"],
    })
}
