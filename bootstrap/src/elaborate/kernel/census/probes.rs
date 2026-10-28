//! D6's probes over one elaborated project (ADR 30.9.26f), shared by the
//! census and the build's kernel gate (ADR 2.10.26d D10).
//!
//! Building one decides δ admission for every definition (ADR 2.10.26l D8),
//! so the gate builds it lazily, on a build's first refusal (D9): a build
//! with nothing refused never pays for it.

use std::collections::{HashMap, HashSet};

use albite_core::conversion::{convertible_in, rename_apart, Convertible, Unfoldings};
use albite_core::typecheck::{AdmittedUnfoldings, DeltaAdmission};
use albite_core::{types_equal_alpha, Term, Type, TypeError};

use super::classify::{mismatch_pair, Probes};
use crate::elaborate::conversion::CONVERSION_STEP_BUDGET;

/// The probes' view of a project: its concrete type names and the bodies δ
/// may unfold.
pub struct ElaboratedProbes {
    pub(crate) concrete_type_names: HashSet<String>,
    /// The bodies the kernel itself admitted (ADR 2.10.26l D8) — the set
    /// enforcement unfolds, not a parallel approximation of it.
    pub(crate) unfoldable: AdmittedUnfoldings,
}

impl ElaboratedProbes {
    /// The probes over `delta`'s program, unfolding exactly what the kernel
    /// admits.
    #[must_use]
    pub fn new(delta: &mut DeltaAdmission<'_>, concrete_type_names: HashSet<String>) -> Self {
        ElaboratedProbes {
            concrete_type_names,
            unfoldable: delta_candidates(delta),
        }
    }

    /// Whether the conversion engine equates two types. Conversion compares
    /// terms, so it can only equate two `Eq`s: element types α-equal and both
    /// endpoint pairs convertible. It looks through a `Pi` or an arrow whose
    /// binder and domain agree, because a stated lemma's `Eq` sits under its
    /// parameters and the engine converts under a free variable (29.9.26c).
    /// Any other pair is `false`.
    #[must_use]
    pub fn converts(&self, left: &Type, right: &Type) -> bool {
        converts_under(left, right, &self.unfoldable)
    }

    /// [`ElaboratedProbes::converts`] with an empty δ map — D5's refinement
    /// of `needs-conversion` into `needs-delta` (ADR 2.10.26d).
    #[must_use]
    pub fn converts_without_delta(&self, left: &Type, right: &Type) -> bool {
        converts_under(left, right, &HashMap::<String, Term>::new())
    }

    /// The probes `refusal` can use; `Probes::default()` for an error no
    /// probed row can match.
    #[must_use]
    pub fn probes_for(&self, refusal: &TypeError) -> Probes {
        if let Some((left, right)) = mismatch_pair(refusal) {
            return Probes {
                lenient_equal: Some(types_equal_alpha(left, right)),
                converts: Some(self.converts(left, right)),
                ..Probes::default()
            };
        }
        let names_concrete = |name: &str| self.concrete_type_names.contains(name);
        match refusal {
            TypeError::UnboundTypeVar(name) => Probes {
                marker: Some(is_marker(name)),
                names_concrete_type: Some(names_concrete(name)),
                ..Probes::default()
            },
            // FREE type variables only: the `α_List` of `μα_List. …` is bound
            // by its own `μ`, so it is a binder, not a marker (D6).
            TypeError::MalformedType(ty) => {
                let free = ty.free_type_vars();
                Probes {
                    marker: Some(free.iter().any(|name| is_marker(name))),
                    names_concrete_type: Some(free.iter().any(|name| names_concrete(name))),
                    ..Probes::default()
                }
            }
            _ => Probes::default(),
        }
    }
}

fn converts_under<U: Unfoldings>(left: &Type, right: &Type, unfoldable: &U) -> bool {
    match (left, right) {
        (Type::Eq(left_ty, left_lhs, left_rhs), Type::Eq(right_ty, right_lhs, right_rhs)) => {
            types_equal_alpha(left_ty, right_ty)
                && terms_convert(left_lhs, right_lhs, unfoldable)
                && terms_convert(left_rhs, right_rhs, unfoldable)
        }
        (Type::Pi(left_var, left_dom, left_cod), Type::Pi(right_var, right_dom, right_cod)) => {
            left_var == right_var
                && types_equal_alpha(left_dom, right_dom)
                && converts_under(left_cod, right_cod, unfoldable)
        }
        (Type::Arrow(left_dom, left_cod), Type::Arrow(right_dom, right_cod)) => {
            types_equal_alpha(left_dom, right_dom)
                && converts_under(left_cod, right_cod, unfoldable)
        }
        _ => false,
    }
}

fn terms_convert<U: Unfoldings>(left: &Term, right: &Term, unfoldable: &U) -> bool {
    let (left, right) = rename_apart(left, right);
    convertible_in(&left, &right, unfoldable, CONVERSION_STEP_BUDGET) == Convertible::Yes
}

/// The bodies δ may unfold: the set the kernel's own admission (`admit_body`,
/// ADR 2.10.26l D3) admits — `Total`, effect-free and typed by the kernel —
/// with each self-recursive one's certified decreasing position.
pub(crate) fn delta_candidates(delta: &mut DeltaAdmission<'_>) -> AdmittedUnfoldings {
    delta.admit_all()
}

/// `@` and `α_` — the elaborator's internal type-variable spellings, the same
/// split `Type::has_mono_blocking_tyvar` makes.
fn is_marker(name: &str) -> bool {
    name.starts_with('@') || name.starts_with("α_")
}
