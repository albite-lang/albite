//! Head-first comparison (ADR 29.9.26c D7).
//!
//! Two terms are convertible when they are syntactically equal, or when their
//! WHNFs have the same head and pairwise convertible arguments. The descent is
//! a worklist, not recursion, so a long numeral chain costs budget, never
//! stack. There is no η (§3): `λx. f(x)` and `f` have different heads.

use std::collections::HashMap;

use crate::eval::EvalStopped;
use crate::Term;

use super::{whnf, Fuel, Renaming, Unfoldings};

/// The verdict on one obligation's sides.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Reason: one verdict per comparison, never stored in bulk; boxing the mismatch only adds an allocation to the failure path
pub enum Convertible {
    Yes,
    /// The first pair the worklist met whose heads differ, for D8's note.
    No {
        first_mismatch: (Term, Term),
    },
    /// The shared budget ran out (D6): a failure, never "equal".
    Stopped(EvalStopped),
}

/// Compare `left` and `right` under `globals`, spending at most `budget`
/// steps across both sides and every sub-comparison (D6). Both sides must
/// already be renamed apart (D2).
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: every caller builds a std `HashMap`; a hasher parameter would only widen the signature both compilers share
pub fn convertible(
    left: &Term,
    right: &Term,
    globals: &HashMap<String, Term>,
    budget: usize,
) -> Convertible {
    convertible_in(left, right, globals, budget)
}

/// [`convertible`] under any [`Unfoldings`] — the kernel's admitted bodies,
/// whose self-recursive members unroll only on a constructor (ADR 2.10.26l
/// D4).
#[must_use]
pub fn convertible_in<G: Unfoldings + ?Sized>(
    left: &Term,
    right: &Term,
    globals: &G,
    budget: usize,
) -> Convertible {
    let mut fuel = Fuel::new(budget);
    let mut binders = Renaming::default();
    let mut work = vec![(left.clone(), right.clone())];
    while let Some((left, right)) = work.pop() {
        if let Err(stop) = fuel.tick() {
            return Convertible::Stopped(stop);
        }
        if left == right {
            continue;
        }
        let heads = whnf(&left, globals, &mut fuel)
            .and_then(|left| whnf(&right, globals, &mut fuel).map(|right| (left, right)));
        let (left, right) = match heads {
            Ok(heads) => heads,
            Err(stop) => return Convertible::Stopped(stop),
        };
        let Some(pairs) = argument_pairs(&left, &right, &mut binders) else {
            return Convertible::No {
                first_mismatch: (left, right),
            };
        };
        // Popped from the end, so reversed: the first argument (the eliminated
        // one, for an elimination) is compared first.
        work.extend(pairs.into_iter().rev());
    }
    Convertible::Yes
}

/// A numeral's value, whichever spelling it has.
fn numeral(term: &Term) -> Option<u64> {
    match term {
        Term::Zero => Some(0),
        Term::NatLit(n) => Some(*n),
        _ => None,
    }
}

/// Both sides of a pair, cloned out of their boxes.
fn pair_of(left: &Term, right: &Term) -> (Term, Term) {
    (left.clone(), right.clone())
}

/// A binder's body on each side, opened with one shared fresh variable.
fn opened(
    (left_binder, left_body): (&str, &Term),
    (right_binder, right_body): (&str, &Term),
    binders: &mut Renaming,
) -> (Term, Term) {
    let fresh = Term::Var(binders.fresh());
    (
        left_body.substitute(left_binder, &fresh),
        right_body.substitute(right_binder, &fresh),
    )
}

/// The argument pairs two WHNFs must agree on, or `None` if their heads
/// differ. `NatLit(n > 0)` against `Succ(x)` peels to `NatLit(n − 1)` against
/// `x`; binders are opened with one fresh variable shared by both sides.
#[allow(clippy::similar_names)] // Reason: `l`/`r` prefixes pair each side's fields positionally (`linl`/`linr`); renaming would obscure the pairing
fn argument_pairs(left: &Term, right: &Term, binders: &mut Renaming) -> Option<Vec<(Term, Term)>> {
    match (numeral(left), numeral(right)) {
        (Some(left_n), Some(right_n)) => return (left_n == right_n).then(Vec::new),
        (Some(literal), None) => return peel(literal, right, false),
        (None, Some(literal)) => return peel(literal, left, true),
        (None, None) => {}
    }
    if !same_head(left, right) {
        return None;
    }
    Some(match (left, right) {
        (Term::Lambda(lx, _, lbody), Term::Lambda(rx, _, rbody))
        | (Term::Fix(lx, _, lbody), Term::Fix(rx, _, rbody)) => {
            vec![opened((lx, lbody), (rx, rbody), binders)]
        }
        (Term::Case(lscrut, lx, linl, ly, linr), Term::Case(rscrut, rx, rinl, ry, rinr)) => vec![
            pair_of(lscrut, rscrut),
            opened((lx, linl), (rx, rinl), binders),
            opened((ly, linr), (ry, rinr), binders),
        ],
        (Term::AdtMatch(lscrut, larms), Term::AdtMatch(rscrut, rarms)) => {
            std::iter::once(pair_of(lscrut, rscrut))
                .chain(
                    larms
                        .iter()
                        .zip(rarms)
                        .map(|((_, lvar, lbody), (_, rvar, rbody))| {
                            opened((lvar, lbody), (rvar, rbody), binders)
                        }),
                )
                .collect()
        }
        (Term::NatRec(_, lzero, lsucc, ln), Term::NatRec(_, rzero, rsucc, rn))
        | (Term::NatInd(_, _, lzero, lsucc, ln), Term::NatInd(_, _, rzero, rsucc, rn)) => vec![
            pair_of(ln, rn),
            pair_of(lzero, rzero),
            pair_of(lsucc, rsucc),
        ],
        _ => children(left).into_iter().zip(children(right)).collect(),
    })
}

/// `NatLit(n)` against a non-numeral: only a `Succ` can match, and only when
/// `n > 0`. `flipped` keeps the pair in its original left/right order.
fn peel(literal: u64, other: &Term, flipped: bool) -> Option<Vec<(Term, Term)>> {
    let Term::Succ(pred) = other else {
        return None;
    };
    let smaller = Term::NatLit(literal.checked_sub(1)?);
    let pair = if flipped {
        ((**pred).clone(), smaller)
    } else {
        (smaller, (**pred).clone())
    };
    Some(vec![pair])
}

/// Every immediate sub-term, in `for_each_subterm` order.
fn children(term: &Term) -> Vec<Term> {
    let mut found = Vec::new();
    term.for_each_subterm(|child| found.push(child.clone()));
    found
}

/// Same variant and the same non-term data: names, tags, literals, arity, and
/// the types inside the term, which are compared with `==` (type-level
/// conversion is Phase 2b). Binder names are not compared — binders are
/// opened instead.
pub(super) fn same_head(left: &Term, right: &Term) -> bool {
    use Term as T;
    if std::mem::discriminant(left) != std::mem::discriminant(right) {
        return false;
    }
    match (left, right) {
        (T::Var(lname), T::Var(rname))
        | (T::Global(lname), T::Global(rname))
        | (T::StringLit(lname), T::StringLit(rname))
        | (T::TyAbs(lname, _), T::TyAbs(rname, _)) => lname == rname,
        (T::IntLit(lvalue), T::IntLit(rvalue)) => lvalue == rvalue,
        (T::Lambda(_, lty, _), T::Lambda(_, rty, _))
        | (T::Fix(_, lty, _), T::Fix(_, rty, _))
        | (T::Let(_, lty, _, _), T::Let(_, rty, _, _))
        | (T::Absurd(lty, _), T::Absurd(rty, _))
        | (T::Inl(lty, _), T::Inl(rty, _))
        | (T::Inr(lty, _), T::Inr(rty, _))
        | (T::Refl(lty, _), T::Refl(rty, _))
        | (T::Fold(lty, _), T::Fold(rty, _))
        | (T::Unfold(lty, _), T::Unfold(rty, _))
        | (T::Annot(_, lty), T::Annot(_, rty))
        | (T::TyApp(_, lty), T::TyApp(_, rty))
        | (T::NatRec(lty, ..), T::NatRec(rty, ..)) => lty == rty,
        (T::NatInd(lbinder, lty, ..), T::NatInd(rbinder, rty, ..)) => {
            lbinder == rbinder && lty == rty
        }
        (T::Subst(lty, lmotive, ..), T::Subst(rty, rmotive, ..)) => {
            lty == rty && lmotive == rmotive
        }
        (T::IntBin(lop, ..), T::IntBin(rop, ..)) => lop == rop,
        (T::AdtConstruct(lty, ltag, _), T::AdtConstruct(rty, rtag, _)) => {
            lty == rty && ltag == rtag
        }
        (T::ExternCall(lname, largs), T::ExternCall(rname, rargs)) => {
            lname == rname && largs.len() == rargs.len()
        }
        (T::AdtMatch(_, larms), T::AdtMatch(_, rarms)) => {
            larms.len() == rarms.len()
                && larms.iter().zip(rarms).all(|(larm, rarm)| larm.0 == rarm.0)
        }
        _ => true,
    }
}
