//! A tactic block's own errors (ADR 2.10.26c D8): a nested kind with its own
//! one-arm-per-line code table, which `explain`'s catalogue test reads as a
//! second oracle beside `codes.rs`. Later tactic slices extend it here, so
//! `ElabErrorKind` gains one variant, not one per slice.

use serde::{Deserialize, Serialize};

use albite_core::{Term, Type};

/// One local a goal's context holds: its name and type.
pub type GoalLocal = (String, Type);

/// Errors a tactic raises before the kernel is reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TacticErrorKind {
    /// The tactic cannot act on the focused goal — or there is no goal left.
    TacticNotApplicable {
        /// The tactic as written (`intro`, `exact`, `apply`, `refl`).
        tactic: String,
        /// The focused goal's target, or `Unit` when no goal is left
        /// ([`NO_GOAL_LEFT`]).
        goal: Type,
        /// Why, in a phrase.
        why: String,
    },

    /// The block ended with goals still open: each one's locals and target.
    UnsolvedGoals { goals: Vec<(Vec<GoalLocal>, Type)> },

    /// A `rewrite` rule rewrites nothing in the goal (ADR 2.10.26e D7).
    RewriteNoOccurrence {
        /// The rule's 1-based position in its list.
        position: usize,
        /// The rule's equation, instantiated: the elaborator keeps no source
        /// text, so this is how the rule is named.
        rule: Type,
        /// The side searched for, after instantiation.
        pattern: Term,
        goal: Type,
        /// Whether the rule was written `<- e`.
        reversed: bool,
        /// Whether the rule's other side occurs: the wrong-direction hint.
        other_side_occurs: bool,
    },

    /// A `simp` call reached its step bound with a rule still applicable
    /// (ADR 4.10.26e D3): its rules likely undo each other.
    SimpStepBound {
        /// The bound, `SIMP_STEP_BOUND`.
        bound: usize,
        /// The 1-based positions of the last rules applied, oldest first:
        /// where a cycle shows.
        last_rules: Vec<usize>,
        /// The goal as the bound found it.
        goal: Type,
    },

    /// `ring`'s two sides normalise to different polynomials (ADR 4.10.26g
    /// D6): the equation is false as a commutative-semiring identity over its
    /// atoms. Each side is rendered in D2's normal form.
    RingUnequal { left: String, right: String },

    /// `omega` could not prove its goal (ADR 6.10.26f D7): the goal is
    /// outside its fragment, or no witness was found.
    OmegaUnproved(OmegaFailure),
}

/// Why `omega` could not prove a `Le`- or `Eq<Nat, …>`-shaped goal (ADR
/// 6.10.26f D7): one code, three messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OmegaFailure {
    /// A side holds a term `ring` cannot read, rendered.
    OutsideFragment { term: String },
    /// Every candidate hypothesis was used and the difference still has a
    /// negative term: `difference_of` is `t - s` (or `s - t` for an `Eq`
    /// goal whose `t - s` is positive), `difference` its value, `negative`
    /// the offending term with its sign.
    NoWitness {
        difference_of: String,
        difference: String,
        negative: String,
    },
    /// Some candidate neither oriented nor cancelled: each one's name and
    /// equation.
    Unoriented { unused: Vec<(String, String)> },
}

/// Generates `TacticErrorKind::code()` from a flat variant→code table.
macro_rules! tactic_error_codes {
    ($($pat:pat => $code:expr),* $(,)?) => {
        impl TacticErrorKind {
            /// Get the error code for this kind.
            pub fn code(&self) -> &'static str {
                match self {
                    $($pat => $code,)*
                }
            }
        }
    };
}

tactic_error_codes! {
    TacticErrorKind::TacticNotApplicable { .. } => "E0082",
    TacticErrorKind::UnsolvedGoals { .. } => "E0083",
    TacticErrorKind::RewriteNoOccurrence { .. } => "E0085",
    TacticErrorKind::SimpStepBound { .. } => "E0086",
    TacticErrorKind::RingUnequal { .. } => "E0087",
    TacticErrorKind::OmegaUnproved(..) => "E0088",
}

/// The `why` of a tactic run after the last goal closed, whose `goal` is a
/// placeholder the message must not print.
pub const NO_GOAL_LEFT: &str = "no goal is left";

impl TacticErrorKind {
    /// The default message.
    #[must_use]
    pub fn default_message(&self) -> String {
        match self {
            TacticErrorKind::TacticNotApplicable { tactic, why, .. } if why == NO_GOAL_LEFT => {
                format!("`{tactic}` has nothing to act on: {why}")
            }
            TacticErrorKind::TacticNotApplicable { tactic, goal, why } => {
                format!("`{tactic}` does not apply to the goal `{goal}`: {why}")
            }
            TacticErrorKind::UnsolvedGoals { goals } => {
                let rendered: Vec<String> = goals.iter().map(render_goal).collect();
                format!(
                    "the `by` block ends with {} unsolved goal(s): {}",
                    goals.len(),
                    rendered.join("; ")
                )
            }
            TacticErrorKind::RewriteNoOccurrence {
                position,
                rule,
                pattern,
                goal,
                reversed,
                other_side_occurs,
            } => {
                let head = format!("`rewrite` rule {position} (`{rule}`)");
                let pattern = pattern.strip_spans();
                if rewrites_nothing(rule) {
                    return format!(
                        "{head} rewrites nothing: both sides are the same term `{pattern}`"
                    );
                }
                let missing =
                    format!("{head} finds no occurrence of `{pattern}` in the goal `{goal}`");
                if !other_side_occurs {
                    missing
                } else if *reversed {
                    format!(
                        "{missing}; the left-hand side occurs: did you mean the rule without `<-`?"
                    )
                } else {
                    format!("{missing}; the right-hand side occurs: did you mean `<-`?")
                }
            }
            TacticErrorKind::SimpStepBound {
                bound,
                last_rules,
                goal,
            } => {
                let rules: Vec<String> = last_rules.iter().map(ToString::to_string).collect();
                format!(
                    "`simp` took {bound} rewrite steps and a rule still applies to the goal \
                     `{goal}`; the last rules applied were {}: rules that undo each other \
                     loop, so give `rewrite` the steps instead",
                    rules.join(", ")
                )
            }
            TacticErrorKind::RingUnequal { left, right } => format!(
                "`ring`: the two sides normalise to different polynomials: `{left}` and `{right}`"
            ),
            TacticErrorKind::OmegaUnproved(failure) => failure.message(),
        }
    }
}

impl OmegaFailure {
    /// D7's three messages.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            OmegaFailure::OutsideFragment { term } => {
                format!("`omega`: the goal is outside its fragment: `ring` cannot read `{term}`")
            }
            OmegaFailure::NoWitness {
                difference_of,
                difference,
                negative,
            } => format!(
                "`omega`: no witness: `{difference_of}` is `{difference}`, with a negative \
                 term `{negative}`"
            ),
            OmegaFailure::Unoriented { unused } => {
                let named: Vec<String> = unused
                    .iter()
                    .map(|(name, equation)| format!("`{name}` (`{equation}`)"))
                    .collect();
                format!(
                    "`omega`: no witness over the oriented hypotheses; unused: {}",
                    named.join(", ")
                )
            }
        }
    }
}

/// Whether an equation's two sides are the same term, so rewriting by it
/// would transport the goal to itself (ADR 2.10.26e D3).
fn rewrites_nothing(rule: &Type) -> bool {
    matches!(rule, Type::Eq(_, a, b) if a.strip_spans() == b.strip_spans())
}

/// `x : A, y : B ⊢ T`, or `⊢ T` with no locals.
fn render_goal((locals, target): &(Vec<GoalLocal>, Type)) -> String {
    let context: Vec<String> = locals.iter().map(|(n, t)| format!("{n} : {t}")).collect();
    if context.is_empty() {
        format!("⊢ {target}")
    } else {
        format!("{} ⊢ {target}", context.join(", "))
    }
}
