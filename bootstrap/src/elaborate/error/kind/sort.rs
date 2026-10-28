//! Sort errors (ADR 4.10.26i D3, D5): a nested kind with its own
//! one-arm-per-line code table, which `explain`'s catalogue test reads as a
//! further oracle beside `codes.rs`, as it reads `TacticErrorKind`'s.

use serde::{Deserialize, Serialize};

use albite_core::Type;

/// The sentence both compilers share for `Prop` in a type position (D3). The
/// battery greps a backtick-free substring of it: "is a sort".
pub const PROP_IS_A_SORT: &str = "is a sort: it classifies propositions such as `a == b`, \
                                  and is not a type with values";

/// Errors of the sort judgment, raised before the kernel is reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SortErrorKind {
    /// `Prop` or `Type` written where a type is expected. Carries the word.
    SortUsedAsType { word: String },

    /// A family whose body is not a proposition (`type Bad(n: Nat): Prop =
    /// Nat`), or one of whose parameters ranges over a proposition. `part`
    /// names which; `expected` the sort it must have, in words.
    FamilySortMismatch {
        family: String,
        part: String,
        ty: Type,
        expected: String,
    },

    /// Arguments on a name the definition in scope binds, which the
    /// elaborator cannot keep (ADR 6.10.26b D3). `own_params: None` is a type
    /// parameter applied to arguments; `Some(params)` a recursive use of the
    /// type being defined that does not pass its own parameters.
    TypeParameterApplied {
        name: String,
        own_params: Option<Vec<String>>,
    },

    /// A type where a term belongs, or a term where a type belongs (ADR
    /// 6.10.26d D3): `Le<Nat, m>`, `List<0>`. `found` is the written argument
    /// in words; `slot` says which of the two was expected.
    TermTypeConfusion { found: String, slot: ConfusedSlot },

    /// A type parameter's kind that is not `A₁ -> … -> Aₙ -> Prop` over data
    /// (ADR 6.10.26h D2). `param` is the parameter; `why` which rule it broke.
    PredicateKindRefused { param: String, why: KindRefusal },

    /// `inl(e)` or `inr(e)` checked against a type that is not a disjunction
    /// (ADR 7.10.26l D2): a sum of data, which its declared constructors
    /// build, or no sum at all. `ctor` is the form as written.
    SumIntroNotDisjunction { ctor: String, expected: Type },

    /// A declared constructor payload or record field whose type holds a
    /// dependent function type (ADR 7.10.26n D2). `field` names it in words.
    DependentFunctionField { field: String, ty: Type },
}

/// Why a [`SortErrorKind::PredicateKindRefused`] kind was refused (ADR
/// 6.10.26h D2): one message per case, one code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KindRefusal {
    /// `P: Prop`: a variable ranging over propositions, which needs
    /// impredicative `Prop` (Q2, v2.1).
    RangesOverPropositions,
    /// A domain that is itself a proposition (`Eq<Nat, 0, 0> -> Prop`).
    DomainIsProposition { domain: Type },
    /// A domain that names a type parameter (`<T, P: T -> Prop>`).
    DomainNamesTypeParameter { domain: String },
    /// A codomain other than `Prop` (`F: Nat -> Nat`): a type operator.
    CodomainNotProp,
    /// Any other bound (`T: Type`, `T: Ord`).
    NotAKind,
    /// A kind on a type definition's parameter, which Q6 keeps prenex in
    /// definitions, never data.
    OnTypeDefinition { type_name: String },
}

/// What a [`SortErrorKind::TermTypeConfusion`] argument should have been.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfusedSlot {
    /// A family's parameter, which takes a term of `param_ty`.
    FamilyParameter {
        family: String,
        param: String,
        param_ty: Type,
    },
    /// A type position.
    Type,
}

/// `FamilySortMismatch`'s `expected` for a body.
pub const FAMILY_BODY_MUST_BE: &str = "a proposition: a data type cannot be declared `Prop`";
/// `FamilySortMismatch`'s `expected` for a parameter's type.
pub const FAMILY_PARAMETER_MUST_BE: &str = "data, as an `exists` binder's type must be";

/// Generates `SortErrorKind::code()` from a flat variant→code table.
macro_rules! sort_error_codes {
    ($($pat:pat => $code:expr),* $(,)?) => {
        impl SortErrorKind {
            /// Get the error code for this kind.
            pub fn code(&self) -> &'static str {
                match self {
                    $($pat => $code,)*
                }
            }
        }
    };
}

sort_error_codes! {
    SortErrorKind::SortUsedAsType { .. } => "E0066",
    SortErrorKind::FamilySortMismatch { .. } => "E0067",
    SortErrorKind::TypeParameterApplied { .. } => "E0068",
    SortErrorKind::TermTypeConfusion { .. } => "E0023",
    SortErrorKind::PredicateKindRefused { .. } => "E0069",
    SortErrorKind::SumIntroNotDisjunction { .. } => "E0024",
    SortErrorKind::DependentFunctionField { .. } => "E0025",
}

impl SortErrorKind {
    /// The default message.
    #[must_use]
    pub fn default_message(&self) -> String {
        match self {
            SortErrorKind::SortUsedAsType { word } => format!("`{word}` {PROP_IS_A_SORT}"),
            SortErrorKind::FamilySortMismatch {
                family,
                part,
                ty,
                expected,
            } => format!("in `{family}`, {part} is `{ty}`, which must be {expected}"),
            SortErrorKind::TypeParameterApplied {
                name,
                own_params: None,
            } => format!(
                "type parameter `{name}` takes no arguments: a type parameter stands \
                 for a type, not for a type constructor"
            ),
            SortErrorKind::TypeParameterApplied {
                name,
                own_params: Some(params),
            } => format!(
                "a recursive use of `{name}` must pass its own parameters, `<{}>`",
                params.join(", ")
            ),
            SortErrorKind::TermTypeConfusion {
                found,
                slot:
                    ConfusedSlot::FamilyParameter {
                        family,
                        param,
                        param_ty,
                    },
            } => format!(
                "{found} is a type, and `{family}`'s parameter `{param}` takes a `{param_ty}` term"
            ),
            SortErrorKind::TermTypeConfusion {
                found,
                slot: ConfusedSlot::Type,
            } => format!("{found} is a term, and a type is expected here"),
            SortErrorKind::PredicateKindRefused { param, why } => kind_refusal_message(param, why),
            SortErrorKind::SumIntroNotDisjunction { ctor, expected } => format!(
                "`{ctor}` builds a disjunction of propositions, and `{expected}` is not one; \
                 a data sum is built by its declared constructors"
            ),
            SortErrorKind::DependentFunctionField { field, ty } => format!(
                "a dependent function type cannot be a field of a data type: {field} is `{ty}`"
            ),
        }
    }
}

/// E0069's message, one per [`KindRefusal`] (ADR 6.10.26h D2).
fn kind_refusal_message(param: &str, why: &KindRefusal) -> String {
    match why {
        KindRefusal::RangesOverPropositions => format!(
            "`{param}: Prop` ranges over propositions, which needs impredicative `Prop`; \
             a predicate parameter takes at least one data argument, `{param}: Nat -> Prop`"
        ),
        KindRefusal::DomainIsProposition { domain } => format!(
            "`{param}`'s argument `{domain}` is a proposition: a predicate's arguments are data"
        ),
        KindRefusal::DomainNamesTypeParameter { domain } => format!(
            "`{param}`'s argument type names the type parameter `{domain}`: a predicate's \
             argument types must be closed"
        ),
        KindRefusal::CodomainNotProp => format!(
            "`{param}`'s kind does not end in `Prop`: a type parameter may be a predicate, \
             never a type operator"
        ),
        KindRefusal::NotAKind => {
            format!("`{param}`'s bound is not a predicate kind: write `{param}: A -> … -> Prop`")
        }
        KindRefusal::OnTypeDefinition { type_name } => format!(
            "`{type_name}`'s parameter `{param}` has a kind: a predicate parameter belongs \
             on a `fn`, `theorem`, `lemma` or `axiom`, not on a type"
        ),
    }
}
