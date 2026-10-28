//! D3's and D4's reduction rules, one per function (ADR 29.9.26c).
//!
//! Each rule reduces its principal argument to WHNF and then either fires or
//! hands back the elimination, rebuilt around that reduced argument, as a
//! neutral term. A rule is called only on its own `Term` form; any other form
//! comes back neutral unchanged, never a panic.

mod fix;
#[cfg(test)]
mod fix_tests;

use std::collections::HashMap;

use crate::eval::{eval_with_env_and_limit, term_to_nat, EvalEnv, EvalStopped};
use crate::Term;

use super::{whnf, Fuel, Step, Unfoldings};

pub(super) use fix::unroll_fixpoint;

/// Steps a closed primitive over literal operands may take. One is enough for
/// every operator; the margin only guards against a representation change.
/// No rule here reduces past a numeral's range (ADR 1.10.26c D4).
const PRIMITIVE_STEP_LIMIT: usize = 64;

type RuleResult = Result<Step, EvalStopped>;

#[allow(clippy::unnecessary_wraps)] // Reason: every rule returns `RuleResult`; this is the shared "no rule fired" arm they `return`
fn unchanged(term: &Term) -> RuleResult {
    Ok(Step::Neutral(term.clone()))
}

/// δ: a global in the map unfolds to its body; any other is neutral.
pub(super) fn delta<G: Unfoldings + ?Sized>(term: &Term, globals: &G) -> RuleResult {
    let Term::Global(name) = term else {
        return unchanged(term);
    };
    Ok(globals.body(name).map_or_else(
        || Step::Neutral(term.clone()),
        |body| Step::Reduced(body.clone()),
    ))
}

/// β by name: the argument is substituted unevaluated.
pub(super) fn beta<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::App(function, argument) = term else {
        return unchanged(term);
    };
    Ok(match whnf(function, globals, fuel)? {
        Term::Lambda(x, _, body) => Step::Reduced(body.substitute(&x, argument)),
        function => Step::Neutral(Term::app(function, (**argument).clone())),
    })
}

/// Type-level β: `TyApp(TyAbs(α, b), τ) → b[α := τ]`.
pub(super) fn type_beta<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::TyApp(abstraction, ty) = term else {
        return unchanged(term);
    };
    Ok(match whnf(abstraction, globals, fuel)? {
        Term::TyAbs(alpha, body) => {
            Step::Reduced(body.substitute_type_vars(&HashMap::from([(alpha, ty.clone())])))
        }
        abstraction => Step::Neutral(Term::ty_app(abstraction, ty.clone())),
    })
}

/// ι for `unfold(fold(t))`. An `unfold` of anything else — a variable,
/// typically — is the ordinary shape of an open term, not a defect.
pub(super) fn unfold<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::Unfold(ty, folded) = term else {
        return unchanged(term);
    };
    Ok(match whnf(folded, globals, fuel)? {
        Term::Fold(_, inner) => Step::Reduced(*inner),
        folded => Step::Neutral(Term::Unfold(ty.clone(), Box::new(folded))),
    })
}

/// ι for `fst`/`snd` of a pair, whatever the pair holds.
pub(super) fn projection<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let (Term::Fst(pair) | Term::Snd(pair)) = term else {
        return unchanged(term);
    };
    let first = matches!(term, Term::Fst(_));
    Ok(match (whnf(pair, globals, fuel)?, first) {
        (Term::Pair(chosen, _), true) | (Term::Pair(_, chosen), false) => Step::Reduced(*chosen),
        (pair, true) => Step::Neutral(Term::fst(pair)),
        (pair, false) => Step::Neutral(Term::snd(pair)),
    })
}

/// ι for `case` on an injection.
pub(super) fn case<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::Case(scrutinee, x, left, y, right) = term else {
        return unchanged(term);
    };
    Ok(match whnf(scrutinee, globals, fuel)? {
        Term::Inl(_, v) => Step::Reduced(left.substitute(x, &v)),
        Term::Inr(_, v) => Step::Reduced(right.substitute(y, &v)),
        s => Step::Neutral(Term::Case(
            Box::new(s),
            x.clone(),
            left.clone(),
            y.clone(),
            right.clone(),
        )),
    })
}

/// ι for `if` on a boolean literal.
pub(super) fn conditional<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::If(cond, then_, else_) = term else {
        return unchanged(term);
    };
    Ok(match whnf(cond, globals, fuel)? {
        Term::True => Step::Reduced((**then_).clone()),
        Term::False => Step::Reduced((**else_).clone()),
        cond => Step::Neutral(Term::if_then_else(
            cond,
            (**then_).clone(),
            (**else_).clone(),
        )),
    })
}

/// ι for a flat-ADT `match` on a constructor with a matching arm.
pub(super) fn adt_match<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::AdtMatch(scrutinee, arms) = term else {
        return unchanged(term);
    };
    let scrutinee = whnf(scrutinee, globals, fuel)?;
    if let Term::AdtConstruct(_, tag, payload) = &scrutinee {
        if let Some((_, var, body)) = arms.iter().find(|(idx, _, _)| idx == tag) {
            return Ok(Step::Reduced(body.substitute(var, payload)));
        }
    }
    Ok(Step::Neutral(Term::AdtMatch(
        Box::new(scrutinee),
        arms.clone(),
    )))
}

/// ι for `subst` on `refl`.
pub(super) fn subst<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let Term::Subst(ty, motive, eq, proof) = term else {
        return unchanged(term);
    };
    Ok(match whnf(eq, globals, fuel)? {
        Term::Refl(..) => Step::Reduced((**proof).clone()),
        eq => Step::Neutral(Term::subst(
            ty.clone(),
            motive.clone(),
            eq,
            (**proof).clone(),
        )),
    })
}

/// ι for `natrec`/`natind` on a numeral head, whatever the other arguments.
pub(super) fn recursor<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let (Term::NatRec(_, zero, succ, n) | Term::NatInd(_, _, zero, succ, n)) = term else {
        return unchanged(term);
    };
    // Same node, new target: the motive (and a `natind`'s binder) ride along.
    let rebuild = |target: Term| {
        let mut rebuilt = term.clone();
        if let Term::NatRec(.., slot) | Term::NatInd(.., slot) = &mut rebuilt {
            **slot = target;
        }
        rebuilt
    };
    let pred = match whnf(n, globals, fuel)? {
        Term::Zero | Term::NatLit(0) => return Ok(Step::Reduced((**zero).clone())),
        Term::NatLit(k) => Term::NatLit(k - 1),
        Term::Succ(pred) => *pred,
        n => return Ok(Step::Neutral(rebuild(n))),
    };
    let recursive = rebuild(pred.clone());
    Ok(Step::Reduced(Term::app(
        Term::app((**succ).clone(), pred),
        recursive,
    )))
}

/// D4: primitive `+` recurses on its **second** argument (Lean 4's `Nat.add`),
/// so `k + 1 ≡ succ(k)` and `k + 0 ≡ k`, while `0 + k` stays neutral.
pub(super) fn add<G: Unfoldings + ?Sized>(term: &Term, globals: &G, fuel: &mut Fuel) -> RuleResult {
    let Term::NatAdd(a, b) = term else {
        return unchanged(term);
    };
    Ok(match whnf(b, globals, fuel)? {
        Term::Zero | Term::NatLit(0) => Step::Reduced((**a).clone()),
        Term::Succ(m) => Step::Reduced(Term::succ(Term::nat_add((**a).clone(), *m))),
        Term::NatLit(n) => add_literal(a, n, globals, fuel)?,
        b => Step::Neutral(Term::nat_add((**a).clone(), b)),
    })
}

/// `augend + NatLit(addend > 0)`: computed when `augend` is a numeral too,
/// otherwise one `Succ` is unrolled — so a long literal costs budget, never a
/// deep term. A sum past `u64::MAX` is not reduced (ADR 1.10.26c D4): a
/// saturated `MAX + 1 ≡ MAX` would let the kernel prove a false equality.
fn add_literal<G: Unfoldings + ?Sized>(
    augend: &Term,
    addend: u64,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let augend = whnf(augend, globals, fuel)?;
    let Some(value) = term_to_nat(&augend) else {
        return Ok(Step::Reduced(Term::succ(Term::nat_add(
            augend,
            Term::NatLit(addend - 1),
        ))));
    };
    Ok(match (value as u64).checked_add(addend) {
        Some(sum) => Step::Reduced(Term::NatLit(sum)),
        None => Step::Neutral(Term::nat_add(augend, Term::NatLit(addend))),
    })
}

/// ADR 30.9.26e D2: primitive `*` recurses on its **second** argument, as `+`
/// does (Lean 4's `Nat.mul`), so `m * 0 ≡ 0` and `m * succ(k) ≡ m * k + m`,
/// while `0 * m` stays neutral. `m * 0 → 0` is the one erasing rule: `m` is
/// discarded unevaluated, which is admissible because a diverging `m` can only
/// reach conversion as an opaque neutral (`#[partial]` is never unfolded).
pub(super) fn mul<G: Unfoldings + ?Sized>(term: &Term, globals: &G, fuel: &mut Fuel) -> RuleResult {
    let Term::NatMul(a, b) = term else {
        return unchanged(term);
    };
    Ok(match whnf(b, globals, fuel)? {
        Term::Zero | Term::NatLit(0) => Step::Reduced(Term::Zero),
        Term::Succ(k) => Step::Reduced(Term::nat_add(
            Term::nat_mul((**a).clone(), *k),
            (**a).clone(),
        )),
        Term::NatLit(n) => mul_literal(a, n, globals, fuel)?,
        b => Step::Neutral(Term::nat_mul((**a).clone(), b)),
    })
}

/// `multiplicand * NatLit(multiplier > 0)`: computed when `multiplicand` is a
/// numeral too; otherwise one step is unrolled — so a long literal costs
/// budget, never a deep term. A product past `u64::MAX` is not reduced (ADR
/// 1.10.26c D4), where the evaluator saturates: the rule agrees with it
/// wherever the evaluator does not saturate.
fn mul_literal<G: Unfoldings + ?Sized>(
    multiplicand: &Term,
    multiplier: u64,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let multiplicand = whnf(multiplicand, globals, fuel)?;
    let Some(value) = term_to_nat(&multiplicand) else {
        return Ok(Step::Reduced(Term::nat_add(
            Term::nat_mul(multiplicand.clone(), Term::NatLit(multiplier - 1)),
            multiplicand,
        )));
    };
    Ok(match (value as u64).checked_mul(multiplier) {
        Some(product) => Step::Reduced(Term::NatLit(product)),
        None => Step::Neutral(Term::nat_mul(multiplicand, Term::NatLit(multiplier))),
    })
}

/// Every primitive other than `+` and `*` that computes only on literal operands.
pub(super) fn is_closed_primitive_form(term: &Term) -> bool {
    matches!(
        term,
        Term::NatSub(..)
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
            | Term::IntBin(..)
            | Term::IntNeg(_)
            | Term::NatToInt(_)
            | Term::IntToNat(_)
    )
}

/// D3's closed primitives: when every operand's WHNF is a literal, compute
/// exactly as the evaluator does, by handing it the literal instance. An open
/// operand, or an operation the evaluator refuses, leaves the term neutral —
/// so do the signed operations that overflow (`IntBin` add/sub/mul, `IntNeg`
/// of the minimum, `NatToInt` above `i64::MAX`), because the evaluator traps
/// on them rather than returning a value (ADR 1.10.26c D4).
pub(super) fn closed_primitive<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> RuleResult {
    let mut literal_instance = term.clone();
    for operand in literal_instance.subterms_mut() {
        let Some(value) = literal(operand, globals, fuel)? else {
            return unchanged(term);
        };
        *operand = value;
    }
    Ok(
        match eval_with_env_and_limit(&literal_instance, &EvalEnv::empty(), PRIMITIVE_STEP_LIMIT) {
            Ok(value) if value.is_value() => Step::Reduced(value),
            _ => Step::Neutral(term.clone()),
        },
    )
}

/// `term`'s value if its WHNF is a literal (a `Succ` chain is read to the
/// end). A chain that climbs past `u64::MAX` is not a literal (ADR 1.10.26c
/// D4).
fn literal<G: Unfoldings + ?Sized>(
    term: &Term,
    globals: &G,
    fuel: &mut Fuel,
) -> Result<Option<Term>, EvalStopped> {
    let mut current = whnf(term, globals, fuel)?;
    let mut successors: u64 = 0;
    loop {
        match current {
            Term::Succ(pred) => {
                fuel.tick()?;
                successors += 1;
                current = whnf(&pred, globals, fuel)?;
            }
            Term::Zero => return Ok(Some(Term::NatLit(successors))),
            Term::NatLit(n) => return Ok(n.checked_add(successors).map(Term::NatLit)),
            Term::True | Term::False | Term::Unit | Term::StringLit(_) | Term::IntLit(_)
                if successors == 0 =>
            {
                return Ok(Some(current))
            }
            _ => return Ok(None),
        }
    }
}
