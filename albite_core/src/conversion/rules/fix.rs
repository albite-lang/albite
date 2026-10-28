//! δ's `fix` rule (ADR 2.10.26l D4): a self-recursive body the kernel
//! admitted unrolls only when the argument in its decreasing position is
//! constructor-headed — the guard Coq's ι-rule for `fix` uses.
//!
//! The kernel admits a self-recursive global `g` as `fix f. b` with the
//! decreasing position its own `fix_admission` certified. The rule fires on an
//! application spine whose head is `g`: it peels every `TyApp`, counts **term**
//! arguments only (the position indexes term parameters, as the termination
//! engine's signature does), and unrolls when the spine reaches that position
//! and the argument there reduces to a constructor. The unrolled head is
//! `b[f := g]`, which is `b[f := fix f. b]` up to one δ step on `g` — and that
//! step is guarded again by this rule, so every unrolling consumes a
//! constructor. A partial application, a neutral decreasing argument, and a
//! `fix` with no recorded position never unroll; the budget is the backstop.

use crate::eval::EvalStopped;
use crate::types::Type;
use crate::Term;

use crate::conversion::{whnf, Fuel, Step, Unfoldings};

/// One argument of an application spine, in application order.
enum SpineArgument {
    Term(Term),
    Type(Type),
}

/// The guarded unrolling of `term`, or `None` when its head is not an
/// admitted fixpoint — the caller's β then runs as before.
pub(in crate::conversion) fn unroll_fixpoint<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> Result<Option<Step>, EvalStopped> {
    let (head, mut arguments) = spine(term);
    let Term::Global(name) = head else {
        return Ok(None);
    };
    let (Some(position), Some(Term::Fix(binder, _, body))) =
        (globals.decreasing(name), globals.body(name))
    else {
        return Ok(None);
    };
    let Some(index) = term_argument_index(&arguments, position) else {
        // A partial application never unrolls (D4).
        return Ok(Some(Step::Neutral(term.clone())));
    };
    let SpineArgument::Term(decreasing) = &arguments[index] else {
        unreachable!("term_argument_index names a term argument");
    };
    let decreasing = whnf(decreasing, globals, fuel)?;
    let unrolls = is_constructor_headed(&decreasing);
    arguments[index] = SpineArgument::Term(decreasing);
    Ok(Some(if unrolls {
        let unrolled = body.substitute(binder, &Term::Global(name.clone()));
        Step::Reduced(rebuild(unrolled, arguments))
    } else {
        Step::Neutral(rebuild(head.clone(), arguments))
    }))
}

/// The head of `term`'s application spine and its arguments, in application
/// order. Span and annotation wrappers on the way down are dropped.
fn spine(term: &Term) -> (&Term, Vec<SpineArgument>) {
    let mut arguments = Vec::new();
    let mut cursor = term;
    loop {
        match cursor {
            Term::App(function, argument) => {
                arguments.push(SpineArgument::Term((**argument).clone()));
                cursor = function;
            }
            Term::TyApp(function, ty) => {
                arguments.push(SpineArgument::Type(ty.clone()));
                cursor = function;
            }
            Term::Spanned(inner, _) | Term::Annot(inner, _) => cursor = inner,
            _ => break,
        }
    }
    arguments.reverse();
    (cursor, arguments)
}

/// The spine index of the `position`-th term argument, if the spine has one.
fn term_argument_index(arguments: &[SpineArgument], position: usize) -> Option<usize> {
    arguments
        .iter()
        .enumerate()
        .filter(|(_, argument)| matches!(argument, SpineArgument::Term(_)))
        .nth(position)
        .map(|(index, _)| index)
}

/// `head` applied to `arguments`, in order.
fn rebuild(head: Term, arguments: Vec<SpineArgument>) -> Term {
    arguments
        .into_iter()
        .fold(head, |function, argument| match argument {
            SpineArgument::Term(argument) => Term::app(function, argument),
            SpineArgument::Type(ty) => Term::ty_app(function, ty),
        })
}

/// D4's constructor heads over Core, which has no `Cons` term: `Zero`,
/// `Succ`, a Nat literal, `Inl`/`Inr`, `Pair`, `Unit`, and `Fold` of an
/// `Inl`/`Inr` — the shape a user list's `Cons(1, Nil)` arrives as.
pub(super) fn is_constructor_headed(term: &Term) -> bool {
    match term {
        Term::Zero
        | Term::Succ(_)
        | Term::NatLit(_)
        | Term::Inl(..)
        | Term::Inr(..)
        | Term::Pair(..)
        | Term::Unit => true,
        Term::Fold(_, inner) => matches!(unwrapped(inner), Term::Inl(..) | Term::Inr(..)),
        _ => false,
    }
}

fn unwrapped(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) | Term::Annot(inner, _) => unwrapped(inner),
        other => other,
    }
}
