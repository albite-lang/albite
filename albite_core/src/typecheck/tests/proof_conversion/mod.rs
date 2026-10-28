//! The proof judgment converts, and types the terms inside an `Eq` (ADR
//! 1.10.26c), `Subst` types as its motive's instance (ADR 1.10.26e), and
//! conversion unfolds only the bodies the kernel admitted (ADR 2.10.26l).
//! Shared builders for the suites.

use super::*;
use crate::context::Context;
use crate::terms::Term;
use crate::types::Type;

mod delta;
mod delta_admission;
mod delta_free;
mod dependent_subst;
mod eq_witnesses;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

fn var(name: &str) -> Term {
    Term::var(name)
}

fn plus(left: Term, right: Term) -> Term {
    Term::nat_add(left, right)
}

fn refl_nat(witness: Term) -> Term {
    Term::refl(Type::Nat, witness)
}

fn over_n() -> Context {
    Context::new().with_term("n", Type::Nat)
}

/// `λj:Nat. λh:hypothesis. proof`, a dependent `natind`'s step.
fn step(hypothesis: Type, proof: Term) -> Term {
    Term::lambda("j", Type::Nat, Term::lambda("h", hypothesis, proof))
}

/// `natind [k. Nat → Eq Nat (k + 0) k] (refl (0 + 0)) step n`: every case
/// holds only by conversion, and the motive names `k`, which no context binds.
fn right_zero_induction() -> Term {
    let motive = Type::arrow(Type::Nat, eq_nat(plus(var("k"), Term::Zero), var("k")));
    let hypothesis = eq_nat(plus(var("j"), Term::Zero), var("j"));
    Term::natind(
        "k",
        motive,
        refl_nat(plus(Term::Zero, Term::Zero)),
        step(hypothesis, refl_nat(Term::succ(var("j")))),
        var("n"),
    )
}
