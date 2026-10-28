//! The census's cause taxonomy and its tally (ADR 30.9.26f D6, D1).
//!
//! Pure: [`classify`] reads a kernel refusal and the probe results the shell
//! computed for it, and decides nothing else. Every branch is therefore
//! testable with injected values, and the taxonomy cannot drift into prose.
//!
//! Moved here from `doctor check proof kernel-census`, which re-exports it,
//! so the census and the build's kernel gate share one classifier (ADR
//! 2.10.26d D10). [`register_class`] is the gate's refinement on top (D5).

use std::collections::BTreeMap;
use std::process::ExitCode;

use albite_core::{Type, TypeError};

use crate::elaborate::kernel::RegisterClass;

/// Why a definition typed, or did not, in the kernel's proof mode.
///
/// Declaration order is D6's table order, which is also first-match order in
/// [`classify`] and the row order of the rendered table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cause {
    /// Row 0: `check_proof` accepted it.
    Admitted,
    /// Row 0: an axiom — counted, never checked (D3).
    Axiom,
    /// Row 1: a `sorry` hole (`SorryInProof`).
    Hole,
    /// Row 2: a `return`, or a `fix` admission refused (`ReturnInProof`,
    /// `FixNotStructural`).
    ExecutableConstruct,
    /// Row 3: a poisoned type (`PoisonInProof`).
    Poison,
    /// Row 4: a cited definition the admission report did not certify `Total`.
    GlobalNotAdmitted,
    /// Row 5: a mentioned name with no `CoreDef` to bind (D3).
    MissingGlobalContext,
    /// Row 6: an elaborator-internal `@`/`α_` type-variable spelling.
    MarkerIllFormed,
    /// Row 7: a bare type variable naming a concrete ADT or record.
    NamedTypeReference,
    /// Row 8: a mismatch the executable judgment's lenient equality accepts.
    StrictEquality,
    /// Row 9: the kernel's `Subst`, which returned bare `Prop`. Kept so the
    /// table's rows keep their order; nothing classifies into it since
    /// the `Prop` type went (ADR 4.10.26i D2).
    DependentSubst,
    /// Row 10: a mismatch the bootstrap's conversion engine equates.
    NeedsConversion,
    /// Row 11: a mismatch no probe explains.
    UnexplainedMismatch,
    /// Row 12: a predicate binder Q6 rules out, or one misused (ADR
    /// 6.10.26h D6): the refusal is the kernel's verdict on the statement's
    /// shape, not a gap in its rules.
    PredicateBinder,
    /// Row 13: any other refusal.
    Unclassified,
}

impl Cause {
    /// Every cause, in table order.
    pub const ALL: [Cause; 15] = [
        Cause::Admitted,
        Cause::Axiom,
        Cause::Hole,
        Cause::ExecutableConstruct,
        Cause::Poison,
        Cause::GlobalNotAdmitted,
        Cause::MissingGlobalContext,
        Cause::MarkerIllFormed,
        Cause::NamedTypeReference,
        Cause::StrictEquality,
        Cause::DependentSubst,
        Cause::NeedsConversion,
        Cause::UnexplainedMismatch,
        Cause::PredicateBinder,
        Cause::Unclassified,
    ];

    /// The kebab-case name the report and the record print.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Cause::Admitted => "admitted",
            Cause::Axiom => "axiom",
            Cause::Hole => "hole",
            Cause::ExecutableConstruct => "executable-construct",
            Cause::Poison => "poison",
            Cause::GlobalNotAdmitted => "global-not-admitted",
            Cause::MissingGlobalContext => "missing-global-context",
            Cause::MarkerIllFormed => "marker-ill-formed",
            Cause::NamedTypeReference => "named-type-reference",
            Cause::StrictEquality => "strict-equality",
            Cause::DependentSubst => "dependent-subst",
            Cause::NeedsConversion => "needs-conversion",
            Cause::UnexplainedMismatch => "unexplained-mismatch",
            Cause::PredicateBinder => "predicate-binder",
            Cause::Unclassified => "unclassified",
        }
    }

    /// Whether this refusal stops the derivation before the rest of the proof
    /// is looked at, so an executable-mode pass has something to add (D5,
    /// rows 1–3).
    #[must_use]
    pub fn masks_the_rest(self) -> bool {
        matches!(
            self,
            Cause::Hole | Cause::ExecutableConstruct | Cause::Poison
        )
    }
}

/// What the shell measured about one refusal (D6). `None` means "not asked":
/// the probe does not apply to this error, or no engine could answer it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Probes {
    /// The unbound name, or any type variable of the malformed type, is
    /// spelled with `@` or `α_`.
    pub marker: Option<bool>,
    /// The same name(s) include a concrete ADT or record name.
    pub names_concrete_type: Option<bool>,
    /// `types_equal_alpha` — the executable judgment's equality — equates the
    /// mismatch pair.
    pub lenient_equal: Option<bool>,
    /// The bootstrap's conversion engine equates the mismatch pair.
    pub converts: Option<bool>,
}

/// The two types a mismatch-set error compares (D6), or `None` for an error
/// outside the set. `BranchTypeMismatch` contributes its then/else pair.
#[must_use]
pub fn mismatch_pair(refusal: &TypeError) -> Option<(&Type, &Type)> {
    match refusal {
        TypeError::TypeMismatch { expected, got }
        | TypeError::ArgumentTypeMismatch { expected, got }
        | TypeError::NatIndMotiveMismatch { expected, got }
        | TypeError::NatRecSuccTypeMismatch { expected, got } => Some((expected, got)),
        TypeError::BranchTypeMismatch {
            then_type,
            else_type,
        } => Some((then_type, else_type)),
        _ => None,
    }
}

/// Decide one refusal's cause: the first row of D6's table that matches.
#[must_use]
pub fn classify(refusal: &TypeError, probes: &Probes) -> Cause {
    match refusal {
        TypeError::SorryInProof => return Cause::Hole,
        TypeError::ReturnInProof
        | TypeError::RegionInProof
        | TypeError::CallFrameInProof
        | TypeError::FixNotStructural { .. } => return Cause::ExecutableConstruct,
        TypeError::PoisonInProof { .. } => return Cause::Poison,
        TypeError::GlobalNotAdmitted { .. } => return Cause::GlobalNotAdmitted,
        TypeError::UnboundVariable(_) => return Cause::MissingGlobalContext,
        TypeError::UnboundTypeVar(_) | TypeError::MalformedType(_) => {
            return classify_type_variable(*probes)
        }
        TypeError::PredicateBinderNotPrenex { .. }
        | TypeError::StrayPredicate { .. }
        | TypeError::PredicateKindEmpty { .. }
        | TypeError::PredicateDomainNotData { .. }
        | TypeError::PredicateDomainOpen { .. }
        | TypeError::PredicateInstantiatesForall { .. }
        | TypeError::NotAPredicate { .. }
        | TypeError::PredicateKindMismatch { .. }
        | TypeError::UnboundPredicateVar(_)
        | TypeError::PredicateVarIsTypeVar(_)
        | TypeError::PredicateArity { .. }
        | TypeError::PredicateIntroShape { .. } => return Cause::PredicateBinder,
        _ => {}
    }
    match mismatch_pair(refusal) {
        Some(_) => classify_mismatch(*probes),
        None => Cause::Unclassified,
    }
}

/// Rows 6–7, and row 12 when neither probe says yes.
fn classify_type_variable(probes: Probes) -> Cause {
    if probes.marker == Some(true) {
        Cause::MarkerIllFormed
    } else if probes.names_concrete_type == Some(true) {
        Cause::NamedTypeReference
    } else {
        Cause::Unclassified
    }
}

/// Rows 8, 10 and 11, over a mismatch pair. Row 9's two keys (bare `Prop`
/// against an `Eq`, and `NotEquality { got: Prop }`) were deleted with
/// the `Prop` type (ADR 4.10.26i D2); they held 0 rows over `main.tg` at removal.
fn classify_mismatch(probes: Probes) -> Cause {
    if probes.lenient_equal == Some(true) {
        Cause::StrictEquality
    } else if probes.converts == Some(true) {
        Cause::NeedsConversion
    } else {
        Cause::UnexplainedMismatch
    }
}

/// The census's counts, separated from printing so the exit code is
/// assertable (D1).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CensusTally {
    /// Definitions the elaborator marks proof-relevant (axioms included).
    pub proof_relevant: usize,
    /// Definitions `--include-total` added: admitted `Total`, not proof-relevant.
    pub total_only: usize,
    /// Axioms, counted and not checked.
    pub axioms: usize,
    /// Definitions `check_proof` accepted.
    pub admitted: usize,
    /// Definitions `check_proof` refused.
    pub refused: usize,
    /// Refusals per primary cause.
    pub primary: BTreeMap<Cause, usize>,
    /// Refusals per secondary cause (D5), where one was computed.
    pub secondary: BTreeMap<Cause, usize>,
    /// Proof-relevant definitions whose statement sorts `Type₀` — a theorem
    /// stating a data type. A report, not a refusal (ADR 4.10.26i D6 (b)).
    pub data_statements: Vec<String>,
}

impl CensusTally {
    /// Every definition the census looked at.
    #[must_use]
    pub fn population(&self) -> usize {
        self.proof_relevant + self.total_only
    }

    /// Count one definition's outcome.
    pub fn record(&mut self, primary: Cause, secondary: Option<Cause>) {
        match primary {
            Cause::Admitted => self.admitted += 1,
            Cause::Axiom => self.axioms += 1,
            refusal => {
                self.refused += 1;
                *self.primary.entry(refusal).or_default() += 1;
                if let Some(cause) = secondary {
                    *self.secondary.entry(cause).or_default() += 1;
                }
            }
        }
    }

    /// 0 on a completed census whatever it counted; 2 when there was nothing
    /// to count, because a census of nothing is not a clean one (D1).
    #[must_use]
    pub fn exit(&self) -> ExitCode {
        ExitCode::from(self.exit_code())
    }

    /// [`CensusTally::exit`] as a number, for assertion.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self.population() == 0 {
            2
        } else {
            0
        }
    }
}

/// What the gate measures about a refusal beyond the census's probes (ADR
/// 2.10.26d D5). `None` means "not asked", and never matches a class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refinement {
    /// The mismatch pair converts with an **empty** δ map.
    pub converts_without_delta: Option<bool>,
    /// The unbound name is the theorem's own.
    pub unbound_is_self: Option<bool>,
}

/// The register class of one refusal, or `None` when no class explains it
/// (D5's table). Only `needs-conversion` that genuinely needs δ is
/// `needs-delta`: a pair that converts without δ while the kernel refuses it
/// is a kernel-conversion disagreement, not δ.
#[must_use]
pub fn register_class(cause: Cause, refinement: Refinement) -> Option<RegisterClass> {
    match cause {
        Cause::NeedsConversion if refinement.converts_without_delta == Some(false) => {
            Some(RegisterClass::NeedsDelta)
        }
        Cause::NamedTypeReference => Some(RegisterClass::NamedType),
        Cause::MissingGlobalContext if refinement.unbound_is_self == Some(true) => {
            Some(RegisterClass::SelfReference)
        }
        _ => None,
    }
}
