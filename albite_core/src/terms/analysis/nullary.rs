//! Nullary-construction predicates (ADR 29.9.26h D3).
//!
//! A heap fold whose value is a constructor with an empty payload — `LNil()`,
//! `TMid()` — is what B19's nullary arm would stop allocating. Codegen asks
//! these at the fold to stamp such allocations with their own profiler class,
//! so a profiled run counts them. The definition is the one
//! `tungsten doctor check ir small-adts` applies to IR (ADR 28.9.26f): a
//! payload is empty when it is `()` or an injection of an empty payload.
//!
//! Pure over a `Term`, so it is tested and mutated without LLVM. A term it
//! cannot see through — a `Var` bound to a construction — is not nullary.

use crate::terms::Term;
use crate::types::Type;

/// Peel the wrappers that carry no value: source spans and annotations.
fn peel(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) | Term::Annot(inner, _) => peel(inner),
        other => other,
    }
}

/// `()`, or an `inl`/`inr` of an empty payload — the census's transitive
/// clause, so `inr (inl ())` is empty and `inr (inl n)` is not.
#[must_use]
pub fn is_empty_payload(term: &Term) -> bool {
    match peel(term) {
        Term::Unit => true,
        Term::Inl(_, payload) | Term::Inr(_, payload) => is_empty_payload(payload),
        _ => false,
    }
}

/// A constructor application whose payload is empty: `adt_construct … k ()`,
/// `inl … ()`, `inr … (inl … ())`. A bare `()` is a payload, not a
/// construction, and is not nullary.
#[must_use]
pub fn is_nullary_construction(term: &Term) -> bool {
    match peel(term) {
        Term::AdtConstruct(_, _, payload) | Term::Inl(_, payload) | Term::Inr(_, payload) => {
            is_empty_payload(payload)
        }
        _ => false,
    }
}

/// The tags of a nullary construction, outermost first (ADR 1.10.26g D4):
/// `[k]` for `adt_construct … k ()`, `[0]` for `inl ()`, `[1, 0]` for
/// `inr (inl ())`. `Some` exactly when [`is_nullary_construction`] holds —
/// save a constructor index past `u32::MAX`, which no `i32` tag can store.
/// These are the bytes of the shared cell codegen emits in place of the fold.
#[must_use]
pub fn nullary_tag_path(term: &Term) -> Option<Vec<u32>> {
    let (outer_tag, payload) = match peel(term) {
        Term::AdtConstruct(_, index, payload) => (u32::try_from(*index).ok()?, payload),
        Term::Inl(_, payload) => (0, payload),
        Term::Inr(_, payload) => (1, payload),
        _ => return None,
    };
    let mut path = vec![outer_tag];
    path.extend(empty_payload_path(payload)?);
    Some(path)
}

/// The nested tags of an empty payload, outermost first: `[]` for `()`,
/// `[0]` for `inl ()`, `[1, 0]` for `inr (inl ())`. `Some` exactly when
/// [`is_empty_payload`] holds.
#[must_use]
pub fn empty_payload_path(term: &Term) -> Option<Vec<u32>> {
    let (tag, payload) = match peel(term) {
        Term::Unit => return Some(Vec::new()),
        Term::Inl(_, payload) => (0, payload),
        Term::Inr(_, payload) => (1, payload),
        _ => return None,
    };
    let mut path = vec![tag];
    path.extend(empty_payload_path(payload)?);
    Some(path)
}

/// Whether any injection in a construction's chain is annotated with a μ
/// type. Such an injection lowers to a heap POINTER, not a by-value tagged
/// struct, so the tag path does not describe the cell's bytes and codegen
/// must decline it (ADR 1.10.26g D1).
#[must_use]
pub fn injects_into_mu(term: &Term) -> bool {
    match peel(term) {
        Term::AdtConstruct(_, _, payload) => injects_into_mu(payload),
        Term::Inl(ty, payload) | Term::Inr(ty, payload) => {
            matches!(ty, Type::Mu(_, _)) || injects_into_mu(payload)
        }
        _ => false,
    }
}
