//! The proof judgment's context-level gates (ADR 29.9.26e ACs 5–7): global
//! admission, poison and holes, and the proof flag reaching every
//! sub-derivation. Split from `proof_gates.rs` by gate.

use super::proof_gates::{loop_at_void, tv, void_stream};
use super::*;
use crate::context::{Context, GlobalAdmission};
use crate::terms::Term;
use crate::types::Type;

// ==========================================================================
// AC 5 — a global is usable in a proof only when bound admitted (D4)
// ==========================================================================

#[test]
fn proof_uses_a_global_only_when_admitted() {
    let admitted = Context::new().with_global("two", Type::Nat, GlobalAdmission::Admitted);
    let opaque = Context::new().with_global("two", Type::Nat, GlobalAdmission::Opaque);
    let hypothesis = Context::new().with_term("two", Type::Nat);
    let refused = || Err(TypeError::GlobalNotAdmitted { name: "two".into() });

    assert_eq!(
        type_of_proof(&admitted, &Term::global("two")),
        Ok(Type::Nat)
    );
    assert_eq!(type_of_proof(&admitted, &Term::var("two")), Ok(Type::Nat));
    assert_eq!(type_of_proof(&opaque, &Term::global("two")), refused());
    assert_eq!(type_of_proof(&opaque, &Term::var("two")), refused());
    assert_eq!(type_of_proof(&hypothesis, &Term::global("two")), refused());
    assert_eq!(type_of_proof(&hypothesis, &Term::var("two")), Ok(Type::Nat));
    assert_eq!(
        type_of_proof(&Context::new(), &Term::global("two")),
        Err(TypeError::UnboundVariable("two".into()))
    );

    for ctx in [&admitted, &opaque, &hypothesis] {
        assert_eq!(type_of(ctx, &Term::global("two")), Ok(Type::Nat));
        assert_eq!(type_of(ctx, &Term::var("two")), Ok(Type::Nat));
    }
}

// ==========================================================================
// AC 6 — poison and holes are refused in a proof only (D9, D10)
// ==========================================================================

fn assert_poison(judgment: &TypeResult<Type>) {
    assert!(
        matches!(judgment, Err(TypeError::PoisonInProof { .. })),
        "{judgment:?}"
    );
}

/// The helper itself must fail on anything but the refusal it names.
#[test]
#[should_panic(expected = "SorryInProof")]
fn assert_poison_rejects_another_refusal() {
    assert_poison(&Err(TypeError::SorryInProof));
}

/// `λp: Eq<Nat, (0 : ty), 0>. ()` — `ty` sits inside a witness term.
fn eq_witness_carrying(ty: Type) -> Term {
    let witness = Term::annot(Term::Zero, ty);
    Term::lambda("p", Type::eq(Type::Nat, witness, Term::Zero), Term::Unit)
}

/// 29.9.26e AC6: poison in an annotation, a binder, a witness term and a
/// hypothesis is refused in proof mode only.
#[test]
fn proof_refuses_poison_everywhere_it_can_hide() {
    let ctx = Context::new();
    let nested_annot = Term::annot(Term::annot(Term::Unit, Type::Error), Type::Void);
    let lambda_param = Term::lambda("x", Type::Error, Term::Unit);
    let witness = eq_witness_carrying(Type::Error);
    let poisoned_hypothesis = Context::new().with_term("h", Type::Error);

    assert_poison(&type_of_proof(&ctx, &nested_annot));
    assert_poison(&type_of_proof(&ctx, &lambda_param));
    assert_poison(&type_of_proof(&ctx, &witness));
    assert_poison(&type_of_proof(&poisoned_hypothesis, &Term::Unit));

    assert_eq!(type_of(&ctx, &nested_annot), Ok(Type::Void));
    assert!(type_of(&ctx, &lambda_param).is_ok());
    assert!(type_of(&ctx, &witness).is_ok());
    assert_eq!(type_of(&poisoned_hypothesis, &Term::Unit), Ok(Type::Unit));
}

#[test]
fn proof_admits_the_same_terms_without_poison() {
    let ctx = Context::new();
    let nested_annot = Term::annot(Term::annot(Term::Unit, Type::Unit), Type::Unit);
    let lambda_param = Term::lambda("x", Type::Unit, Term::Unit);
    let clean_hypothesis = Context::new().with_term("h", Type::Unit);
    assert_eq!(type_of_proof(&ctx, &nested_annot), Ok(Type::Unit));
    assert!(type_of_proof(&ctx, &lambda_param).is_ok());
    assert!(type_of_proof(&ctx, &eq_witness_carrying(Type::Nat)).is_ok());
    assert_eq!(
        type_of_proof(&clean_hypothesis, &Term::Unit),
        Ok(Type::Unit)
    );
}

#[test]
fn proof_refuses_sorry_and_a_return_hidden_in_a_witness() {
    let hole = Term::annot(Term::Sorry, Type::Void);
    assert_eq!(
        type_of_proof(&Context::new(), &hole),
        Err(TypeError::SorryInProof)
    );
    assert_eq!(type_of(&Context::new(), &hole), Ok(Type::Void));

    let witness = Term::Return(Box::new(Term::Zero));
    let hidden = Term::lambda("p", Type::eq(Type::Nat, witness, Term::Zero), Term::Unit);
    assert_eq!(
        type_of_proof(&Context::new(), &hidden),
        Err(TypeError::ReturnInProof)
    );
}

#[test]
fn proof_admits_a_real_inhabitant_where_sorry_was() {
    let filled = Term::annot(Term::Zero, Type::Nat);
    assert_eq!(type_of_proof(&Context::new(), &filled), Ok(Type::Nat));
}

// ==========================================================================
// AC 7 — proof mode reaches every sub-derivation (D2)
// ==========================================================================

/// 29.9.26e AC7: `fix f: Void. f` placed under each rule that extends the context. The
/// pre-pass does not look at `Fix`, so each refusal proves the flag arrived.
#[test]
fn proof_mode_reaches_every_binder() {
    let bad = loop_at_void;
    let unit_or_unit = Type::sum(Type::Unit, Type::Unit);
    let scrutinee = Term::inl(unit_or_unit, Term::Unit);
    let pick = Type::Adt("Pick".into(), vec![], vec![("PA".into(), Type::Unit)]);
    let placements = [
        ("lambda", Term::lambda("y", Type::Nat, bad())),
        ("tyabs", Term::ty_abs("a", bad())),
        (
            "let definition",
            Term::let_in("x", Type::Void, bad(), Term::Unit),
        ),
        ("let body", Term::let_in("x", Type::Nat, Term::Zero, bad())),
        (
            "case left",
            Term::case(scrutinee.clone(), "l", bad(), "r", Term::var("v")),
        ),
        (
            "case right",
            Term::case(scrutinee, "l", Term::var("v"), "r", bad()),
        ),
        (
            "adt match",
            Term::adt_match(
                Term::adt_construct(pick, 0, Term::Unit),
                vec![(0, "u".into(), Box::new(bad()))],
            ),
        ),
        ("fix body", Term::fix("g", Type::Void, bad())),
    ];
    let ctx = Context::new().with_term("v", Type::Void);
    for (placement, term) in placements {
        let judgment = type_of_proof(&ctx, &term);
        assert!(
            matches!(judgment, Err(TypeError::FixNotStructural { ref name, .. }) if name == "f"),
            "{placement}: {judgment:?}"
        );
        assert!(type_of(&ctx, &term).is_ok(), "{placement}");
    }
    assert!(Context::new()
        .for_proof()
        .with_term("x", Type::Nat)
        .is_proof());
}

// ==========================================================================
// check_proof — the strict comparison at the top
// ==========================================================================

#[test]
fn check_proof_compares_strictly() {
    let ctx = Context::new().with_type_var("a").with_term("x", tv("a"));
    // `a` equals any μ binding `a` to the executable judgment only.
    assert_eq!(check(&ctx, &Term::var("x"), &void_stream()), Ok(()));
    assert_eq!(
        check_proof(&ctx, &Term::var("x"), &void_stream()),
        Err(TypeError::TypeMismatch {
            expected: void_stream(),
            got: tv("a"),
        })
    );
    assert_eq!(check_proof(&ctx, &Term::var("x"), &tv("a")), Ok(()));
    assert_eq!(
        check_proof(
            &Context::new(),
            &Term::Return(Box::new(Term::Unit)),
            &Type::Void
        ),
        Err(TypeError::ReturnInProof)
    );
}

// ==========================================================================
// The six refusals render (ADR 29.9.26e §2.3)
// ==========================================================================

#[test]
fn each_proof_refusal_renders_what_it_refused() {
    let rendered = [
        (
            TypeError::FixNotStructural {
                name: "f".into(),
                reason: "NoDescent".into(),
            },
            "fix `f` is not structural in a proof: NoDescent",
        ),
        (
            TypeError::NonStrictlyPositiveMu {
                mu: Type::Nat,
                field: "g → Void".into(),
            },
            "Nat is not strictly positive in g → Void",
        ),
        (
            TypeError::ReturnInProof,
            "`return` is not allowed in a proof",
        ),
        (
            TypeError::GlobalNotAdmitted {
                name: "loop".into(),
            },
            "`loop` is not an admitted global in a proof",
        ),
        (
            TypeError::PoisonInProof { ty: Type::Nat },
            "poisoned type in a proof: Nat",
        ),
        (TypeError::SorryInProof, "`sorry` is not allowed in a proof"),
    ];
    for (error, expected) in rendered {
        assert_eq!(error.to_string(), expected);
    }
}
