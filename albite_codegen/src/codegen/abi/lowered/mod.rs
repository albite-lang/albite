//! The canonical lowered-signature descriptor (ADR 1.7.26e R6): [`sig`] holds
//! the pure data model + exact `musttail`-compatibility check; [`emit`] holds
//! the LLVM-side construction and attribute attachment.

mod describe;
mod emit;
mod sig;

pub(crate) use sig::{LoweredSignature, LoweredSlot, MusttailIncompat};
// Only the ABI unit tests name the slot role / attribute types directly.
#[cfg(test)]
pub(crate) use sig::{SlotAttrs, SlotRole};
