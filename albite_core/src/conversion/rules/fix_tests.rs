//! ADR 2.10.26l AC 2: `fix` unrolls only on a constructor. Pure: every test
//! builds its fixpoint, its recorded position and its terms directly.

use std::collections::HashMap;

use crate::conversion::{convertible, convertible_in, whnf, Convertible, Fuel, Unfoldings};
use crate::eval::EvalStopped;
use crate::terms::termination::tests::fixtures::{
    length_body, list_fn, list_of, list_to_nat, list_type, match_list, var,
};
use crate::{Term, Type};

use super::fix::is_constructor_headed;

/// Admitted bodies with their recorded positions, as the kernel's context
/// hands them to conversion.
#[derive(Default)]
struct Admitted {
    bodies: HashMap<String, Term>,
    positions: HashMap<String, usize>,
}

impl Admitted {
    fn with(mut self, name: &str, body: Term, position: Option<usize>) -> Self {
        self.bodies.insert(name.to_string(), body);
        if let Some(position) = position {
            self.positions.insert(name.to_string(), position);
        }
        self
    }
}

impl Unfoldings for Admitted {
    fn body(&self, name: &str) -> Option<&Term> {
        self.bodies.get(name)
    }

    fn decreasing(&self, name: &str) -> Option<usize> {
        self.positions.get(name).copied()
    }
}

/// `length` admitted as `fix f. λl. …f(t)…`, decreasing on its only argument.
fn length(position: Option<usize>) -> Admitted {
    let body = Term::fix("f", list_to_nat(), length_body(var("f")));
    Admitted::default().with("length", body, position)
}

fn length_of(list: Term) -> Term {
    Term::app(Term::global("length"), list)
}

fn converts(left: &Term, right: &Term, globals: &Admitted, budget: usize) -> Convertible {
    convertible_in(left, right, globals, budget)
}

#[test]
fn fix_unrolls_on_a_constructor() {
    let globals = length(Some(0));
    assert_eq!(
        converts(
            &length_of(list_of(&[1])),
            &Term::NatLit(1),
            &globals,
            10_000
        ),
        Convertible::Yes
    );
    assert_eq!(
        converts(
            &length_of(list_of(&[4, 5])),
            &Term::NatLit(2),
            &globals,
            10_000
        ),
        Convertible::Yes
    );
    assert!(matches!(
        converts(
            &length_of(list_of(&[1])),
            &Term::NatLit(2),
            &globals,
            10_000
        ),
        Convertible::No { .. }
    ));
}

#[test]
fn fix_does_not_unroll_on_a_variable() {
    let neutral = length_of(var("xs"));
    // No budget at all: a neutral decreasing argument costs nothing past the
    // head, and the head is not δ-unfolded.
    assert_eq!(
        whnf(&neutral, &length(Some(0)), &mut Fuel::new(0)),
        Ok(neutral)
    );
}

#[test]
fn a_fix_with_no_recorded_position_never_unrolls() {
    let globals = length(None);
    assert!(matches!(
        converts(
            &length_of(list_of(&[1])),
            &Term::NatLit(1),
            &globals,
            10_000
        ),
        Convertible::No { .. }
    ));
    // The plain name → body map records no position either.
    assert!(matches!(
        convertible(
            &length_of(list_of(&[1])),
            &Term::NatLit(1),
            &globals.bodies,
            10_000
        ),
        Convertible::No { .. }
    ));
}

/// `Λa. λx:a. λl. match l { Nil => 0, Cons(_, t) => 1 + f[a](x)(t) }`,
/// decreasing on its second term parameter.
fn second_parameter_fixpoint() -> Admitted {
    let self_call = Term::app(
        Term::app(Term::ty_app(var("f"), Type::TyVar("a".into())), var("x")),
        var("t"),
    );
    let body = Term::ty_abs(
        "a",
        Term::lambda(
            "x",
            Type::TyVar("a".into()),
            list_fn(
                "l",
                match_list(
                    var("l"),
                    Term::Zero,
                    "t",
                    Term::nat_add(Term::succ(Term::Zero), self_call),
                ),
            ),
        ),
    );
    let ty = Type::forall(
        "a",
        Type::arrow(Type::TyVar("a".into()), Type::arrow(list_type(), Type::Nat)),
    );
    Admitted::default().with("count", Term::fix("f", ty, body), Some(1))
}

#[test]
fn fix_unrolls_on_its_second_parameter() {
    let globals = second_parameter_fixpoint();
    let count = |first: Term, second: Term| {
        Term::app(
            Term::app(Term::ty_app(Term::global("count"), Type::Nat), first),
            second,
        )
    };
    assert_eq!(
        converts(
            &count(var("x"), list_of(&[7, 8])),
            &Term::NatLit(2),
            &globals,
            10_000
        ),
        Convertible::Yes
    );
    // Only the first argument is a constructor: neutral, head not unfolded.
    let first_only = count(Term::NatLit(3), var("ys"));
    assert_eq!(
        whnf(&first_only, &globals, &mut Fuel::new(0)),
        Ok(first_only)
    );
    // A partial application never unrolls, however its argument looks.
    let partial = Term::app(
        Term::ty_app(Term::global("count"), Type::Nat),
        list_of(&[1]),
    );
    assert_eq!(whnf(&partial, &globals, &mut Fuel::new(0)), Ok(partial));
}

#[test]
fn fix_unrolling_shares_the_budget() {
    // Deep enough that unrolling it outruns the budget (each cell costs
    // several steps). Kept short because every term walk recurses, and a
    // debug test thread's stack is small.
    let deep: Vec<u64> = (0..60).collect();
    assert!(matches!(
        converts(
            &length_of(list_of(&deep)),
            &Term::NatLit(60),
            &length(Some(0)),
            100
        ),
        Convertible::Stopped(EvalStopped::StepLimit { limit: 100 })
    ));
}

/// `λm. λn. if m == 0 then n else succ(f(m - 1)(n))`, decreasing on `m`.
fn add_on_first() -> Admitted {
    let recursive = Term::app(
        Term::app(
            var("f"),
            Term::NatSub(Box::new(var("m")), Box::new(Term::NatLit(1))),
        ),
        var("n"),
    );
    let body = Term::lambda(
        "m",
        Type::Nat,
        Term::lambda(
            "n",
            Type::Nat,
            Term::if_then_else(
                Term::NatEq(Box::new(var("m")), Box::new(Term::Zero)),
                var("n"),
                Term::succ(recursive),
            ),
        ),
    );
    let ty = Type::arrow(Type::Nat, Type::arrow(Type::Nat, Type::Nat));
    Admitted::default().with("add", Term::fix("f", ty, body), Some(0))
}

#[test]
fn fix_unrolls_on_a_nat_literal() {
    let globals = add_on_first();
    let add = |m: Term, n: Term| Term::app(Term::app(Term::global("add"), m), n);
    assert_eq!(
        converts(
            &add(Term::NatLit(2), var("k")),
            &Term::succ(Term::succ(var("k"))),
            &globals,
            10_000
        ),
        Convertible::Yes
    );
    let open = add(var("j"), Term::NatLit(2));
    assert_eq!(whnf(&open, &globals, &mut Fuel::new(0)), Ok(open));
}

#[test]
fn constructor_heads_are_exactly_d4s() {
    let list = list_type();
    let sum = Type::sum(Type::Unit, Type::Nat);
    for head in [
        Term::Zero,
        Term::succ(var("n")),
        Term::NatLit(3),
        Term::inl(sum.clone(), Term::Unit),
        Term::inr(sum.clone(), var("n")),
        Term::pair(var("a"), var("b")),
        Term::Unit,
        list_of(&[1]),
        list_of(&[]),
    ] {
        assert!(is_constructor_headed(&head), "{head}");
    }
    for neutral in [
        var("xs"),
        Term::fold(list.clone(), var("payload")),
        Term::app(Term::global("f"), Term::Unit),
        Term::True,
    ] {
        assert!(!is_constructor_headed(&neutral), "{neutral}");
    }
}
