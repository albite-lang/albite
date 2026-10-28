//! The `cases` pair arm (ADR 6.10.26e AC 2, D6): the β-redex over hand-built
//! Core, then through the elaborator — the arm's locals, `_`, a bound name
//! refused, and a goal that mentions the scrutinee.
//!
//! `elab_ok` does not run the kernel boundary, so each elaborated proof is
//! re-checked here with `check_proof`, as `pair_pattern.rs` does for 6.10.26c.

use albite_core::typecheck::check_proof;
use albite_core::{Context, Term, Type};

use crate::elaborate::error::TacticErrorKind;
use crate::elaborate::tactics::cases::pair::{pair_fields, pair_template};
use crate::elaborate::tactics::tests::tactic_error;
use crate::elaborate::tests::elab_ok;

const LE: &str = "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n";

/// `exists x: Nat. Eq Nat x 0`.
fn exists_zero() -> Type {
    Type::sigma(
        "x",
        Type::Nat,
        Type::eq(Type::Nat, Term::var("x"), Term::Zero),
    )
}

// ADR 6.10.26e AC2: the fields are the witness and the evidence at it.
#[test]
fn the_fields_are_the_witness_and_the_evidence_at_it() {
    let fields =
        pair_fields(&exists_zero(), ["k".to_string(), "e".to_string()]).expect("an exists");
    assert_eq!(
        fields,
        vec![
            ("k".to_string(), Type::Nat),
            (
                "e".to_string(),
                Type::eq(Type::Nat, Term::var("k"), Term::Zero)
            ),
        ]
    );
    assert_eq!(pair_fields(&Type::Nat, ["k".into(), "e".into()]), None);
    let product = Type::product(Type::Nat, Type::Nat);
    assert_eq!(pair_fields(&product, ["k".into(), "e".into()]), None);
}

// ADR 6.10.26e AC2: the template is 6.10.26c's redex, annotated at the goal
// unchanged.
#[test]
fn the_template_is_the_annotated_redex() {
    let fields = pair_fields(&exists_zero(), ["k".to_string(), "e".to_string()]).unwrap();
    let goal = Type::eq(Type::Nat, Term::Zero, Term::Zero);
    let evidence_ty = fields[1].1.clone();
    let expected = Term::annot(
        Term::app(
            Term::app(
                Term::lambda(
                    "k",
                    Type::Nat,
                    Term::lambda("e", evidence_ty, Term::var("%hole")),
                ),
                Term::fst(Term::var("h")),
            ),
            Term::snd(Term::var("h")),
        ),
        goal.clone(),
    );
    assert_eq!(pair_template("h", &fields, "%hole", &goal), expected);
}

/// Elaborate `LE` + `source` and require the kernel to admit `name`. No
/// proof below cites a global, so the kernel's context is empty.
fn admitted(source: &str, name: &str) -> Term {
    let defs = elab_ok(&format!("{LE}{source}"));
    let def = defs
        .iter()
        .find(|def| def.name == name)
        .unwrap_or_else(|| panic!("no definition `{name}`"));
    assert_eq!(
        check_proof(&Context::new(), &def.term.term, &def.ty),
        Ok(()),
        "{name}: {}",
        def.term.term
    );
    def.term.term.strip_spans()
}

// ADR 6.10.26e AC2: the arm sees `k` and `e`, and its proof lands under the
// redex's λs; the kernel admits it.
#[test]
fn the_arm_sees_the_witness_and_the_evidence() {
    let term = admitted(
        "theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by {
             cases h { (k, e) => { exists k; exact e } }
         }",
        "t",
    );
    let printed = term.to_string();
    assert!(printed.contains("fst h"), "{printed}");
    assert!(printed.contains("snd h"), "{printed}");
    assert!(printed.contains("λk:Nat"), "{printed}");
}

// ADR 6.10.26e AC2: `_` takes a synthesised `%cases` name, and the arm
// still closes.
#[test]
fn an_underscore_takes_a_synthesised_name() {
    let term = admitted(
        "theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by {
             cases h { (_, _) => exact h }
         }",
        "t",
    );
    assert!(term.to_string().contains("%cases"), "{term}");
}

// ADR 6.10.26e AC2: the goal is `G` as stated, so one that mentions `h` is
// not refused, and `h` closes it.
#[test]
fn a_goal_that_mentions_the_scrutinee_is_closed_with_it() {
    admitted(
        "theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by {
             cases h { (k, e) => exact h }
         }",
        "t",
    );
}

fn refusal_why(source: &str) -> String {
    match tactic_error(&format!("{LE}{source}")) {
        TacticErrorKind::TacticNotApplicable { tactic, why, .. } => {
            assert_eq!(tactic, "cases");
            why
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

// ADR 6.10.26e AC2: a name already bound, or the same name twice, is refused
// with today's message (3.10.26o D1's no-capture rule).
#[test]
fn a_bound_or_repeated_name_is_refused() {
    let why = refusal_why(
        "theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by { cases h { (a, e) => exact h } }",
    );
    assert_eq!(why, "`a` is already bound");
    let why = refusal_why(
        "theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by { cases h { (h, e) => exact h } }",
    );
    assert_eq!(why, "`h` is already bound");
    let why = refusal_why(
        "theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by { cases h { (k, k) => exact h } }",
    );
    assert_eq!(why, "`k` is bound twice in one arm");
}

// ADR 6.10.26e AC2: a pair arm over a non-`exists` local, or over a global,
// is refused.
#[test]
fn a_pair_arm_needs_an_exists_local() {
    let why =
        refusal_why("theorem t(n: Nat) -> Le<n, n> by { cases n { (k, e) => exact (0, refl) } }");
    assert_eq!(why, "a pair arm needs an `exists` local, and `n` is `Nat`");
    let why = refusal_why(
        "theorem g(n: Nat) -> Le<n, n> { (0, refl) }
         theorem t(n: Nat) -> Le<n, n> by { cases g { (k, e) => exact (0, refl) } }",
    );
    assert_eq!(why, "the scrutinee is not a local: `g` is a global");
}

// ADR 6.10.26e AC2: an arm that leaves its goal open is E0083, at the step.
#[test]
fn an_open_arm_is_unsolved_goals() {
    let kind = tactic_error(&format!(
        "{LE}theorem t(a: Nat, b: Nat, h: Le<a, b>) -> Le<a, b> by {{ cases h {{ (k, e) => exists k }} }}"
    ));
    assert!(
        matches!(kind, TacticErrorKind::UnsolvedGoals { .. }),
        "{kind:?}"
    );
}
