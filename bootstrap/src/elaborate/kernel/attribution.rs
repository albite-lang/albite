//! Which tactic a kernel refusal names (ADR 2.10.26c D8). Pure: the checker
//! is injected, so the walk is tested without a kernel.

use crate::span::Span;

/// The step a refusal is attributed to: a tactic's name and span, or — when
/// no single step is refused but the whole proof is, a context fault — `by`
/// and the block's first tactic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Culprit {
    pub(crate) tactic: String,
    pub(crate) span: Span,
}

/// The name a refusal no single step owns is attributed to.
pub(crate) const WHOLE_BLOCK: &str = "by";

/// Walk `steps` from last to first and name the first one `refused` — each
/// step checked by its index. It is a **minimal** refused step: every step
/// checked before it was accepted, and those are the steps whose contributions
/// it can contain. When none is refused, the block is named, at its first
/// step — or at `block_span` when it has none.
pub(crate) fn attribute(
    steps: &[(&str, Span)],
    block_span: Span,
    mut refused: impl FnMut(usize) -> bool,
) -> Culprit {
    for index in (0..steps.len()).rev() {
        if refused(index) {
            let (tactic, span) = steps[index];
            return Culprit {
                tactic: tactic.to_string(),
                span,
            };
        }
    }
    Culprit {
        tactic: WHOLE_BLOCK.to_string(),
        span: steps.first().map_or(block_span, |(_, span)| *span),
    }
}
