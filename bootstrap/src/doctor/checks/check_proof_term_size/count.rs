//! The count behind `doctor check proof term-size` (ADR 13.10.26c D3, D4): a
//! pure walk over one definition's term.

use albite_core::Term;

/// How large one proof body is (D3). A `Global` cited twice counts twice:
/// `global_occurrences` is not a distinct-dependency count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProofTermSize {
    pub subst: usize,
    pub refl: usize,
    pub global_occurrences: usize,
    pub nodes: usize,
    /// Match width (ADR 15.10.26g D3): each `AdtMatch`'s arm count plus 2 per
    /// `Case` (a binary peel), nested ones summed — a definition's exposure
    /// to a padding change, not its padding.
    pub case_arms: usize,
}

impl ProofTermSize {
    /// The field-wise sum, for the totals line.
    #[must_use]
    pub fn plus(self, other: ProofTermSize) -> ProofTermSize {
        ProofTermSize {
            subst: self.subst + other.subst,
            refl: self.refl + other.refl,
            global_occurrences: self.global_occurrences + other.global_occurrences,
            nodes: self.nodes + other.nodes,
            case_arms: self.case_arms + other.case_arms,
        }
    }
}

/// Count `term` as the elaborator emitted it — nothing normalised or unfolded.
/// Only `Term`-valued children are visited (`Term::for_each_subterm`), so the
/// `Type`s a term carries — binder annotations, a `Subst`'s motive and
/// carrier — never contribute: the metric is the proof's size, not its
/// statement's (D3). Every node visited counts, `Spanned` wrappers included.
#[must_use]
pub fn count_proof_nodes(term: &Term) -> ProofTermSize {
    let mut size = ProofTermSize::default();
    count_into(term, &mut size);
    size
}

fn count_into(term: &Term, size: &mut ProofTermSize) {
    size.nodes += 1;
    match term {
        Term::Subst(..) => size.subst += 1,
        Term::Refl(..) => size.refl += 1,
        Term::Global(_) => size.global_occurrences += 1,
        Term::Case(..) => size.case_arms += 2,
        Term::AdtMatch(_, arms) => size.case_arms += arms.len(),
        _ => {}
    }
    term.for_each_subterm(|child| count_into(child, size));
}
