//! Region errors, E0094–E0096 (ADR 9.10.26o D4): a nested kind with its own
//! one-arm-per-line code table, the shape `handle.rs` uses. The self-host
//! raises the same three as E0730–E0732, with the same sentences.

use serde::{Deserialize, Serialize};

/// Refusals of `region { e }` (`B1.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionErrorKind {
    /// The region's value, or an exit crossing it, has a type that is not
    /// first-order data (G3): `found` is the offending part of it.
    NotFirstOrder { found: String },
    /// The region body names something G4 keeps out of it: a resource
    /// handle bound outside it, a function whose declared type contains one,
    /// a handle constructor, or an outer variable whose type contains a type
    /// parameter of the enclosing definition. `reason` completes the sentence.
    ReachesOutside { name: String, reason: String },
    /// A `region` in a `theorem` or `lemma` body (G6).
    RegionInProof,
}

/// Generates `RegionErrorKind::code()` from a flat variant→code table.
macro_rules! region_error_codes {
    ($($pat:pat => $code:expr),* $(,)?) => {
        impl RegionErrorKind {
            /// Get the error code for this kind.
            pub fn code(&self) -> &'static str {
                match self {
                    $($pat => $code,)*
                }
            }
        }
    };
}

region_error_codes! {
    RegionErrorKind::NotFirstOrder { .. } => "E0094",
    RegionErrorKind::ReachesOutside { .. } => "E0095",
    RegionErrorKind::RegionInProof => "E0096",
}

impl RegionErrorKind {
    /// The default message, word for word the self-host's.
    #[must_use]
    pub fn default_message(&self) -> String {
        match self {
            RegionErrorKind::NotFirstOrder { found } => {
                format!("a region's value must be first-order data, found `{found}`")
            }
            RegionErrorKind::ReachesOutside { name, reason } => {
                format!("a region body reaches `{name}`, {reason}")
            }
            RegionErrorKind::RegionInProof => "a `region` is not allowed in a proof".to_string(),
        }
    }
}

/// E0095's reason for a resource handle reached from `definition`.
#[must_use]
pub fn handle_reason(definition: &str) -> String {
    format!("a resource handle, in `{definition}`")
}

/// E0095's reason for an outer variable whose type holds a type parameter.
pub const TYPE_PARAMETER_REASON: &str = "whose type holds a type parameter";
