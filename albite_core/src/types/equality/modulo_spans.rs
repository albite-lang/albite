//! `Eq` witness terms compared modulo source spans (ADR 29.9.26d D6).
//!
//! A call elaborates to `Spanned(App(…), span)`, so the derived `==` on
//! `Term` never equates an `Eq` type that mentions a call with the same type
//! restated elsewhere — every `natind` IH annotation restates its motive. A
//! span carries no meaning, so this walk skips `Spanned` on either side and
//! otherwise compares node by node, embedded types by the enclosing α-rule.
//! It allocates nothing, and it only accepts more than `==`: it is not
//! conversion.

use std::collections::HashMap;
use std::mem::discriminant;

use crate::terms::Term;

use super::types_equal_with_env;

/// Whether `left` and `right` are the same term once every `Spanned` is erased.
pub(super) fn terms_equal_modulo_spans(
    left: &Term,
    right: &Term,
    env: &mut HashMap<String, String>,
) -> bool {
    let (left, right) = (peel_spans(left), peel_spans(right));
    discriminant(left) == discriminant(right)
        && labels_equal(left, right)
        && embedded_types_equal(left, right, env)
        && subterms_equal(left, right, env)
}

fn peel_spans(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) => peel_spans(inner),
        _ => term,
    }
}

/// Pairwise over the children, in `for_each_subterm` order. `labels_equal`
/// has already matched the child counts of the two variadic variants.
fn subterms_equal(left: &Term, right: &Term, env: &mut HashMap<String, String>) -> bool {
    let mut index = 0;
    let mut equal = true;
    left.for_each_subterm(|child| {
        equal = equal
            && right
                .nth_subterm(index)
                .is_some_and(|other| terms_equal_modulo_spans(child, other, env));
        index += 1;
    });
    equal && right.nth_subterm(index).is_none()
}

/// Every node carries at most two embedded types (`Subst`).
fn embedded_types_equal(left: &Term, right: &Term, env: &mut HashMap<String, String>) -> bool {
    let (mut left_types, mut right_types) = ([None; 2], [None; 2]);
    let mut count = 0;
    left.for_each_embedded_type(|ty| {
        left_types[count] = Some(ty);
        count += 1;
    });
    count = 0;
    right.for_each_embedded_type(|ty| {
        right_types[count] = Some(ty);
        count += 1;
    });
    left_types
        .iter()
        .zip(right_types.iter())
        .all(|pair| match pair {
            (Some(l), Some(r)) => types_equal_with_env(l, r, env),
            (None, None) => true,
            _ => false,
        })
}

/// The non-term, non-type payload of two same-variant nodes. Exhaustive on
/// `left`, so a new variant carrying a name or a number is a compile error
/// here rather than a label this walk silently ignores.
pub(super) fn labels_equal(left: &Term, right: &Term) -> bool {
    match (left, right) {
        (Term::Var(x), Term::Var(y))
        | (Term::Global(x), Term::Global(y))
        | (Term::StringLit(x), Term::StringLit(y))
        | (Term::Lambda(x, ..), Term::Lambda(y, ..))
        | (Term::Fix(x, ..), Term::Fix(y, ..))
        | (Term::Let(x, ..), Term::Let(y, ..))
        | (Term::TyAbs(x, _), Term::TyAbs(y, _))
        | (Term::NatInd(x, ..), Term::NatInd(y, ..)) => x == y,
        (Term::ExternCall(x, xs), Term::ExternCall(y, ys)) => x == y && xs.len() == ys.len(),
        (Term::NatLit(x), Term::NatLit(y)) => x == y,
        (Term::IntLit(x), Term::IntLit(y)) => x == y,
        (Term::IntBin(x, ..), Term::IntBin(y, ..)) => x == y,
        (Term::AdtConstruct(_, x, _), Term::AdtConstruct(_, y, _)) => x == y,
        (Term::Case(_, x1, _, x2, _), Term::Case(_, y1, _, y2, _)) => x1 == y1 && x2 == y2,
        (Term::AdtMatch(_, xs), Term::AdtMatch(_, ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys.iter())
                    .all(|((xi, xv, _), (yi, yv, _))| xi == yi && xv == yv)
        }
        _ => label_free(left),
    }
}

/// The variants with no label: a same-discriminant pair of these is equal
/// here. A labelled variant reaching this arm is a mismatched pair.
fn label_free(term: &Term) -> bool {
    match term {
        Term::Var(_)
        | Term::Global(_)
        | Term::StringLit(_)
        | Term::Lambda(..)
        | Term::Fix(..)
        | Term::Let(..)
        | Term::TyAbs(..)
        | Term::NatInd(..)
        | Term::ExternCall(..)
        | Term::NatLit(_)
        | Term::IntLit(_)
        | Term::IntBin(..)
        | Term::AdtConstruct(..)
        | Term::Case(..)
        | Term::AdtMatch(..) => false,
        Term::True
        | Term::False
        | Term::Unit
        | Term::Zero
        | Term::Sorry
        | Term::App(..)
        | Term::If(..)
        | Term::Absurd(..)
        | Term::Succ(_)
        | Term::NatRec(..)
        | Term::NatAdd(..)
        | Term::NatSub(..)
        | Term::NatMul(..)
        | Term::NatDiv(..)
        | Term::NatMod(..)
        | Term::NatEq(..)
        | Term::NatLt(..)
        | Term::NatLe(..)
        | Term::NatGt(..)
        | Term::NatGe(..)
        | Term::BoolAnd(..)
        | Term::BoolOr(..)
        | Term::BoolNot(_)
        | Term::StrConcat(..)
        | Term::StrLen(_)
        | Term::StrEq(..)
        | Term::StrCharAt(..)
        | Term::StrSubstring(..)
        | Term::Pair(..)
        | Term::Fst(_)
        | Term::Snd(_)
        | Term::Inl(..)
        | Term::Inr(..)
        | Term::TyApp(..)
        | Term::Refl(..)
        | Term::Subst(..)
        | Term::Fold(..)
        | Term::Unfold(..)
        | Term::RefNew(_)
        | Term::RefGet(_)
        | Term::RefSet(..)
        | Term::Annot(..)
        | Term::Return(_)
        | Term::Spanned(..)
        | Term::IntNeg(_)
        | Term::NatToInt(_)
        | Term::IntToNat(_)
        | Term::Region(..)
        | Term::CallFrame(_) => true,
    }
}
