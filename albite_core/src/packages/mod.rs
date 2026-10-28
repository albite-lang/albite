//! The package planner: reading `albite.toml` and walking the dependency
//! graph it names (ADR 29.9.26g, 3.10.26m), shared by both compilers.
//!
//! It moved here from the bootstrap's `driver/packages/` (ADR 6.10.26j D2) so
//! the self-host plans through the same code over an FFI (`ffi::packages`),
//! as the termination mirror shares its engine (ADR 19.8.26d). A user program
//! that never plans carries none of it: user links strip unreferenced
//! sections (ADR 6.10.26i).
//!
//! The planner speaks neither compiler's error type. A refusal is a
//! [`PlanProblem`] — a kind, a file, a byte range and the message — which the
//! bootstrap turns into its `ElabError` at one site and the self-host into a
//! located record. The reserved words are an input for the same reason: the
//! bootstrap's list lives in its lexer, which the core cannot see.

pub mod graph;
pub mod manifest;

// graph_tests.rs is `graph`'s own `#[path]` child.
#[cfg(test)]
mod manifest_tests;
#[cfg(test)]
mod tests_legacy_manifest;

use std::path::{Path, PathBuf};

pub use graph::{fold_dots, plan_graph, DependencyPlan, PlannedPackage, Probes};
pub use manifest::{
    choose_manifest, find_manifest, identifier_problem, manifest_in, parse_manifest, Dependency,
    Manifest, ManifestChoice, LEGACY_MANIFEST_FILE, LIBRARY_ROOT_FILE, MANIFEST_FILE,
};

/// A half-open byte range into one file: where a refusal points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ByteRange {
    pub start: u32,
    pub end: u32,
}

impl ByteRange {
    #[must_use]
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

/// What the planner refused. The bootstrap's `PackageErrorKind` arms the
/// planner raises, minus the extern collision, which needs mounted trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanProblemKind {
    /// A manifest off slice 1's schema (E0120).
    ManifestInvalid { reason: String },
    /// A dependency whose directory, manifest or `lib.tg` is missing (E0121).
    DependencyNotFound { dependency: String, reason: String },
    /// A key disagreeing with its package's name, another package's name or
    /// an application module (E0122).
    PackageNameConflict { dependency: String, reason: String },
    /// A graph that loops back on itself; `chain` names the closing package
    /// twice (E0124).
    DependencyCycle { chain: Vec<String> },
}

impl PlanProblemKind {
    /// The tag the FFI hands the self-host: the bootstrap's code number, so
    /// one number means one refusal on both sides of the seam.
    #[must_use]
    pub fn tag(&self) -> u64 {
        match self {
            PlanProblemKind::ManifestInvalid { .. } => 120,
            PlanProblemKind::DependencyNotFound { .. } => 121,
            PlanProblemKind::PackageNameConflict { .. } => 122,
            PlanProblemKind::DependencyCycle { .. } => 124,
        }
    }

    /// The message both compilers print, word for word.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            PlanProblemKind::ManifestInvalid { reason } => {
                format!("invalid `{MANIFEST_FILE}`: {reason}")
            }
            PlanProblemKind::DependencyNotFound { dependency, reason } => {
                format!("dependency `{dependency}` not found: {reason}")
            }
            PlanProblemKind::PackageNameConflict { dependency, reason } => {
                format!("dependency `{dependency}` conflicts: {reason}")
            }
            PlanProblemKind::DependencyCycle { chain } => {
                format!("dependency cycle: {}", chain.join(" → "))
            }
        }
    }
}

/// One refusal, located: the file it spans into and the byte range there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProblem {
    pub kind: PlanProblemKind,
    pub file: PathBuf,
    pub range: ByteRange,
    pub message: String,
}

impl PlanProblem {
    /// A refusal of `kind` at `range` in `file`, its message rendered here.
    #[must_use]
    pub fn new(kind: PlanProblemKind, file: &Path, range: ByteRange) -> Self {
        let message = kind.message();
        Self {
            kind,
            file: file.to_path_buf(),
            range,
            message,
        }
    }
}

/// A non-fatal remark about a plan (ADR 20.10.26i D2): a manifest read under
/// its old name, or one the new name shadows. Separate from [`PlanProblem`],
/// which refuses; a note never fails a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanNote {
    /// The `tungsten.toml` the note is about: one note per path.
    pub file: PathBuf,
    /// The text both compilers print after `note: `, word for word.
    pub message: String,
}

impl PlanNote {
    /// `file` was read under the old name.
    #[must_use]
    pub fn legacy_name(file: &Path) -> Self {
        Self {
            file: file.to_path_buf(),
            message: format!(
                "`{}` uses the old manifest name; rename it to `{MANIFEST_FILE}`",
                file.display()
            ),
        }
    }

    /// `file` sits beside an `albite.toml`, which is read instead.
    #[must_use]
    pub fn shadowed(file: &Path) -> Self {
        Self {
            file: file.to_path_buf(),
            message: format!(
                "`{}` is ignored: `{MANIFEST_FILE}` beside it is read instead",
                file.display()
            ),
        }
    }
}

/// The directory a manifest governs: its dependencies' paths are relative to
/// it, and sibling discovery runs from it (29.9.26g D1).
#[must_use]
pub fn manifest_dir(manifest: &Manifest) -> PathBuf {
    manifest
        .path
        .parent()
        .unwrap_or(Path::new(""))
        .to_path_buf()
}
