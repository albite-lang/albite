//! The proof state: goals with their contexts, metavariables as holes, and
//! syntactic grafting (ADR 2.10.26c D2). Pure data — nothing here elaborates.
//!
//! A goal is a metavariable. A tactic solves the focused one with a
//! [`Partial`] — Core with a named hole per sub-goal — and the holes' metas
//! become the new goals, at the front. `zonk` then grafts each solution into
//! its hole **syntactically**: a hole under `λh` is filled by a term that
//! mentions that very `h`, and capture-avoiding substitution would rename the
//! binder and make the proof wrong.

use std::collections::BTreeMap;

use albite_core::{Term, Type};

/// A metavariable's index in [`ProofState::metas`].
pub(crate) type MetaId = usize;

/// The prefix of every hole spelling. `%` fails the lexer's `is_ident_start`,
/// so no source name can be one, and none of the elaborator's synthesised
/// spellings (`@`, `α_`, `#x`, `?`, `′`) begins with it (P0.4).
pub(crate) const HOLE_PREFIX: &str = "%goal";

/// The spelling of `meta`'s hole.
#[must_use]
pub(crate) fn hole_name(meta: MetaId) -> String {
    format!("{HOLE_PREFIX}{meta}")
}

/// One goal: the locals `intro` added (the theorem's own parameters are in
/// the elaborator's scope already), the target, and — once a tactic ran on
/// it — its solution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Meta {
    pub(crate) locals: Vec<(String, Type)>,
    pub(crate) target: Type,
    pub(crate) solution: Option<Partial>,
}

/// Core with a hole per sub-goal: `holes` maps each hole's spelling to the
/// meta that fills it, in the order those metas become goals.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Partial {
    pub(crate) template: Term,
    pub(crate) holes: Vec<(String, MetaId)>,
}

impl Partial {
    /// A complete term: no sub-goals (`exact`, `refl`).
    #[must_use]
    pub(crate) fn closed(term: Term) -> Self {
        Partial {
            template: term,
            holes: Vec::new(),
        }
    }
}

/// Why `zonk` could not produce a term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnsolvedMeta(pub(crate) MetaId);

/// The open goals, focus first, and every meta ever made.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProofState {
    pub(crate) goals: Vec<MetaId>,
    pub(crate) metas: Vec<Meta>,
}

impl ProofState {
    /// One goal, `target`, with no locals. Its meta is the root, `0`.
    #[must_use]
    pub(crate) fn new(target: Type) -> Self {
        ProofState {
            goals: vec![0],
            metas: vec![Meta {
                locals: Vec::new(),
                target,
                solution: None,
            }],
        }
    }

    /// The focused goal, if any is left.
    #[must_use]
    pub(crate) fn focus(&self) -> Option<MetaId> {
        self.goals.first().copied()
    }

    /// A new unsolved meta, not yet a goal: a `Partial`'s hole becomes one
    /// when [`ProofState::refine`] installs it.
    pub(crate) fn fresh_meta(&mut self, locals: Vec<(String, Type)>, target: Type) -> MetaId {
        self.metas.push(Meta {
            locals,
            target,
            solution: None,
        });
        self.metas.len() - 1
    }

    /// Solve the focused goal `goal` with `partial`, and put its holes' metas,
    /// in order, at the front of the goal list. Every tactic is this call.
    pub(crate) fn refine(&mut self, goal: MetaId, partial: Partial) {
        debug_assert_eq!(self.focus(), Some(goal), "only the focused goal is refined");
        let mut goals: Vec<MetaId> = partial.holes.iter().map(|(_, meta)| *meta).collect();
        goals.extend(self.goals.iter().copied().filter(|g| *g != goal));
        self.goals = goals;
        self.metas[goal].solution = Some(partial);
    }

    /// `meta`'s complete term, every hole grafted.
    ///
    /// # Errors
    ///
    /// The first unsolved meta reached.
    pub(crate) fn zonk(&self, meta: MetaId) -> Result<Term, UnsolvedMeta> {
        self.zonk_with(meta, &BTreeMap::new())
    }

    /// [`ProofState::zonk`], with `overrides` standing in for the named metas'
    /// solutions — how the kernel boundary abstracts a `sorry` (D8).
    ///
    /// # Errors
    ///
    /// The first unsolved meta reached that no override covers.
    pub(crate) fn zonk_with(
        &self,
        meta: MetaId,
        overrides: &BTreeMap<MetaId, Term>,
    ) -> Result<Term, UnsolvedMeta> {
        if let Some(term) = overrides.get(&meta) {
            return Ok(term.clone());
        }
        let Some(partial) = &self.metas[meta].solution else {
            return Err(UnsolvedMeta(meta));
        };
        let mut solved = BTreeMap::new();
        for (spelling, hole) in &partial.holes {
            solved.insert(spelling.as_str(), self.zonk_with(*hole, overrides)?);
        }
        Ok(graft(&partial.template, &solved))
    }
}

/// Replace each hole `Var` in `template` by its solution, renaming nothing.
///
/// Templates place holes only under the nodes the engine assembles — a `λ`,
/// an application, a span, `rewrite`'s `Subst` (ADR 2.10.26e D4),
/// `induction`'s `NatInd` cases (ADR 3.10.26e D2), `cases`'s annotated
/// `Case`/`AdtMatch` arms (ADR 3.10.26f D2), `exists`'s annotated `Pair`
/// (ADR 6.10.26e D4) and `left`/`right`'s `Inl`/`Inr` (ADR 7.10.26l D2) —
/// so only those are walked;
/// everything else is elaborator output, which cannot spell a hole. A later
/// tactic that builds a new shape around a hole adds it here.
fn graft(template: &Term, solved: &BTreeMap<&str, Term>) -> Term {
    match template {
        Term::Var(name) => solved
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| template.clone()),
        Term::Lambda(binder, ty, body) => Term::lambda(binder, ty.clone(), graft(body, solved)),
        Term::App(head, arg) => Term::app(graft(head, solved), graft(arg, solved)),
        Term::Spanned(inner, span) => Term::Spanned(Box::new(graft(inner, solved)), *span),
        Term::Subst(base, motive, eq, proof) => Term::subst(
            base.clone(),
            motive.clone(),
            (**eq).clone(),
            graft(proof, solved),
        ),
        Term::NatInd(binder, motive, base, step, scrutinee) => Term::natind(
            binder.clone(),
            motive.clone(),
            graft(base, solved),
            graft(step, solved),
            (**scrutinee).clone(),
        ),
        Term::Annot(inner, ty) => Term::annot(graft(inner, solved), ty.clone()),
        Term::Pair(first, second) => Term::pair(graft(first, solved), graft(second, solved)),
        Term::Inl(ty, inner) => Term::inl(ty.clone(), graft(inner, solved)),
        Term::Inr(ty, inner) => Term::inr(ty.clone(), graft(inner, solved)),
        Term::Case(scrutinee, left_binder, left, right_binder, right) => Term::case(
            (**scrutinee).clone(),
            left_binder.clone(),
            graft(left, solved),
            right_binder.clone(),
            graft(right, solved),
        ),
        Term::AdtMatch(scrutinee, arms) => Term::adt_match(
            (**scrutinee).clone(),
            arms.iter()
                .map(|(index, binder, body)| {
                    (*index, binder.clone(), Box::new(graft(body, solved)))
                })
                .collect(),
        ),
        _ => template.clone(),
    }
}
