//! The kernel gate's decision for one term-mode theorem (ADR 2.10.26d §2.2).
//!
//! Pure: the kernel's outcome, the theorem's register row and the exempt
//! names it mentions go in; a [`GateVerdict`] comes out. Every arm of §2.1 is
//! therefore a test over injected values, and `discharge` is only the shell
//! that runs the kernel and maps the verdict to a diagnostic.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::register::{Exemption, RegisterClass};
use crate::elaborate::kernel::Cause;

/// Whether a refusal fails the build. Read once, on the caller thread, from
/// the census's reporting window alone — never from `--termination` (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelMode {
    Gate,
    /// A census is elaborating: it classifies refusals itself.
    Report,
}

impl KernelMode {
    /// `Report` exactly when a reporting window is open.
    #[must_use]
    pub fn from_window(open: bool) -> Self {
        if open {
            KernelMode::Report
        } else {
            KernelMode::Gate
        }
    }
}

/// What the kernel said about one theorem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelOutcome {
    Admitted,
    /// The term contains `sorry`: not checked (D7).
    Hole,
    /// Refused, with the census's cause and D5's class (`None`: no class).
    Refused {
        cause: Cause,
        class: Option<RegisterClass>,
    },
}

/// How a register row and the build disagree (D4's rules 2–4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// A `needs-delta` row for a theorem the kernel admits.
    Stale,
    /// The row's class is not what the kernel observed.
    Drifted {
        row: RegisterClass,
        observed: String,
    },
    /// An admitted theorem mentions exempt ones and has no row.
    CitesWithoutRow { cited: Vec<String> },
    /// A `cites-exempt` row whose list is not what the theorem mentions.
    CitesMismatch {
        listed: Vec<String>,
        cited: Vec<String>,
    },
    /// A `cites-exempt` row none of whose names is exempt any more.
    CitesCleared,
}

impl Violation {
    /// The clause the diagnostic prints after the theorem's name.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Violation::Stale => "the kernel admits it, so its needs-delta row is stale: remove the row and lower KERNEL_EXEMPTION_CEILING".to_string(),
            Violation::Drifted { row, observed } => format!(
                "its row says {} but the kernel's refusal is {observed}",
                row.label()
            ),
            Violation::CitesWithoutRow { cited } => format!(
                "it cites the exempt theorem(s) {}, so it needs a cites-exempt row",
                cited.join(", ")
            ),
            Violation::CitesMismatch { listed, cited } => format!(
                "its cites-exempt row lists {} but it cites {}",
                listed.join(", "),
                cited.join(", ")
            ),
            Violation::CitesCleared => "nothing its cites-exempt row lists is exempt any more, so the row is stale".to_string(),
        }
    }
}

/// The gate's decision for one theorem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    Pass,
    Exempt(RegisterClass),
    SorryStatus,
    Refused {
        cause: Cause,
        class: Option<RegisterClass>,
    },
    RegisterViolation(Violation),
}

impl GateVerdict {
    /// What a refusal is called: its class, else the census's cause.
    #[must_use]
    pub fn refusal_label(cause: Cause, class: Option<RegisterClass>) -> &'static str {
        class.map_or_else(|| cause.label(), RegisterClass::label)
    }
}

/// D4's rules over one theorem. `cited_exempt` is the exempt theorems it
/// mentions, itself excluded.
#[must_use]
pub fn decide(
    mode: KernelMode,
    outcome: KernelOutcome,
    row: Option<&Exemption>,
    cited_exempt: &BTreeSet<String>,
) -> GateVerdict {
    if mode == KernelMode::Report {
        return GateVerdict::Pass;
    }
    match outcome {
        KernelOutcome::Hole => GateVerdict::SorryStatus,
        KernelOutcome::Admitted => decide_admitted(row, cited_exempt),
        KernelOutcome::Refused { cause, class } => decide_refused(cause, class, row),
    }
}

fn decide_refused(
    cause: Cause,
    class: Option<RegisterClass>,
    row: Option<&Exemption>,
) -> GateVerdict {
    match row {
        None => GateVerdict::Refused { cause, class },
        Some(row) if class == Some(row.class) => GateVerdict::Exempt(row.class),
        // No class explains the refusal, so no row can silence it (D5).
        Some(_) if class.is_none() => GateVerdict::Refused { cause, class },
        Some(row) => GateVerdict::RegisterViolation(Violation::Drifted {
            row: row.class,
            observed: GateVerdict::refusal_label(cause, class).to_string(),
        }),
    }
}

fn decide_admitted(row: Option<&Exemption>, cited_exempt: &BTreeSet<String>) -> GateVerdict {
    let cited: Vec<String> = cited_exempt.iter().cloned().collect();
    let Some(row) = row else {
        return if cited.is_empty() {
            GateVerdict::Pass
        } else {
            GateVerdict::RegisterViolation(Violation::CitesWithoutRow { cited })
        };
    };
    if row.class != RegisterClass::CitesExempt {
        return GateVerdict::RegisterViolation(Violation::Stale);
    }
    if cited.is_empty() {
        return GateVerdict::RegisterViolation(Violation::CitesCleared);
    }
    let listed: BTreeSet<&String> = row.cites.iter().collect();
    if listed == cited.iter().collect() {
        GateVerdict::Exempt(RegisterClass::CitesExempt)
    } else {
        GateVerdict::RegisterViolation(Violation::CitesMismatch {
            listed: row.cites.clone(),
            cited,
        })
    }
}

/// How many theorems a build exempted, by class — what `check` names (D11).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExemptTally(BTreeMap<RegisterClass, usize>);

impl ExemptTally {
    pub fn record(&mut self, class: RegisterClass) {
        *self.0.entry(class).or_default() += 1;
    }

    pub fn absorb(&mut self, other: ExemptTally) {
        for (class, count) in other.0 {
            *self.0.entry(class).or_default() += count;
        }
    }

    #[must_use]
    pub fn total(&self) -> usize {
        self.0.values().sum()
    }

    /// `needs-delta 3`, classes in D5's order.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (class, count) in &self.0 {
            if !out.is_empty() {
                out.push_str(", ");
            }
            let _ = write!(out, "{} {count}", class.label());
        }
        out
    }
}
