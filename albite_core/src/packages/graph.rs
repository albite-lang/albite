//! The dependency graph: a depth-first walk from the application's manifest
//! over canonical directories (ADR 3.10.26m D2), refusing a cycle (D3) and
//! resolving a diamond to one package (D4).
//!
//! Pure over injected probes, like slice 1's planner it replaces: the walk
//! never touches the filesystem itself, so every rule is testable in memory
//! (`graph_tests.rs`). Every refusal in the graph is collected, not only the
//! first.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::path::{Component, Path, PathBuf};

use super::manifest::{
    manifest_in, parse_manifest, Dependency, Manifest, LEGACY_MANIFEST_FILE, LIBRARY_ROOT_FILE,
    MANIFEST_FILE,
};
use super::{manifest_dir, ByteRange, PlanNote, PlanProblem, PlanProblemKind};

/// The filesystem, as the walk sees it.
pub struct Probes<'a> {
    /// A directory's canonical form — a package's identity — or why not.
    pub canonical: &'a dyn Fn(&Path) -> Result<PathBuf, String>,
    pub exists: &'a dyn Fn(&Path) -> bool,
    pub read: &'a dyn Fn(&Path) -> Option<String>,
    /// The caller's reserved words, refused as package names (ADR 6.10.26j D2).
    pub reserved: &'a [&'a str],
}

/// One package of the graph, checked and named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPackage {
    /// Its `[package] name`, which every key naming it equals (D3 of 29.9.26g).
    pub name: String,
    /// Its `lib.tg`, under the directory as first spelled, `..` folded away
    /// when that names the same directory: what diagnostics print.
    pub library_root: PathBuf,
    pub manifest: Manifest,
    /// The packages it declared, as indices into [`DependencyPlan::packages`].
    pub declares: Vec<usize>,
    /// The manifest and key that first named it — where a refusal about the
    /// package as a whole is spanned (D6).
    pub introduced_by: (PathBuf, ByteRange),
}

/// The walk's result: every package once, in the order first reached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyPlan {
    pub packages: Vec<PlannedPackage>,
    /// The application's own declarations, as indices into `packages`.
    pub application_declares: Vec<usize>,
    /// Non-fatal remarks, one per legacy manifest path, in the order met
    /// (ADR 20.10.26i D2).
    pub notes: Vec<PlanNote>,
}

/// Walk the graph from `app` (D2). `app_modules` are the application's
/// top-level modules, which share the root with every package (D4).
///
/// # Errors
/// Every refusal in the graph, not only the first.
pub fn plan_graph<S: BuildHasher>(
    app: &Manifest,
    app_modules: &HashSet<String, S>,
    probes: &Probes<'_>,
) -> Result<DependencyPlan, Vec<PlanProblem>> {
    let app_dir = manifest_dir(app);
    let app_identity = (probes.canonical)(&app_dir).unwrap_or(app_dir);
    let mut walk = Walk {
        probes,
        app_modules,
        plan: DependencyPlan::default(),
        by_dir: HashMap::new(),
        by_name: HashMap::new(),
        stack: vec![(app_identity, app.name.clone())],
        errors: Vec::new(),
    };
    if let Some(note) = application_note(app, probes) {
        walk.note(note);
    }
    walk.plan.application_declares = walk.declared_by(app);
    if walk.errors.is_empty() {
        Ok(walk.plan)
    } else {
        Err(walk.errors)
    }
}

/// The application manifest's note: read under the old name, or beside an
/// old file it shadows (ADR 20.10.26i D1). Its own existence is not probed —
/// the caller already read it.
fn application_note(app: &Manifest, probes: &Probes<'_>) -> Option<PlanNote> {
    if app
        .path
        .file_name()
        .is_some_and(|name| name == LEGACY_MANIFEST_FILE)
    {
        return Some(PlanNote::legacy_name(&app.path));
    }
    let legacy = manifest_dir(app).join(LEGACY_MANIFEST_FILE);
    (probes.exists)(&legacy).then(|| PlanNote::shadowed(&legacy))
}

/// The walk's state.
struct Walk<'p, S> {
    probes: &'p Probes<'p>,
    app_modules: &'p HashSet<String, S>,
    plan: DependencyPlan,
    /// Canonical directory → its package, or `None` once it was refused, so
    /// a second path to a broken package reports nothing twice.
    by_dir: HashMap<PathBuf, Option<usize>>,
    /// Package name → the directory that claimed it (D4's uniqueness).
    by_name: HashMap<String, PathBuf>,
    /// The packages being walked, outermost first: canonical dir and name.
    stack: Vec<(PathBuf, String)>,
    errors: Vec<PlanProblem>,
}

/// A refusal spanned onto `dep`'s key in the manifest that declares it.
fn on_key(from: &Manifest, dep: &Dependency, kind: PlanProblemKind) -> PlanProblem {
    PlanProblem::new(kind, &from.path, dep.key_span)
}

impl<S: BuildHasher> Walk<'_, S> {
    /// Record `note` unless its path already has one: a diamond notes once.
    fn note(&mut self, note: PlanNote) {
        if !self.plan.notes.iter().any(|n| n.file == note.file) {
            self.plan.notes.push(note);
        }
    }

    /// Visit each of `manifest`'s dependencies in manifest order; the ones
    /// that planned, as indices.
    fn declared_by(&mut self, manifest: &Manifest) -> Vec<usize> {
        let mut declared = Vec::new();
        for dep in &manifest.dependencies {
            if let Some(index) = self.visit(manifest, dep) {
                if !declared.contains(&index) {
                    declared.push(index);
                }
            }
        }
        declared
    }

    /// D2's checks, in its fixed order, then the package's own dependencies.
    fn visit(&mut self, from: &Manifest, dep: &Dependency) -> Option<usize> {
        if self.app_modules.contains(&dep.key) {
            self.errors.push(on_key(
                from,
                dep,
                PlanProblemKind::PackageNameConflict {
                    dependency: dep.key.clone(),
                    reason: "the application already has a top-level module of that name".into(),
                },
            ));
            return None;
        }
        let dir = manifest_dir(from).join(&dep.path);
        let not_found = |reason: String| {
            let dependency = dep.key.clone();
            on_key(
                from,
                dep,
                PlanProblemKind::DependencyNotFound { dependency, reason },
            )
        };
        if !(self.probes.exists)(&dir) {
            self.errors
                .push(not_found(format!("`{}` does not exist", dir.display())));
            return None;
        }
        let canonical = match (self.probes.canonical)(&dir) {
            Ok(canonical) => canonical,
            Err(reason) => {
                let why = format!("`{}` cannot be resolved: {reason}", dir.display());
                self.errors.push(not_found(why));
                return None;
            }
        };
        if let Some(chain) = self.cycle_through(&canonical) {
            let cycle = PlanProblemKind::DependencyCycle { chain };
            self.errors.push(on_key(from, dep, cycle));
            return None;
        }
        if let Some(seen) = self.by_dir.get(&canonical) {
            return *seen;
        }
        let display = display_dir(&dir, &canonical, self.probes);
        let planned = self.check_package(from, dep, &display);
        let index = planned.and_then(|manifest| self.admit(from, dep, &display, manifest));
        self.by_dir.insert(canonical.clone(), index);
        let index = index?;
        let manifest = self.plan.packages[index].manifest.clone();
        self.stack.push((canonical, manifest.name.clone()));
        let declares = self.declared_by(&manifest);
        self.stack.pop();
        self.plan.packages[index].declares = declares;
        Some(index)
    }

    /// The chain by name, application first, when `canonical` is already
    /// being walked (D3): the package that closes it appears twice.
    fn cycle_through(&self, canonical: &Path) -> Option<Vec<String>> {
        let start = self.stack.iter().position(|(dir, _)| dir == canonical)?;
        let mut chain: Vec<String> = self.stack.iter().map(|(_, name)| name.clone()).collect();
        chain.push(self.stack[start].1.clone());
        Some(chain)
    }

    /// D2's checks (5) and (6): the directory holds a manifest and a
    /// `lib.tg`, and the manifest parses and is named as its key.
    fn check_package(&mut self, from: &Manifest, dep: &Dependency, dir: &Path) -> Option<Manifest> {
        let not_found = |reason: String| {
            let dependency = dep.key.clone();
            on_key(
                from,
                dep,
                PlanProblemKind::DependencyNotFound { dependency, reason },
            )
        };
        let chosen = manifest_in(dir, self.probes.exists);
        let Some((manifest_path, note, text)) =
            chosen.and_then(|(path, note)| Some((path.clone(), note, (self.probes.read)(&path)?)))
        else {
            let reason = format!(
                "no `{MANIFEST_FILE}` (or `{LEGACY_MANIFEST_FILE}`) in `{}`",
                dir.display()
            );
            self.errors.push(not_found(reason));
            return None;
        };
        if let Some(note) = note {
            self.note(note);
        }
        if !(self.probes.exists)(&dir.join(LIBRARY_ROOT_FILE)) {
            let reason = format!("no `{LIBRARY_ROOT_FILE}` in `{}`", dir.display());
            self.errors.push(not_found(reason));
            return None;
        }
        let manifest = match parse_manifest(&text, &manifest_path, self.probes.reserved) {
            Ok(manifest) => manifest,
            Err(mut refusals) => {
                self.errors.append(&mut refusals);
                return None;
            }
        };
        if manifest.name != dep.key {
            self.errors.push(on_key(
                from,
                dep,
                PlanProblemKind::PackageNameConflict {
                    dependency: dep.key.clone(),
                    reason: format!(
                        "that package is named `{}`; the key must match it",
                        manifest.name
                    ),
                },
            ));
            return None;
        }
        Some(manifest)
    }

    /// Record a checked package, unless another directory already took its
    /// name (D4: names are unique across the graph).
    fn admit(
        &mut self,
        from: &Manifest,
        dep: &Dependency,
        dir: &Path,
        manifest: Manifest,
    ) -> Option<usize> {
        if let Some(other) = self.by_name.get(&manifest.name) {
            let reason = format!(
                "two packages are named `{}`: `{}` and `{}`",
                manifest.name,
                other.display(),
                dir.display()
            );
            let dependency = dep.key.clone();
            let kind = PlanProblemKind::PackageNameConflict { dependency, reason };
            self.errors.push(on_key(from, dep, kind));
            return None;
        }
        self.by_name
            .insert(manifest.name.clone(), dir.to_path_buf());
        self.plan.packages.push(PlannedPackage {
            name: manifest.name.clone(),
            library_root: dir.join(LIBRARY_ROOT_FILE),
            manifest,
            declares: Vec::new(),
            introduced_by: (from.path.clone(), dep.key_span),
        });
        Some(self.plan.packages.len() - 1)
    }
}

/// The directory a package's files are read from and printed as: `dir` with
/// `.` and `..` folded away lexically, when that still names `canonical` —
/// a symlink can make the folded path another directory, and then `dir`.
fn display_dir(dir: &Path, canonical: &Path, probes: &Probes<'_>) -> PathBuf {
    let folded = fold_dots(dir);
    let same = (probes.canonical)(&folded).is_ok_and(|c| c == canonical);
    if same {
        folded
    } else {
        dir.to_path_buf()
    }
}

/// `path` with `.` dropped and each `..` cancelling the segment before it,
/// lexically; a leading `..` is kept. An empty result is `.`.
#[must_use]
pub fn fold_dots(path: &Path) -> PathBuf {
    let mut parts: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if matches!(parts.last(), Some(Component::Normal(_))) => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return PathBuf::from(".");
    }
    parts.iter().collect()
}

// Tests: graph_tests.rs — the walk over in-memory probes (ADR 3.10.26m AC 1–3)
#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
