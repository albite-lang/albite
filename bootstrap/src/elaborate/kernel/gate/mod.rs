//! The term-mode kernel gate (ADR 2.10.26d): the exemption register, the pure
//! verdict, and the shell that runs each obligation through the kernel.
//! Grouped out of `kernel/` by ADR 3.10.26p (P0.1), with no behaviour change.

mod register;
mod term;
mod verdict;

pub(super) use term::TermChecks;
pub(crate) use term::{term_verdicts, TermGate};

pub use register::{register_key, repository_root, Exemption, Register, RegisterClass};
pub use verdict::{decide, ExemptTally, GateVerdict, KernelMode, KernelOutcome, Violation};
