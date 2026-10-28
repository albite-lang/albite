//! Decision procedures (B11, decision_procedures.md Q10): tactics that decide
//! a fragment and build the proof themselves, each in its own directory, from
//! steps the kernel already types — so none adds a kernel rule.
//!
//! - `ring/`: `ring`, commutative-semiring normalisation over `Nat` (ADR
//!   4.10.26g)
//! - `omega/`: `omega`, bounded linear `Nat` goals by witness synthesis plus
//!   `ring` (ADR 6.10.26f)

pub(super) mod omega;
pub(super) mod ring;
