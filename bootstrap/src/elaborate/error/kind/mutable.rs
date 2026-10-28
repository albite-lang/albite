//! Mutable-binding errors, E0104–E0106 (ADR 13.10.26r D4): a nested kind with
//! its own one-arm-per-line code table, the shape `region.rs` uses. The
//! self-host raises the same three as E0740–E0742, with the same sentences.

use serde::{Deserialize, Serialize};

/// Refusals of `let mut` and assignment (`B4.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutableErrorKind {
    /// The assigned name's innermost binding is not `let mut`: a parameter,
    /// a plain `let`, a pattern variable or a global.
    NotMut { name: String },
    /// The assignment targets a binding declared outside the block that holds
    /// it, and that block is not in a join position; or it sits in an arm that
    /// leaves the construct through `exit` (`return` or `?`).
    NotJoined { name: String, exit: Option<String> },
    /// A closure mentions a `mut` binding declared outside it (M3).
    Captured { name: String },
    /// A region body assigns a `mut` binding declared outside it (M6).
    InRegion { name: String },
}

/// Generates `MutableErrorKind::code()` from a flat variant→code table.
macro_rules! mutable_error_codes {
    ($($pat:pat => $code:expr),* $(,)?) => {
        impl MutableErrorKind {
            /// Get the error code for this kind.
            pub fn code(&self) -> &'static str {
                match self {
                    $($pat => $code,)*
                }
            }
        }
    };
}

mutable_error_codes! {
    MutableErrorKind::NotMut { .. } => "E0104",
    MutableErrorKind::NotJoined { .. } => "E0104",
    MutableErrorKind::Captured { .. } => "E0105",
    MutableErrorKind::InRegion { .. } => "E0106",
}

impl MutableErrorKind {
    /// The default message, word for word the self-host's.
    #[must_use]
    pub fn default_message(&self) -> String {
        match self {
            MutableErrorKind::NotMut { name } => {
                format!("cannot assign to `{name}`: it is not declared `let mut`")
            }
            MutableErrorKind::NotJoined { name, exit: None } => format!(
                "cannot assign to `{name}` here: only an `if`, `match` or block statement \
                 hands an assignment back to later code"
            ),
            MutableErrorKind::NotJoined {
                name,
                exit: Some(exit),
            } => format!(
                "cannot assign to `{name}` in a branch that can leave through `{exit}`: \
                 a branch that hands an assignment back cannot exit early"
            ),
            MutableErrorKind::Captured { name } => {
                format!(
                    "a closure cannot mention `{name}`, a `let mut` binding declared outside it"
                )
            }
            MutableErrorKind::InRegion { name } => {
                format!(
                    "cannot assign to `{name}` inside a region: it is declared outside the region"
                )
            }
        }
    }
}
