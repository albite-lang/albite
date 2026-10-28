//! The census's half of the kernel boundary (ADR 30.9.26f, 2.10.26d D10): the
//! cause taxonomy and the probes over an elaborated project, shared with the
//! gate. Grouped out of `kernel/` by ADR 3.10.26p (P0.1), with no behaviour
//! change.

mod classify;
mod probes;

pub use classify::{
    classify, mismatch_pair, register_class, Cause, CensusTally, Probes, Refinement,
};
pub(crate) use probes::delta_candidates;
pub use probes::ElaboratedProbes;
