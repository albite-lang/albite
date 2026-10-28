//! Substitution and free variables enter a `Subst` motive (ADR 1.10.26e AC 3).

use std::collections::HashSet;

use crate::terms::Term;
use crate::types::Type;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `subst [Nat] [motive] h p`, with `h` and `p` plain variables.
fn subst_with(motive: Type) -> Term {
    Term::subst(Type::Nat, motive, Term::var("h"), Term::var("p"))
}

fn motive_of(term: &Term) -> &Type {
    match term {
        Term::Subst(_, motive, _, _) => motive,
        other => panic!("expected a Subst, got {other:?}"),
    }
}

/// 1.10.26e AC 3: a free variable of the motive is substituted, as are the
/// two terms beside it.
#[test]
fn substitution_enters_a_subst_motive() {
    let motive = Type::pi("x", Type::Nat, eq_nat(Term::var("a"), Term::var("x")));
    let substituted = subst_with(motive).substitute("a", &Term::Zero);
    assert_eq!(
        motive_of(&substituted),
        &Type::pi("x", Type::Nat, eq_nat(Term::Zero, Term::var("x")))
    );
    // A constant (Arrow) motive is substituted too.
    let constant = Type::arrow(Type::Nat, eq_nat(Term::var("a"), Term::var("a")));
    let substituted = subst_with(constant).substitute("a", &Term::Zero);
    assert_eq!(
        motive_of(&substituted),
        &Type::arrow(Type::Nat, eq_nat(Term::Zero, Term::Zero))
    );
}

/// 1.10.26e AC 3: the motive's binder shadows a substitution for its own name.
#[test]
fn a_subst_motive_binder_shadows_the_substituted_name() {
    let motive = Type::pi("x", Type::Nat, eq_nat(Term::var("x"), Term::var("x")));
    let substituted = subst_with(motive.clone()).substitute("x", &Term::Zero);
    assert_eq!(motive_of(&substituted), &motive);
}

/// 1.10.26e AC 3: `(Π x. Eq a x)[a := x]` renames the binder, so the
/// substituted `x` stays free and distinct from it.
#[test]
fn substitution_renames_a_capturing_motive_binder() {
    let motive = Type::pi("x", Type::Nat, eq_nat(Term::var("a"), Term::var("x")));
    let substituted = subst_with(motive).substitute("a", &Term::var("x"));
    let Type::Pi(binder, _, body) = motive_of(&substituted) else {
        panic!("the motive stays a Pi");
    };
    assert_ne!(binder, "x");
    assert_eq!(**body, eq_nat(Term::var("x"), Term::var(binder.clone())));
}

/// 1.10.26e AC 3: the motive's free term variables are the `Subst`'s, its
/// binder excepted.
#[test]
fn free_vars_count_a_subst_motive_but_not_its_binder() {
    let motive = Type::pi("x", Type::Nat, eq_nat(Term::var("a"), Term::var("x")));
    let free = subst_with(motive).free_vars();
    let expected: HashSet<String> = ["a", "h", "p"].iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(free, expected);
}
