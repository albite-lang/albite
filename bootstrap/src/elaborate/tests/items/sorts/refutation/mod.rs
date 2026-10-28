//! `absurd(h)` reaches the kernel's refutation (ADR 7.10.26m AC 2, D4).
//!
//! The built-in lowers to `Absurd(τ, h)` on `Void` and on a `Bool` equation
//! between `true` and `false`, converting its witnesses only in a proof body,
//! as the kernel does. Every other hypothesis is E0010; with no expected type
//! the call never reaches the built-in and stays E0001; and a user `absurd`
//! keeps its meaning.

use super::{def, first_code, under_lambdas};
use crate::elaborate::tests::{elab_err, elab_ok};
use albite_core::typecheck::{check_proof, GlobalAdmission};
use albite_core::{Context, Term, Type};

/// The discriminator D5's `succ_ne_zero` carries `n + 1 == 0` through.
const DISC: &str = "|x: Nat| natrec(Bool, true, fn(k: Nat, acc: Bool) => false, x)";

/// Elaborate `source` and require the kernel to admit `name` at its stated
/// type, as a proof; returns its term with spans and `λ`s peeled.
fn admitted(source: &str, name: &str) -> Term {
    let defs = elab_ok(source);
    let target = def(&defs, name);
    let context = Context::new();
    assert_eq!(
        check_proof(&context, &target.term.term, &target.ty),
        Ok(()),
        "{name}: {}",
        target.term.term
    );
    under_lambdas(&target.term.term).strip_spans()
}

fn is_absurd_of_h(term: &Term, conclusion: &Type) -> bool {
    matches!(term, Term::Absurd(ty, h) if ty == conclusion && **h == Term::var("h"))
}

// The helper's own contract: only `Absurd(conclusion, h)` answers yes, so a
// lowering test cannot pass on some other term.
#[test]
fn is_absurd_of_h_names_only_that_shape() {
    let absurd = |ty: Type, h: &str| Term::absurd(ty, Term::var(h));
    assert!(is_absurd_of_h(&absurd(Type::Nat, "h"), &Type::Nat));
    assert!(!is_absurd_of_h(&absurd(Type::Bool, "h"), &Type::Nat));
    assert!(!is_absurd_of_h(&absurd(Type::Nat, "g"), &Type::Nat));
    assert!(!is_absurd_of_h(&Term::var("h"), &Type::Nat));
}

// 7.10.26m AC2 (D4): on `Void`, outside a proof, `absurd(h)` is `Absurd(τ, h)`.
#[test]
fn absurd_on_void_lowers_to_absurd() {
    let defs = elab_ok("fn f(h: Void) -> Nat { absurd(h) }");
    let term = under_lambdas(&def(&defs, "f").term.term).strip_spans();
    assert!(is_absurd_of_h(&term, &Type::Nat), "{term}");
}

// 7.10.26m AC2 (D1, D4): `true == false` lowers the same, and the kernel admits it.
#[test]
fn absurd_on_true_equals_false_is_admitted() {
    let source = "pub theorem t(h: Eq<Bool, true, false>) -> Void { absurd(h) }\n";
    assert!(is_absurd_of_h(&admitted(source, "t"), &Type::Void));
}

// 7.10.26m AC2 (D4): the mirror, outside a proof, by equality modulo spans.
#[test]
fn absurd_on_false_equals_true_outside_a_proof() {
    let defs = elab_ok("fn f(h: Eq<Bool, false, true>) -> Nat { absurd(h) }");
    let term = under_lambdas(&def(&defs, "f").term.term).strip_spans();
    assert!(is_absurd_of_h(&term, &Type::Nat), "{term}");
}

// 7.10.26m AC2 (D4): in a proof body the witnesses convert δ-free, so
// `succ_ne_zero` is admitted.
#[test]
fn a_convertible_refutation_is_admitted_in_a_proof() {
    let source = format!(
        "pub theorem s(n: Nat, h: Eq<Nat, n + 1, 0>) -> Void {{ absurd(cong({DISC}, h)) }}\n"
    );
    let term = admitted(&source, "s");
    assert!(matches!(term, Term::Absurd(Type::Void, _)), "{term}");
}

// 7.10.26m AC2 (D4): outside a proof the same call is refused, even after a
// theorem has been elaborated — the proof-body flag is restored.
#[test]
fn a_convertible_refutation_is_refused_outside_a_proof() {
    let source = format!(
        "pub theorem t(h: Eq<Bool, true, false>) -> Void {{ absurd(h) }}\n\
         fn g(n: Nat, h: Eq<Nat, n + 1, 0>) -> Void {{ absurd(cong({DISC}, h)) }}\n"
    );
    let errors = elab_err(&source);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind.code(), "E0010");
}

// 7.10.26m AC2 (D4): a satisfiable or untrusted hypothesis is E0010, with the note.
#[test]
fn every_other_hypothesis_is_a_mismatch() {
    for hypothesis in [
        "Eq<Bool, true, true>",
        "Eq<Nat, 0, 1>",
        "Eq<Nat, 1, 1>",
        "Nat",
    ] {
        let source = format!("pub theorem t(h: {hypothesis}) -> Void {{ absurd(h) }}\n");
        let errors = elab_err(&source);
        assert_eq!(errors[0].kind.code(), "E0010", "{hypothesis}");
        assert!(
            errors[0]
                .notes
                .iter()
                .any(|n| n.message.contains("`absurd` needs `Void`")),
            "{hypothesis}: {:?}",
            errors[0].notes
        );
    }
    let open = "pub theorem t(b: Bool, h: Eq<Bool, b, false>) -> Void { absurd(h) }\n";
    assert_eq!(first_code(open), "E0010");
}

// 7.10.26m AC2 (D4): with no expected type the built-in is never reached.
#[test]
fn absurd_with_no_expected_type_is_unresolved() {
    assert_eq!(
        first_code("fn f(h: Void) -> Unit { let _v = absurd(h); () }"),
        "E0001"
    );
}

// 7.10.26m D4: a user `absurd` in scope keeps its meaning.
#[test]
fn a_user_absurd_wins() {
    let defs = elab_ok("fn absurd(x: Nat) -> Nat { x }\nfn g() -> Nat { absurd(3) }");
    let term = under_lambdas(&def(&defs, "g").term.term).strip_spans();
    assert!(!matches!(term, Term::Absurd(..)), "{term}");
}

// 7.10.26m D4: two arguments are not the built-in.
#[test]
fn absurd_with_two_arguments_is_unresolved() {
    assert_eq!(first_code("fn f(h: Void) -> Nat { absurd(h, h) }"), "E0001");
}

// The kernel admits the lowered term under a context of earlier globals too.
#[test]
fn a_citation_of_a_refutation_is_admitted() {
    let source = "pub theorem t(h: Eq<Bool, true, false>) -> Void { absurd(h) }\n\
                  pub theorem u(h: Eq<Bool, false, true>) -> Eq<Nat, 0, 1> { absurd(t(sym(h))) }\n";
    let defs = elab_ok(source);
    let context = Context::new().with_global(
        def(&defs, "t").name.clone(),
        def(&defs, "t").ty.clone(),
        GlobalAdmission::Admitted,
    );
    let citing = def(&defs, "u");
    assert_eq!(check_proof(&context, &citing.term.term, &citing.ty), Ok(()));
}

// 7.10.26m D4: a poisoned signature reports its own fault and `absurd` adds
// none: the codes are exactly those of the same signature with no `absurd`.
// A signature error stops the file before any body elaborates, so this does
// not reach `check_absurd`'s `Type::Error` arm, which stays a defensive guard
// (as `check_sum_intro`'s does): no parsed input reaches it.
#[test]
fn a_poisoned_signature_adds_no_absurd_error() {
    let codes = |source: &str| -> Vec<&'static str> {
        elab_err(source).iter().map(|e| e.kind.code()).collect()
    };
    let with_absurd = codes("fn f(h: NoSuchType) -> Nat { absurd(h) }");
    assert_eq!(with_absurd, codes("fn f(h: NoSuchType) -> Nat { 0 }"));
    assert!(!with_absurd.contains(&"E0010"), "{with_absurd:?}");
}
