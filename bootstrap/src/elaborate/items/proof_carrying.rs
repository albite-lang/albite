//! Which `fn`s the kernel checks like a theorem (ADR 13.10.26t D1).
//!
//! The kernel binds a cited global at its declared type and types its body
//! only when δ unfolds it. A `fn` whose type holds no proposition is safe to
//! cite unchecked: nothing a theorem can take out of it is a proof. One whose
//! type holds a proposition anywhere — its result, a side of a pair or sum, a
//! constructor's field — is not, because a `let` or a `match` takes the proof
//! back out and the theorem then rests on a body nobody checked. So the set is
//! read from the type, not its sort: `(Eq<…>, Nat)` sorts `Type₀` and still
//! carries one.
//!
//! Lives under `items/` because `kernel/mod.rs` has no line to spare for a
//! `mod` declaration; the elaborator and the kernel census both call it. Its
//! tests are `kernel::tests::proof_fn`, beside the gate it feeds.

use std::collections::BTreeSet;

use albite_core::Type;

/// Whether `ty` has a `Prop`-sorted component anywhere a value of it can hand
/// one back: under a function's codomain (never its domain, which a caller
/// supplies), in either side of a product, sum or sigma, inside a μ, in an
/// ADT's arguments or constructor payloads, or in the encoding `encoding_of`
/// gives a named type — a record reaches elaboration as a bare name.
///
/// A type variable with no encoding — a type parameter, a bound μ marker — is
/// no proposition: a generic `T` result is outside the set (D1).
pub(crate) fn carries_proof<'t>(ty: &Type, encoding_of: impl Fn(&str) -> Option<&'t Type>) -> bool {
    let mut seen = BTreeSet::new();
    walk(ty, &encoding_of, &mut seen)
}

fn walk<'t>(
    ty: &Type,
    encoding_of: &impl Fn(&str) -> Option<&'t Type>,
    seen: &mut BTreeSet<String>,
) -> bool {
    match ty {
        Type::Eq(..) | Type::Void | Type::PredApp(..) => true,
        Type::Arrow(_, codomain)
        | Type::Pi(_, _, codomain)
        | Type::Forall(_, codomain)
        | Type::ForallPred(_, _, codomain)
        | Type::Mu(_, codomain)
        | Type::Ptr(codomain)
        | Type::Ref(codomain) => walk(codomain, encoding_of, seen),
        Type::Product(left, right) | Type::Sum(left, right) | Type::Sigma(_, left, right) => {
            walk(left, encoding_of, seen) || walk(right, encoding_of, seen)
        }
        Type::Adt(_, args, variants) => {
            args.iter().any(|arg| walk(arg, encoding_of, seen))
                || variants
                    .iter()
                    .any(|(_, payload)| walk(payload, encoding_of, seen))
        }
        Type::App(_, args) => args.iter().any(|arg| walk(arg, encoding_of, seen)),
        Type::TyVar(name) => walk_named(name, encoding_of, seen),
        Type::Bool
        | Type::Nat
        | Type::Int
        | Type::Unit
        | Type::String
        | Type::Error
        | Type::PredAbs(..) => false,
    }
}

/// A named type through its encoding, each name read once: a recursive
/// record's encoding names itself, and a second visit can add nothing. The
/// collection-time `@` spelling and the bare one are the same type here.
fn walk_named<'t>(
    name: &str,
    encoding_of: &impl Fn(&str) -> Option<&'t Type>,
    seen: &mut BTreeSet<String>,
) -> bool {
    let bare = name.strip_prefix('@').unwrap_or(name);
    if !seen.insert(bare.to_string()) {
        return false;
    }
    encoding_of(bare)
        .or_else(|| encoding_of(name))
        .is_some_and(|encoding| walk(encoding, encoding_of, seen))
}
