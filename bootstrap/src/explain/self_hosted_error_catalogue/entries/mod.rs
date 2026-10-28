//! `SelfHostedErrorEntry` data table — the self-hosted compiler's error codes,
//! split out of the parent module as pure data (ADR 24.7.26e review), and
//! by code block into the siblings below when it reached its line cap
//! (ADR 6.10.26j D4).

#[derive(Clone, Copy)]
pub(crate) struct SelfHostedErrorEntry {
    pub(crate) code: &'static str,
    pub(crate) name: &'static str,
    pub(crate) category: &'static str,
    pub(crate) description: &'static str,
}

mod items_and_patterns;
mod proofs_and_gates;
mod syntax_and_internal;
mod typing_and_names;

use std::sync::LazyLock;

/// Every entry, in code order: the four blocks, concatenated once.
pub(super) static SELF_HOSTED_ERRORS: LazyLock<Vec<SelfHostedErrorEntry>> = LazyLock::new(|| {
    [
        typing_and_names::TYPING_AND_NAMES,
        items_and_patterns::ITEMS_AND_PATTERNS,
        proofs_and_gates::PROOFS_AND_GATES,
        syntax_and_internal::SYNTAX_AND_INTERNAL,
    ]
    .concat()
});
