//! ADR 13.10.26t AC 2 — a `fn` that can hand back a proof is checked like a
//! term-mode theorem, and one that cannot is not.
//!
//! A folder of its own because `kernel/tests/` holds its eight-file cap.

mod string_primitives;

use std::collections::HashMap;

use albite_core::{Term, Type};

use super::eq_nat;
use crate::elaborate::carries_proof;
use crate::elaborate::kernel::ObligationOrigin;
use crate::elaborate::Elaborator;
use crate::parse;

const BIG: &str = "fn big() -> Nat { 18446744073709551615 }\n";

/// `source` elaborated as a project, the way `check` does: the single-file
/// path refuses §1's `refl` itself, and only the module walk's δ discharge
/// accepts it, so the witness exists only here. The first error's message,
/// or `None` when the project elaborates.
fn project_error(source: &str) -> Option<String> {
    let dir = tempfile::tempdir().expect("a tempdir");
    let main = dir.path().join("main.tg");
    std::fs::write(&main, source).expect("the fixture is written");
    match crate::driver::elaborate_project_cold(&main, 20) {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    }
}

/// The kernel's refusal of `def` as the first error, with its cause.
fn assert_refused(source: &str, def: &str, cause: &str) {
    let error = project_error(source).expect("the project is refused");
    let expected = format!("the kernel refused the proof of `{def}`: {cause}");
    assert!(error.contains(&expected), "{error}");
}

/// The definitions `source` records a kernel obligation for, with the
/// discharge deferred so the obligations stay readable.
fn obligations(source: &str) -> Vec<(String, bool)> {
    let (ast, parse_errors) = parse(source);
    assert!(parse_errors.is_empty(), "{parse_errors:?}");
    let ctx = Box::leak(Box::new(albite_core::Context::new()));
    let mut elaborator = Elaborator::new(ctx);
    elaborator.defer_kernel = true;
    elaborator
        .elaborate_file(&ast)
        .unwrap_or_else(|errors| panic!("elaboration failed: {errors:?}"));
    elaborator
        .kernel_obligations
        .iter()
        .map(|o| (o.def.clone(), o.origin == ObligationOrigin::Term))
        .collect()
}

fn no_encoding(_: &str) -> Option<&'static Type> {
    None
}

fn false_eq() -> Type {
    eq_nat(Term::Zero, Term::nat_lit(1))
}

// 13.10.26t AC 2 — the witness: §1.1's file is refused at `bad`, not admitted
// at `cites`.
#[test]
fn a_false_proof_fn_is_refused_at_the_fn() {
    let source = format!(
        "{BIG}fn bad() -> Eq<Nat, big() + 1, big()> {{ refl }}\n\
         theorem cites(): Eq<Nat, big() + 1, big()> {{ bad() }}\n"
    );
    assert_refused(&source, "bad", "unexplained-mismatch");
}

// 13.10.26t AC 2 — §1.2: a proof inside a pair is checked at the `fn`.
#[test]
fn a_proof_inside_a_pair_is_checked() {
    let source = format!(
        "{BIG}fn bad_pair() -> (Eq<Nat, big() + 1, big()>, Nat) {{ (refl, 0) }}\n\
         theorem t(): Eq<Nat, big() + 1, big()> {{ let (p, _n) = bad_pair(); p }}\n"
    );
    assert_refused(&source, "bad_pair", "unexplained-mismatch");
}

// 13.10.26t AC 2 — §1.2: a proof inside a constructor is checked at the `fn`.
#[test]
fn a_proof_inside_a_constructor_is_checked() {
    let source = format!(
        "{BIG}type Opt<A> = Non | Som(A)\n\
         fn bad_opt() -> Opt<Eq<Nat, big() + 1, big()>> {{ Som(refl) }}\n"
    );
    assert_refused(&source, "bad_opt", "unexplained-mismatch");
}

// 13.10.26t AC 2 — a true proof `fn` is admitted, and its obligation is a
// term-mode one.
#[test]
fn a_true_proof_fn_is_admitted() {
    let source = "fn same(n: Nat) -> Eq<Nat, n, n> { refl }\n\
                  theorem uses(): Eq<Nat, 3, 3> { same(3) }\n";
    assert_eq!(project_error(source), None);
    assert_eq!(
        obligations(source),
        vec![("same".to_string(), true), ("uses".to_string(), true)]
    );
}

// 13.10.26t AC 2 — §1.2's control: a proof the result type does not expose
// records no obligation, so `f` is not checked.
#[test]
fn a_fn_whose_type_holds_no_proof_records_no_obligation() {
    let source = "fn f() -> Nat { let p: Eq<Nat, 1, 1> = refl; 0 }\n\
                  fn g<T>(x: T) -> T { x }\n";
    assert!(obligations(source).is_empty());
}

// 13.10.26t AC 2 — `#[partial]` opts out: citing one is already E0063.
#[test]
fn a_partial_proof_fn_records_no_obligation() {
    let source = "#[partial]\nfn spin(n: Nat) -> Eq<Nat, n, n> { spin(n) }\n";
    assert!(obligations(source).is_empty());
}

// 13.10.26t AC 2 — a `sorry` proof `fn` is a hole: warned, never an error
// (D2), though it is now in the set.
#[test]
fn a_sorry_proof_fn_is_not_an_error() {
    let source = "fn hole() -> Eq<Nat, 0, 1> { sorry }\n\
                  theorem zero_is_one(): Eq<Nat, 0, 1> { hole() }\n";
    assert_eq!(project_error(source), None);
    assert_eq!(obligations(source)[0], ("hole".to_string(), true));
}

// 13.10.26t AC 2 — D1's predicate on each shape it names.
#[test]
fn carries_proof_finds_a_proposition_anywhere_a_value_hands_one_back() {
    let proof = false_eq;
    let in_set = [
        proof(),
        Type::Void,
        Type::arrow(Type::Nat, proof()),
        Type::product(proof(), Type::Nat),
        Type::sum(Type::Unit, proof()),
        Type::sigma("n", Type::Nat, proof()),
        Type::Forall("T".into(), Box::new(proof())),
        Type::Mu("a".into(), Box::new(Type::sum(Type::Unit, proof()))),
        Type::Adt(
            "Three".into(),
            vec![],
            vec![("One".into(), proof()), ("Two".into(), Type::Nat)],
        ),
        Type::Adt("Box".into(), vec![proof()], vec![]),
        Type::App("Box".into(), vec![proof()]),
        Type::Ref(Box::new(proof())),
        Type::Ptr(Box::new(proof())),
        Type::ForallPred("P".into(), vec![Type::Nat], Box::new(proof())),
    ];
    for ty in &in_set {
        assert!(carries_proof(ty, no_encoding), "{ty}");
    }
    let outside = [
        Type::Nat,
        Type::Mu("a".into(), Box::new(Type::sum(Type::Unit, Type::Nat))),
        Type::Forall("T".into(), Box::new(Type::TyVar("T".into()))),
        Type::arrow(proof(), Type::Nat),
        Type::product(Type::Nat, Type::String),
        Type::Error,
    ];
    for ty in &outside {
        assert!(!carries_proof(ty, no_encoding), "{ty}");
    }
}

// 13.10.26t AC 2 — a record reaches elaboration as a bare name, so it is read
// through its encoding, in either spelling, and a self-naming one terminates.
#[test]
fn carries_proof_reads_a_named_type_through_its_encoding() {
    let encodings: HashMap<String, Type> = HashMap::from([
        ("Rec".into(), Type::product(false_eq(), Type::Nat)),
        ("Pt".into(), Type::product(Type::Nat, Type::Nat)),
        (
            "Loop".into(),
            Type::sum(Type::Unit, Type::TyVar("Loop".into())),
        ),
    ]);
    let lookup = |name: &str| encodings.get(name);
    assert!(carries_proof(&Type::TyVar("Rec".into()), lookup));
    assert!(carries_proof(&Type::TyVar("@Rec".into()), lookup));
    assert!(!carries_proof(&Type::TyVar("Pt".into()), lookup));
    assert!(!carries_proof(&Type::TyVar("Loop".into()), lookup));
    assert!(!carries_proof(&Type::TyVar("Rec".into()), no_encoding));

    // An encoding finalised mid-collection is keyed `@Name`: the spelling
    // written is tried after the bare one.
    let collecting: HashMap<String, Type> =
        HashMap::from([("@Held".into(), Type::product(Type::Nat, false_eq()))]);
    assert!(carries_proof(&Type::TyVar("@Held".into()), |name| {
        collecting.get(name)
    }));
}
