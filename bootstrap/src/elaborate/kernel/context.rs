//! Each definition's typing context: only the globals it mentions, marked
//! from admission (ADR 30.9.26f D3), and the bodies δ admitted over their
//! mention closure (ADR 2.10.26l D8).
//!
//! One builder for the census and the enforcement (ADR 2.10.26c D6): moved
//! here unchanged from `doctor check proof kernel-census`, which re-exports
//! it, so the census measures exactly the context the build enforces.
//!
//! Minimal on purpose, not only for speed: `Context::with_global` clones, so
//! binding every definition of `main.tg` for every definition would be
//! quadratic, and a global the term never mentions is never looked up, so
//! leaving it out cannot change the verdict.
//!
//! Every definition reaches it presented (ADR 3.10.26p D2,
//! [`present_definitions`]): a record is its encoding, not its name.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use albite_core::context::GlobalAdmission;
use albite_core::terms::termination::AdmissionState;
use albite_core::typecheck::{mentioned_names, DeltaAdmission, DeltaCandidate};
use albite_core::{Context, Term, Type};

use super::RecordPresentation;
use crate::elaborate::{CoreDef, DefTerminationMeta};

/// Each definition's `#[decreasing(arg)]`, by definition name: the hint the
/// kernel's `fix_admission` re-checks (ADR 2.10.26l D4).
pub type DecreasingHints = HashMap<String, String>;

/// Every name a `Var` or `Global` node in `term` spells, and every name the
/// `Eq` witnesses of `ty` and of the types `term` embeds spell — the kernel's
/// own mention set (`albite_core::typecheck::mentioned_names`), so the
/// names bound here are the names δ's closure starts from.
///
/// Both node kinds, because the kernel checks admission on the *lookup*, not
/// on the node (29.9.26e D4): a `Var` naming a global is refused as surely as
/// a `Global`. A λ-bound name is collected too; binding it as a global is
/// harmless, since the binder's own `with_term` shadows it. Witness names,
/// because the kernel types the terms inside an `Eq` in proof mode (ADR
/// 1.10.26c D7, D8), including a `Subst` motive's: a `trans` motive can name
/// a global nothing else does (ADR 1.10.26e D6).
#[must_use]
pub fn mentioned_globals(term: &Term, ty: &Type) -> BTreeSet<String> {
    mentioned_names(term, ty)
}

/// Every definition as the kernel sees it: each record name replaced by its
/// encoding (ADR 3.10.26p D2), in its type, in every type its body embeds,
/// and so in every bound global's type and every body δ admits — the census
/// and the gate bind and admit from this list, so they present alike. An
/// empty presentation borrows every definition unchanged.
#[must_use]
pub fn present_definitions<'d>(
    defs: impl IntoIterator<Item = &'d CoreDef>,
    presentation: &RecordPresentation,
) -> Vec<Cow<'d, CoreDef>> {
    defs.into_iter()
        .map(|def| {
            if presentation.is_empty() {
                Cow::Borrowed(def)
            } else {
                Cow::Owned(presentation.present_def(def))
            }
        })
        .collect()
}

/// The `#[decreasing(arg)]` of every definition that has one.
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: every caller holds the elaborator's std `HashMap`
pub fn decreasing_hints(meta: &HashMap<String, DefTerminationMeta>) -> DecreasingHints {
    meta.iter()
        .filter_map(|(name, meta)| {
            let parameter = meta.attrs.decreasing.as_ref()?;
            Some((name.clone(), parameter.name.clone()))
        })
        .collect()
}

/// δ's candidates over one build (ADR 2.10.26l D3, D8): every definition,
/// `Total` exactly when the termination report says so, with its hint (none
/// on a path that holds no termination metadata, where an ambiguous recursion
/// is then simply not admitted — never wrongly admitted). The
/// kernel decides which bodies it admits; this only hands them over.
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: every caller holds a std `HashMap`
pub fn delta_admission<'d>(
    defs: impl IntoIterator<Item = &'d CoreDef>,
    admission: &BTreeMap<String, AdmissionState>,
    hints: Option<&'d DecreasingHints>,
) -> DeltaAdmission<'d> {
    DeltaAdmission::new(defs.into_iter().map(|def| {
        let candidate = DeltaCandidate {
            ty: &def.ty,
            body: &def.term.term,
            total: admission.get(&def.name) == Some(&AdmissionState::Total),
            decreasing: hints
                .and_then(|hints| hints.get(&def.name))
                .map(String::as_str),
        };
        (def.name.clone(), candidate)
    }))
}

/// [`bind_globals`], then the bodies δ admits over the mention closure of
/// `mentioned` — the one context the census and every kernel gate check a
/// proof in, so a census row and a gate verdict cannot disagree (D8).
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: callers all use the default hasher
pub fn bind_with_bodies(
    mentioned: &BTreeSet<String>,
    defs_by_name: &HashMap<&str, &Type>,
    admission: &BTreeMap<String, AdmissionState>,
    delta: &mut DeltaAdmission<'_>,
) -> Context {
    delta.bind(&bind_globals(mentioned, defs_by_name, admission), mentioned)
}

/// Bind each mentioned name that names a definition, `Admitted` exactly when
/// the termination report says `Total` and `Opaque` otherwise — fail closed.
/// A name with no definition stays unbound, so the kernel reports it as
/// `UnboundVariable`.
#[must_use]
#[allow(clippy::implicit_hasher)] // Reason: callers all use the default hasher
pub fn bind_globals(
    mentioned: &BTreeSet<String>,
    defs_by_name: &HashMap<&str, &Type>,
    admission: &BTreeMap<String, AdmissionState>,
) -> Context {
    let mut ctx = Context::new();
    for name in mentioned {
        let Some(ty) = defs_by_name.get(name.as_str()) else {
            continue;
        };
        let marker = if admission.get(name) == Some(&AdmissionState::Total) {
            GlobalAdmission::Admitted
        } else {
            GlobalAdmission::Opaque
        };
        ctx = ctx.with_global(name.clone(), (*ty).clone(), marker);
    }
    ctx
}
