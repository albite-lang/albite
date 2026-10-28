//! ADR 29.9.26c: head equality and the literal-operand edge cases, asserted
//! directly. Split from `tests.rs` (at its size limit); each test here pins a
//! decision a comparison-level test cannot reach — two syntactically equal
//! sides never get as far as `same_head`. The by-name guard tests that drive
//! the bootstrap's discharge stayed there, in its `whnf_tests.rs`
//! (ADR 30.9.26g D6).

use crate::terms::IntBinOp;
use crate::{Term, Type};

use super::compare::same_head;
use super::tests::head;

fn var(name: &str) -> Term {
    Term::var(name)
}

// 29.9.26c D7: names, literals and in-term types must match for two heads to agree.
#[test]
fn same_head_compares_names_literals_and_types() {
    for (name, other) in [
        (var("x"), var("y")),
        (Term::global("f"), Term::global("g")),
        (Term::string_lit("a"), Term::string_lit("b")),
        (Term::ty_abs("T", Term::Zero), Term::ty_abs("U", Term::Zero)),
        (Term::IntLit(1), Term::IntLit(2)),
        (
            Term::refl(Type::Nat, Term::Zero),
            Term::refl(Type::Bool, Term::Zero),
        ),
    ] {
        assert!(same_head(&name, &name.clone()), "{name} against itself");
        assert!(!same_head(&name, &other), "{name} against {other}");
    }
    let add = Term::int_bin(IntBinOp::Add, Term::IntLit(1), Term::IntLit(2));
    let sub = Term::int_bin(IntBinOp::Sub, Term::IntLit(1), Term::IntLit(2));
    assert!(same_head(&add, &add.clone()));
    assert!(!same_head(&add, &sub));
    assert!(!same_head(&Term::Zero, &Term::Unit), "different variants");
}

// 29.9.26c D7: a two-part head differs when EITHER part does.
#[test]
fn same_head_needs_every_part_of_a_compound_head() {
    let subst = |ty: Type, motive: Type| Term::subst(ty, motive, var("e"), var("p"));
    let base = subst(Type::Nat, Type::Nat);
    assert!(same_head(&base, &subst(Type::Nat, Type::Nat)));
    assert!(!same_head(&base, &subst(Type::Bool, Type::Nat)));
    assert!(!same_head(&base, &subst(Type::Nat, Type::Bool)));

    let construct = |ty: Type, tag| Term::adt_construct(ty, tag, Term::Zero);
    assert!(same_head(
        &construct(Type::Nat, 0),
        &construct(Type::Nat, 0)
    ));
    assert!(!same_head(
        &construct(Type::Nat, 0),
        &construct(Type::Bool, 0)
    ));
    assert!(!same_head(
        &construct(Type::Nat, 0),
        &construct(Type::Nat, 1)
    ));

    let call = |name: &str, arity| Term::extern_call(name, vec![Term::Zero; arity]);
    assert!(same_head(&call("tg_f", 1), &call("tg_f", 1)));
    assert!(!same_head(&call("tg_f", 1), &call("tg_g", 1)));
    assert!(!same_head(&call("tg_f", 1), &call("tg_f", 2)));

    let arm = |tag| (tag, "p".to_string(), Box::new(var("p")));
    let matching = |arms| Term::adt_match(var("s"), arms);
    assert!(same_head(&matching(vec![arm(0)]), &matching(vec![arm(0)])));
    assert!(!same_head(&matching(vec![arm(0)]), &matching(vec![arm(1)])));
    assert!(!same_head(
        &matching(vec![arm(0)]),
        &matching(vec![arm(0), arm(1)])
    ));
}

// 29.9.26c D3: a literal instance the evaluator leaves stuck stays neutral —
// it is not "reduced" to itself forever until the budget runs out.
#[test]
fn a_stuck_literal_instance_stays_neutral() {
    let ill_typed = Term::nat_eq(Term::string_lit("a"), Term::NatLit(0));
    assert_eq!(head(ill_typed.clone()), ill_typed);
}

// 29.9.26c D3: a successor chain reads only down to a numeral.
#[test]
fn a_successor_of_a_non_numeral_is_not_a_literal() {
    let not_a_numeral = Term::bool_not(Term::succ(Term::True));
    assert_eq!(head(not_a_numeral.clone()), not_a_numeral);
}
