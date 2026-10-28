//! Affine resource handle errors, E0092–E0093 (ADR 7.10.26t D7): a nested kind
//! with its own one-arm-per-line code table, the shape `package.rs` and
//! `tactic.rs` use. The self-host raises the same two as E0720–E0721, with
//! the same sentences, so the records gate pairs them.

use serde::{Deserialize, Serialize};

/// Refusals of the affine handle rule (`B4.1`): a `StringBuilder` or `Bytes`
/// used after it was handed over, or captured by a closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandleErrorKind {
    /// A second consuming use of `handle` in `definition`, or an observer
    /// after the consuming one, on one path.
    UsedAfterConsumed { handle: String, definition: String },
    /// A closure in `definition` mentions `handle`, bound outside it.
    CapturedByClosure { handle: String, definition: String },
}

/// Generates `HandleErrorKind::code()` from a flat variant→code table.
macro_rules! handle_error_codes {
    ($($pat:pat => $code:expr),* $(,)?) => {
        impl HandleErrorKind {
            /// Get the error code for this kind.
            pub fn code(&self) -> &'static str {
                match self {
                    $($pat => $code,)*
                }
            }
        }
    };
}

handle_error_codes! {
    HandleErrorKind::UsedAfterConsumed { .. } => "E0092",
    HandleErrorKind::CapturedByClosure { .. } => "E0093",
}

impl HandleErrorKind {
    /// The default message, word for word the self-host's.
    #[must_use]
    pub fn default_message(&self) -> String {
        match self {
            HandleErrorKind::UsedAfterConsumed { handle, definition } => {
                format!("handle `{handle}` is used after it was consumed, in `{definition}`")
            }
            HandleErrorKind::CapturedByClosure { handle, definition } => {
                format!("handle `{handle}` is captured by a closure, in `{definition}`")
            }
        }
    }
}
