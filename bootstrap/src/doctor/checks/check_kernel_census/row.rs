//! One definition through the kernel: D5's two passes and D6's classifier
//! (ADR 30.9.26f).
//!
//! The probes come through [`ProbeEngine`], so the two-pass logic is tested
//! with a fake engine and no elaboration.

use albite_core::typecheck::{check, check_proof};
use albite_core::{Context, Term, Type, TypeError};

use super::{classify, Cause, Probes};

/// Computes D6's probes for one kernel refusal. The shell's implementation
/// reads the project output and the conversion engine.
pub trait ProbeEngine {
    /// The probes `refusal` can use; `Probes::default()` (all `None`) for an
    /// error no probed row can match.
    fn probes_for(&self, refusal: &TypeError) -> Probes;
}

/// One definition's census outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusRow {
    /// The definition's name.
    pub name: String,
    /// The proof-mode outcome: its first refusal, or `admitted`/`axiom`.
    pub primary: Cause,
    /// Executable mode's outcome, computed only when the primary masks the
    /// rest of the proof (D5).
    pub secondary: Option<Cause>,
    /// The proof-mode refusal, rendered on one line.
    pub refusal: Option<String>,
}

impl CensusRow {
    /// An axiom's row: counted, not checked (D3).
    #[must_use]
    pub fn axiom(name: &str) -> Self {
        CensusRow {
            name: name.to_string(),
            primary: Cause::Axiom,
            secondary: None,
            refusal: None,
        }
    }
}

/// Check `term : ty` under the proof judgment; when that refusal stops the
/// derivation before the rest is looked at, check it again in executable mode
/// and classify that outcome as the secondary cause (D5).
#[must_use]
pub fn census_definition(
    name: &str,
    ctx: &Context,
    term: &Term,
    ty: &Type,
    probes: &dyn ProbeEngine,
) -> CensusRow {
    let Err(refusal) = check_proof(ctx, term, ty) else {
        return CensusRow {
            name: name.to_string(),
            primary: Cause::Admitted,
            secondary: None,
            refusal: None,
        };
    };
    let primary = classify(&refusal, &probes.probes_for(&refusal));
    let secondary = primary
        .masks_the_rest()
        .then(|| match check(ctx, term, ty) {
            Ok(()) => Cause::Admitted,
            Err(error) => classify(&error, &probes.probes_for(&error)),
        });
    CensusRow {
        name: name.to_string(),
        primary,
        secondary,
        refusal: Some(refusal.to_string()),
    }
}
