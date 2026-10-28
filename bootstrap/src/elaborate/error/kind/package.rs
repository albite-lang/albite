//! Package errors, E0120–E0124 (ADR 29.9.26g, 3.10.26m D8): a nested kind with
//! its own one-arm-per-line code table, which `explain`'s `package_codes.rs`
//! reads as an oracle beside `codes.rs`, as `tactic.rs` is read for tactics.
//! `kind/mod.rs` had no room for a fifth flat variant; the next package kind
//! lands here, and `ElabErrorKind` keeps its one `Package(_)` arm.

use serde::{Deserialize, Serialize};

/// Errors raised while reading manifests and mounting packages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageErrorKind {
    /// A manifest off slice 1's schema.
    ManifestInvalid { reason: String },
    /// A path dependency whose directory, manifest or `lib.tg` is missing.
    DependencyNotFound { dependency: String, reason: String },
    /// A dependency key disagreeing with its package's name, another
    /// package's name, or a module of the package that declared it.
    PackageNameConflict { dependency: String, reason: String },
    /// A C symbol two packages' externs declare. Functions, theorems,
    /// lemmas and axioms left this arm when values were keyed by module
    /// (ADR 4.10.26j D6); the variant keeps its name and bincode slot.
    CrossPackageNameCollision { name: String, packages: [String; 2] },
    /// A dependency graph that loops back on itself (ADR 3.10.26m D3):
    /// `chain` runs from the application to the package that closes it,
    /// whose name therefore appears twice.
    DependencyCycle { chain: Vec<String> },
}

/// Generates `PackageErrorKind::code()` from a flat variant→code table.
macro_rules! package_error_codes {
    ($($pat:pat => $code:expr),* $(,)?) => {
        impl PackageErrorKind {
            /// Get the error code for this kind.
            pub fn code(&self) -> &'static str {
                match self {
                    $($pat => $code,)*
                }
            }
        }
    };
}

package_error_codes! {
    PackageErrorKind::ManifestInvalid { .. } => "E0120",
    PackageErrorKind::DependencyNotFound { .. } => "E0121",
    PackageErrorKind::PackageNameConflict { .. } => "E0122",
    PackageErrorKind::CrossPackageNameCollision { .. } => "E0123",
    PackageErrorKind::DependencyCycle { .. } => "E0124",
}

impl PackageErrorKind {
    /// The default message.
    #[must_use]
    pub fn default_message(&self) -> String {
        match self {
            PackageErrorKind::ManifestInvalid { reason } => {
                format!("invalid `albite.toml`: {reason}")
            }
            PackageErrorKind::DependencyNotFound { dependency, reason } => {
                format!("dependency `{dependency}` not found: {reason}")
            }
            PackageErrorKind::PackageNameConflict { dependency, reason } => {
                format!("dependency `{dependency}` conflicts: {reason}")
            }
            PackageErrorKind::CrossPackageNameCollision { name, packages } => format!(
                "two packages declare the C symbol `{name}`: package `{}` and package `{}`",
                packages[0], packages[1]
            ),
            PackageErrorKind::DependencyCycle { chain } => {
                format!("dependency cycle: {}", chain.join(" → "))
            }
        }
    }
}
