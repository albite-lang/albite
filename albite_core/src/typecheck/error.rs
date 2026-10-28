//! Type checking errors
//!
//! This module defines the error types returned by the type checker.

use std::fmt;

use crate::types::Type;

/// Type checking errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    /// Variable not found in context
    UnboundVariable(String),

    /// Type variable not in scope
    UnboundTypeVar(String),

    /// Expected a function type, got something else
    NotAFunction { got: Type },

    /// Type mismatch in application
    ArgumentTypeMismatch { expected: Type, got: Type },

    /// Expected a product type for fst/snd
    NotAProduct { got: Type },

    /// Expected a sum type for case
    NotASum { got: Type },

    /// Branches of if/case have different types
    BranchTypeMismatch { then_type: Type, else_type: Type },

    /// Expected Bool for if condition
    ConditionNotBool { got: Type },

    /// Expected Nat for natrec/natind/succ
    NotANat { got: Type },

    /// Expected a forall type for type application
    NotPolymorphic { got: Type },

    /// Expected Void, or a refuted `Bool` equation, for absurd (ADR 7.10.26m D1)
    NotVoid { got: Type },

    /// Expected an equality type for subst
    NotEquality { got: Type },

    /// General type mismatch
    TypeMismatch { expected: Type, got: Type },

    /// Type is not well-formed
    MalformedType(Type),

    /// Natrec type mismatch: succ case doesn't have right type
    NatRecSuccTypeMismatch {
        expected: Type, // Nat → τ → τ
        got: Type,
    },

    /// `NatInd` motive mismatch
    NatIndMotiveMismatch { expected: Type, got: Type },

    /// Motive is not a function from τ to Prop
    MotiveNotFunction { got: Type },

    // === Phase 2B: Flat ADT Errors (ADR 2.2.26) ===
    /// Invalid variant index in ADT construct/match
    InvalidVariantIndex { index: usize, num_variants: usize },

    /// Expected an ADT type
    NotAnAdt { got: Type },

    /// Match expression has no arms
    EmptyMatch,

    // === Proof judgment gates (ADR 29.9.26e) ===
    /// A `fix` in a proof that the termination engine does not certify total.
    FixNotStructural { name: String, reason: String },

    /// A `fold`/`unfold` over a `μ` whose variable occurs non-strictly
    /// positively — refused in both modes.
    NonStrictlyPositiveMu { mu: Type, field: String },

    /// A `return` in a proof.
    ReturnInProof,

    /// A `region` in a proof (ADR 9.10.26o D3): a proof term frees nothing.
    RegionInProof,

    /// A call frame in a proof (ADR 13.10.26n D3): the evaluator's function
    /// boundary, which no elaborated term holds.
    CallFrameInProof,

    /// A name in a proof that resolves to an opaque global, or a `Global`
    /// node that resolves to a hypothesis rather than an admitted global.
    GlobalNotAdmitted { name: String },

    /// A poisoned (`Type::Error`) type in a proof term or its context.
    PoisonInProof { ty: Type },

    /// A `sorry` hole in a proof.
    SorryInProof,

    // === Proof-mode `Eq` witnesses (ADR 1.10.26c D7) ===
    /// A term inside `eq` that does not have `eq`'s base type under the
    /// proof judgment; `cause` is the inner refusal.
    IllTypedEqWitness { eq: Type, cause: Box<TypeError> },

    // === Proof-mode dependent case split (ADR 3.10.26o D8) ===
    /// A term the dependent case rule does not split, and why. Internal:
    /// `type_of_annot` answers the existing rule's error instead (D2), so
    /// only the rule's direct tests observe it.
    DependentCaseShape { why: &'static str },

    // === Dependent pair (ADR 4.10.26h D3) ===
    /// A `Sigma` whose binder ranges over a proposition, not data. Q4 asks
    /// for the binder's sort to be `Type₀`; until sorts exist (4.10.26i) the
    /// stand-in refuses `Prop` and anything `is_proposition` answers yes for.
    SigmaBinderNotData { got: Type },

    // === Predicate binders (ADR 6.10.26h D6) ===
    /// A `ForallPred` anywhere but a global's prenex prefix: under an arrow,
    /// as a λ's annotation, or as a `TyApp` argument (impredicative).
    PredicateBinderNotPrenex { ty: Type },
    /// A `PredAbs` anywhere but a `TyApp` argument against a `ForallPred`.
    StrayPredicate { ty: Type },
    /// `P` bound with no domain: a variable ranging over propositions (Q2).
    PredicateKindEmpty { var: String },
    /// A domain of `P`'s kind that sorts `Prop`, not `Type₀`.
    PredicateDomainNotData { var: String, domain: Type },
    /// A domain of `P`'s kind that names a type variable, so an
    /// instantiation could change its sort.
    PredicateDomainOpen { var: String, domain: Type },
    /// A `PredAbs` instantiating a plain `Forall`.
    PredicateInstantiatesForall { got: Type },
    /// A `ForallPred` instantiated at something that is not a `PredAbs`.
    NotAPredicate { got: Type },
    /// A `PredAbs` whose parameters are not the kind's domains, or whose
    /// body does not sort `Prop`.
    PredicateKindMismatch { expected: Vec<Type>, got: Type },
    /// `P<…>` with `P` not bound in the context at all.
    UnboundPredicateVar(String),
    /// `P<…>` with `P` bound as a type variable, which has no kind.
    PredicateVarIsTypeVar(String),
    /// `P<…>` with a different number of arguments than `P`'s kind.
    PredicateArity {
        var: String,
        expected: usize,
        got: usize,
    },
    /// An annotation with a `ForallPred` prefix whose term is not a `TyAbs`
    /// chain of the prefix's length.
    PredicateIntroShape { ty: Type },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // "Expected X, got: {got}" family
        if let Some((expected_desc, got)) = self.expected_got_msg() {
            return write!(f, "Expected {expected_desc}, got: {got}");
        }
        // "X mismatch: expected {e}, got {g}" family
        if let Some((label, expected, got)) = self.mismatch_msg() {
            return write!(f, "{label} mismatch: expected {expected}, got {got}");
        }
        // Predicate-binder refusals (ADR 6.10.26h D6)
        if let Some(message) = self.predicate_msg() {
            return write!(f, "{message}");
        }
        match self {
            TypeError::UnboundVariable(v) => write!(f, "Unbound variable: {v}"),
            TypeError::UnboundTypeVar(v) => write!(f, "Unbound type variable: {v}"),
            TypeError::BranchTypeMismatch {
                then_type,
                else_type,
            } => {
                write!(f, "Branch type mismatch: {then_type} vs {else_type}")
            }
            TypeError::ConditionNotBool { got } => {
                write!(f, "Condition must be Bool, got: {got}")
            }
            TypeError::MalformedType(ty) => write!(f, "Malformed type: {ty}"),
            TypeError::MotiveNotFunction { got } => {
                write!(f, "Motive must be a function type, got: {got}")
            }
            TypeError::InvalidVariantIndex {
                index,
                num_variants,
            } => {
                write!(
                    f,
                    "Invalid variant index {index} for ADT with {num_variants} variants"
                )
            }
            TypeError::EmptyMatch => {
                write!(f, "Match expression has no arms")
            }
            TypeError::FixNotStructural { name, reason } => {
                write!(f, "fix `{name}` is not structural in a proof: {reason}")
            }
            TypeError::NonStrictlyPositiveMu { mu, field } => {
                write!(f, "{mu} is not strictly positive in {field}")
            }
            TypeError::ReturnInProof => write!(f, "`return` is not allowed in a proof"),
            TypeError::RegionInProof => write!(f, "`region` is not allowed in a proof"),
            TypeError::CallFrameInProof => {
                write!(f, "an evaluator call frame is not allowed in a proof")
            }
            TypeError::GlobalNotAdmitted { name } => {
                write!(f, "`{name}` is not an admitted global in a proof")
            }
            TypeError::PoisonInProof { ty } => {
                write!(f, "poisoned type in a proof: {ty}")
            }
            TypeError::SorryInProof => write!(f, "`sorry` is not allowed in a proof"),
            TypeError::IllTypedEqWitness { eq, cause } => {
                write!(f, "ill-typed term inside {eq}: {cause}")
            }
            TypeError::DependentCaseShape { why } => {
                write!(f, "not a dependent case split: {why}")
            }
            TypeError::SigmaBinderNotData { got } => {
                write!(f, "an `exists` must bind data, not a proposition: {got}")
            }
            // Handled by helpers above
            TypeError::NotAFunction { .. }
            | TypeError::NotAProduct { .. }
            | TypeError::NotASum { .. }
            | TypeError::NotANat { .. }
            | TypeError::NotPolymorphic { .. }
            | TypeError::NotVoid { .. }
            | TypeError::NotEquality { .. }
            | TypeError::NotAnAdt { .. }
            | TypeError::ArgumentTypeMismatch { .. }
            | TypeError::TypeMismatch { .. }
            | TypeError::NatRecSuccTypeMismatch { .. }
            | TypeError::NatIndMotiveMismatch { .. }
            | TypeError::PredicateBinderNotPrenex { .. }
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
            | TypeError::PredicateIntroShape { .. } => unreachable!(),
        }
    }
}

impl TypeError {
    /// The message of a predicate-binder refusal (ADR 6.10.26h D6).
    fn predicate_msg(&self) -> Option<String> {
        Some(match self {
            TypeError::PredicateBinderNotPrenex { ty } => {
                format!("a predicate binder is allowed only at a definition's head: {ty}")
            }
            TypeError::StrayPredicate { ty } => {
                format!("a predicate is allowed only as a type argument: {ty}")
            }
            TypeError::PredicateKindEmpty { var } => {
                format!("`{var}` ranges over propositions, which needs impredicative `Prop`")
            }
            TypeError::PredicateDomainNotData { var, domain } => {
                format!("`{var}`'s domain `{domain}` is a proposition, not data")
            }
            TypeError::PredicateDomainOpen { var, domain } => {
                format!("`{var}`'s domain `{domain}` names a type variable")
            }
            TypeError::PredicateInstantiatesForall { got } => {
                format!("a predicate cannot instantiate a type parameter: {got}")
            }
            TypeError::NotAPredicate { got } => {
                format!("a predicate parameter needs a predicate, got: {got}")
            }
            TypeError::PredicateKindMismatch { expected, got } => {
                let domains: Vec<String> = expected.iter().map(ToString::to_string).collect();
                format!(
                    "expected a predicate over ({}) → Prop, got: {got}",
                    domains.join(", ")
                )
            }
            TypeError::UnboundPredicateVar(var) => format!("Unbound predicate variable: {var}"),
            TypeError::PredicateVarIsTypeVar(var) => {
                format!("`{var}` is a type variable, not a predicate, so it takes no arguments")
            }
            TypeError::PredicateArity { var, expected, got } => {
                format!("`{var}` takes {expected} argument(s), got {got}")
            }
            TypeError::PredicateIntroShape { ty } => {
                format!("a definition with a predicate parameter must abstract it: {ty}")
            }
            _ => return None,
        })
    }
}

impl TypeError {
    /// "Expected X, got: {got}" — single-type-mismatch errors.
    fn expected_got_msg(&self) -> Option<(&str, &Type)> {
        match self {
            TypeError::NotAFunction { got } => Some(("function type", got)),
            TypeError::NotAProduct { got } => Some(("product type", got)),
            TypeError::NotASum { got } => Some(("sum type", got)),
            TypeError::NotANat { got } => Some(("Nat", got)),
            TypeError::NotPolymorphic { got } => Some(("polymorphic type (∀α. τ)", got)),
            TypeError::NotVoid { got } => {
                Some(("Void or an equation between `true` and `false`", got))
            }
            TypeError::NotEquality { got } => Some(("Eq type", got)),
            TypeError::NotAnAdt { got } => Some(("ADT type", got)),
            _ => None,
        }
    }

    /// "X mismatch: expected {e}, got {g}" — two-type-mismatch errors.
    fn mismatch_msg(&self) -> Option<(&str, &Type, &Type)> {
        match self {
            TypeError::ArgumentTypeMismatch { expected, got } => {
                Some(("Argument type", expected, got))
            }
            TypeError::TypeMismatch { expected, got } => Some(("Type", expected, got)),
            TypeError::NatRecSuccTypeMismatch { expected, got } => {
                Some(("natrec succ case type", expected, got))
            }
            TypeError::NatIndMotiveMismatch { expected, got } => {
                Some(("natind motive", expected, got))
            }
            _ => None,
        }
    }
}

impl std::error::Error for TypeError {}
