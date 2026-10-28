//! A disjunction is built and split at the surface (ADR 7.10.26l AC 1, AC 2).
//!
//! `inl`/`inr` lower to the core's `Inl`/`Inr`, a disjunction `match` to a
//! non-dependent `Case`, and `cases` to 3.10.26o's annotated split — each
//! re-checked here by the kernel. A user type's constructors keep their
//! meaning, a user `inl` included, and every refusal lands at its code.

use super::{def, first_code, under_lambdas};
use crate::elaborate::tests::{elab_err, elab_ok};
use albite_core::typecheck::{check_proof, GlobalAdmission};
use albite_core::{Context, Term, Type};

const EQS: &str = "Eq<Nat, 0, 0> + Eq<Nat, 1, 1>";

/// Elaborate `source` and require the kernel to admit `name` at its stated
/// type, with every earlier definition in context; returns its term.
fn admitted(source: &str, name: &str) -> Term {
    let defs = elab_ok(source);
    let context = defs.iter().fold(Context::new(), |ctx, d| {
        ctx.with_global(d.name.clone(), d.ty.clone(), GlobalAdmission::Admitted)
    });
    let target = def(&defs, name);
    assert_eq!(
        check_proof(&context, &target.term.term, &target.ty),
        Ok(()),
        "{name}: {}",
        target.term.term
    );
    target.term.term.strip_spans()
}

/// The definition `name`'s term in `source`, spans and `λ`s peeled.
fn lowered(source: &str, name: &str) -> Term {
    let defs = elab_ok(source);
    under_lambdas(&def(&defs, name).term.term).strip_spans()
}

// ADR 7.10.26l AC1 (D2): `inl(e)` against `A + B` is `Inl(A + B, e)`, and
// `inr(e)` is `Inr`; the kernel admits both.
#[test]
fn inl_and_inr_lower_to_the_core_injections() {
    let left = admitted(&format!("pub theorem l() -> {EQS} {{ inl(refl) }}\n"), "l");
    let Term::Inl(Type::Sum(..), inner) = under_lambdas(&left) else {
        panic!("expected Inl(Sum, _), got {left}");
    };
    assert!(matches!(**inner, Term::Refl(..)), "{inner}");
    let right = admitted(&format!("pub theorem r() -> {EQS} {{ inr(refl) }}\n"), "r");
    assert!(
        matches!(under_lambdas(&right), Term::Inr(Type::Sum(..), _)),
        "{right}"
    );
}

// ADR 7.10.26l AC1 (D3): a disjunction `match` is `Case`, its `inl` arm left
// whatever order the arms are written in.
#[test]
fn a_disjunction_match_lowers_to_case() {
    let source = format!("fn side(h: {EQS}) -> Nat {{ match h {{ inr(b) => 0, inl(a) => 1 }} }}");
    let Term::Case(scrutinee, left_binder, left, right_binder, right) = lowered(&source, "side")
    else {
        panic!("expected a Case");
    };
    assert_eq!(*scrutinee, Term::var("h"));
    assert_eq!((left_binder.as_str(), right_binder.as_str()), ("a", "b"));
    assert_eq!(*left, Term::succ(Term::Zero));
    assert_eq!(*right, Term::Zero);
}

// ADR 7.10.26l AC1 (D2, D3): `left`, `right` and `cases` assemble a term the
// kernel admits; the split is 3.10.26o's annotated `Case`.
#[test]
fn left_right_and_cases_build_an_annotated_split() {
    let source = format!(
        "pub theorem swap(h: {EQS}) -> Eq<Nat, 1, 1> + Eq<Nat, 0, 0> by {{\n\
             cases h {{ inl(a) => {{ right; exact a }}, inr(b) => {{ left; exact b }} }}\n\
         }}\n"
    );
    let term = admitted(&source, "swap");
    let Term::Annot(split, Type::Sum(..)) = under_lambdas(&term) else {
        panic!("expected Annot(_, Sum), got {term}");
    };
    let Term::Case(_, _, left, _, right) = &**split else {
        panic!("expected a Case, got {split}");
    };
    assert!(left.to_string().contains("inr"), "{left}");
    assert!(right.to_string().contains("inl"), "{right}");
}

// ADR 7.10.26l AC1 (D3): a user type's `match` lowers as it did — its
// constructors resolve first, so `Dir`'s `R` arm is still the `inr` branch.
#[test]
fn a_user_types_match_is_unchanged() {
    let source = "type Dir = L | R\nfn f(d: Dir) -> Nat { match d { R => 2, L => 1 } }";
    let core = lowered(source, "f").to_string();
    assert!(core.starts_with("(case d of inl"), "{core}");
    let source = "type Dir = L | R\n\
                  theorem t(d: Dir) -> Eq<Nat, 0, 0> by { cases d { L => refl, R => refl } }";
    assert!(matches!(lowered(source, "t"), Term::Annot(..)));
}

// ADR 7.10.26l AC1 (D3): a user constructor named `inl` wins. `P` declares
// `inr` first, so its `inl` is index 1, the core's right side.
#[test]
fn a_user_constructor_named_inl_wins() {
    let source = "pub type P = inr(Eq<Nat, 0, 0>) | inl(Eq<Nat, 1, 1>)\n\
                  fn pick(p: P) -> Nat { match p { inl(a) => 1, inr(b) => 0 } }";
    let core = lowered(source, "pick").to_string();
    let (left, right) = core.split_once("| inr").expect("two branches");
    assert!(left.contains("zero"), "{core}");
    assert!(right.contains("succ zero"), "{core}");
}

// ADR 7.10.26l AC1 (D2): a declared type whose payloads are proofs sorts
// `Prop`, so `inl` builds it — harmless, since the kernel admits `Inl` at
// any sum.
#[test]
fn inl_builds_a_declared_type_of_proofs() {
    let source = "type D = A(Eq<Nat, 0, 0>) | B(Eq<Nat, 1, 1>)\nfn d() -> D { inl(refl) }";
    assert!(matches!(lowered(source, "d"), Term::Inl(..)));
}

// ADR 7.10.26l AC2 (D2, D7): `inl` against a data sum, or against no sum, is
// E0024, naming the form and the type.
#[test]
fn inl_against_a_data_sum_is_e0024() {
    let errors = elab_err("fn f() -> Nat + Bool { inl(3) }");
    assert_eq!(errors[0].kind.code(), "E0024");
    assert!(
        errors[0].message.contains("`inl` builds a disjunction"),
        "{}",
        errors[0].message
    );
    assert!(
        errors[0].message.contains("(Nat + Bool)"),
        "{}",
        errors[0].message
    );
    let errors = elab_err("fn g() -> Nat { inr(3) }");
    assert_eq!(errors[0].kind.code(), "E0024");
    assert!(
        errors[0].message.starts_with("`inr`"),
        "{}",
        errors[0].message
    );
}

// ADR 7.10.26l AC2 (D2): with no expected type the side is unknown — E0011,
// where E0001 was; a value named `inl` keeps its meaning.
#[test]
fn inl_with_no_expected_type_is_e0011() {
    assert_eq!(
        first_code("fn f() -> Unit { let h = inl(refl); () }"),
        "E0011"
    );
    elab_ok("fn inl(n: Nat) -> Nat { n }\nfn f() -> Nat { inl(3) }");
    assert_eq!(first_code("fn f() -> Nat { inl(1, 2) }"), "E0001");
}

// ADR 7.10.26l AC2 (D2): `left` on a goal that is not a disjunction is E0082.
#[test]
fn left_on_an_equation_is_e0082() {
    let errors = elab_err("theorem t(n: Nat) -> Eq<Nat, n, n> by { left; refl }");
    assert_eq!(errors[0].kind.code(), "E0082");
    assert!(
        errors[0].message.contains("not a disjunction"),
        "{}",
        errors[0].message
    );
    let errors = elab_err("theorem t() -> Nat + Bool by { right; refl }");
    assert_eq!(errors[0].kind.code(), "E0082");
}

// ADR 7.10.26l AC2 (D3): a missing arm names its side; a repeated one is
// refused in the existing wording.
#[test]
fn cases_on_a_disjunction_needs_both_arms() {
    let missing = elab_err(&format!(
        "theorem t(h: {EQS}) -> Eq<Nat, 0, 0> by {{ cases h {{ inl(a) => exact a }} }}"
    ));
    assert_eq!(missing[0].kind.code(), "E0082");
    assert!(
        missing[0].message.contains("no arm for `inr`"),
        "{}",
        missing[0].message
    );
    let repeated = elab_err(&format!(
        "theorem t(h: {EQS}) -> Eq<Nat, 0, 0> by {{ cases h {{ inl(a) => exact a, inl(b) => refl }} }}"
    ));
    assert!(
        repeated[0].message.contains("`inl` has two arms"),
        "{}",
        repeated[0].message
    );
}

// ADR 7.10.26l AC2 (D3): an arm that is not `inl`/`inr` keeps today's
// error, and so does a data sum's `inl` arm (§ 3).
#[test]
fn a_foreign_arm_keeps_todays_error() {
    assert_eq!(
        first_code(&format!(
            "fn f(h: {EQS}) -> Nat {{ match h {{ inl(_) => 1, Some(_) => 0 }} }}"
        )),
        "E0003"
    );
    assert_eq!(
        first_code("fn f(s: Nat + Bool) -> Nat { match s { inl(a) => a, inr(b) => 0 } }"),
        "E0003"
    );
}

// ADR 7.10.26l AC2 (D3): the lowering is non-dependent, so an expected type
// that needs `h` substituted is refused at the arm — by the arm's own check,
// here `refl`'s E0071.
#[test]
fn a_dependent_result_is_refused_at_the_arm() {
    let source = format!(
        "fn tag(h: {EQS}) -> Nat {{ match h {{ inl(_) => 1, inr(_) => 0 }} }}\n\
         pub theorem dep(h: {EQS}) -> Eq<Nat, tag(h), 1> + Eq<Nat, tag(h), 0> {{\n\
             match h {{ inl(a) => inl(refl), inr(b) => inr(refl) }}\n\
         }}"
    );
    assert_eq!(first_code(&source), "E0071");
}
