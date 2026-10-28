//! Local path dependencies: an `albite.toml` names library folders, each
//! library may name its own, and every package's `lib.tg` is mounted once as
//! a named, public child of the root module tree (ADR 29.9.26g, I9 slice 1;
//! ADR 3.10.26m, slice 2a).
//!
//! Mounting grafts into the tree rather than riding the sibling merge, which
//! registers module names and elaborates no body (D4 of 29.9.26g). Everything
//! that decides — the graph walk (`albite_core::packages`, shared with the
//! self-host since ADR 6.10.26j), which names clash with a library's own
//! modules, and which top-level names two packages share (D8) — is a pure
//! function over injected probes and parsed trees; [`mount_packages`] is the
//! one effectful seam `prepare_project` calls.

#[cfg(test)]
mod elaboration_tests;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use albite_core::packages::{
    parse_manifest, plan_graph, ByteRange, DependencyPlan, Manifest, PlanProblem, PlanProblemKind,
    PlannedPackage, Probes,
};

use crate::ast::Item;
use crate::cache::BuildCache;
use crate::elaborate::{
    ElabError, ElabErrorKind, ModulePath, PackageErrorKind, PackageId, PackageTable,
};
use crate::span::Span;
use crate::token::ALL_KEYWORDS;

use super::diagnostics::compile_failure::{report_note, Sinks};
use super::modules::{get_module_name_from_parsed, parse_module_tree, ParsedModule};
use super::PipelineError;

pub(crate) use albite_core::packages::{find_manifest, manifest_dir};

/// A core byte range as a bootstrap span.
pub(crate) fn span_of(range: ByteRange) -> Span {
    Span::new(range.start, range.end)
}

/// The one site a planner refusal becomes an `ElabError` (ADR 6.10.26j D2):
/// the same kind, span and file. Its message is the bootstrap's own rendering,
/// which the core's equals word for word (AC 1).
pub(crate) fn plan_problem_error(problem: PlanProblem) -> ElabError {
    let kind = match problem.kind {
        PlanProblemKind::ManifestInvalid { reason } => PackageErrorKind::ManifestInvalid { reason },
        PlanProblemKind::DependencyNotFound { dependency, reason } => {
            PackageErrorKind::DependencyNotFound { dependency, reason }
        }
        PlanProblemKind::PackageNameConflict { dependency, reason } => {
            PackageErrorKind::PackageNameConflict { dependency, reason }
        }
        PlanProblemKind::DependencyCycle { chain } => PackageErrorKind::DependencyCycle { chain },
    };
    ElabError::new(span_of(problem.range), ElabErrorKind::Package(kind))
        .with_file_path(problem.file)
}

/// Every refusal of a plan, converted.
fn plan_problem_errors(problems: Vec<PlanProblem>) -> Vec<ElabError> {
    problems.into_iter().map(plan_problem_error).collect()
}

/// What mounting produced: the package table for resolution and visibility,
/// and every manifest read, for the source map.
#[derive(Debug, Default)]
pub(crate) struct MountedPackages {
    pub(crate) table: PackageTable,
    pub(crate) manifests: Vec<Manifest>,
}

/// Read and parse the manifest governing `entry` (D1), if there is one.
pub(crate) fn load_manifest(entry: &Path) -> Result<Option<Manifest>, Vec<ElabError>> {
    let Some(path) = find_manifest(entry, Path::is_file) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    parse_manifest(&text, &path, ALL_KEYWORDS)
        .map(Some)
        .map_err(plan_problem_errors)
}

/// Where sibling discovery runs for `entry` (D1): the manifest's directory
/// when one was found, else today's `find_workspace_root`, unchanged.
pub(crate) fn sibling_root(
    entry: &Path,
    manifest: &Result<Option<Manifest>, Vec<ElabError>>,
) -> PathBuf {
    match manifest {
        Ok(Some(found)) => manifest_dir(found),
        _ => super::modules::find_workspace_root(entry),
    }
}

impl MountedPackages {
    /// Print the mounted packages under `--verbose` (D11), beside the
    /// sibling list `prepare_project` already prints.
    pub(crate) fn announce(self, verbose: bool) -> Self {
        if let Some(line) = self.announcement(verbose) {
            eprintln!("{line}");
        }
        self
    }

    /// The `--verbose` line, when there is one: nothing without the flag or
    /// without a mounted package. The first manifest is the application's;
    /// every package after it is listed, transitive ones included.
    pub(crate) fn announcement(&self, verbose: bool) -> Option<String> {
        if !verbose || self.table.is_empty() {
            return None;
        }
        let names: Vec<_> = self.manifests.iter().skip(1).map(|m| &m.name).collect();
        Some(format!("Mounted {} package(s): {names:?}", names.len()))
    }
}

/// ADR 3.10.26m D4: a library's dependency key equal to one of that
/// library's own top-level modules, so `use units::f` would be ambiguous.
/// `own_modules[i]` are package `i`'s top-level module names.
pub(crate) fn library_module_clashes(
    plan: &DependencyPlan,
    own_modules: &[HashSet<String>],
) -> Vec<ElabError> {
    let mut errors = Vec::new();
    for (package, modules) in plan.packages.iter().zip(own_modules) {
        for dep in &package.manifest.dependencies {
            if !modules.contains(&dep.key) {
                continue;
            }
            let kind = PackageErrorKind::PackageNameConflict {
                dependency: dep.key.clone(),
                reason: format!(
                    "`{}` already has a top-level module of that name",
                    package.name
                ),
            };
            errors.push(
                ElabError::new(span_of(dep.key_span), ElabErrorKind::Package(kind))
                    .with_file_path(&package.manifest.path),
            );
        }
    }
    errors
}

/// Where an extern is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Definition {
    pub(crate) file: PathBuf,
    pub(crate) span: Span,
}

/// Every C symbol a module tree's externs declare — the explicit `symbol`,
/// else the extern's name — first declaration kept. Functions, theorems,
/// lemmas and axioms are no longer counted: a value's identity is its module
/// and name, so two packages may share one (ADR 4.10.26j D6). An extern's C
/// symbol is its identity, so two packages declaring one still collide.
pub(crate) fn extern_names(trees: &[&ParsedModule]) -> BTreeMap<String, Definition> {
    let mut names = BTreeMap::new();
    let mut stack: Vec<&ParsedModule> = trees.to_vec();
    while let Some(module) = stack.pop() {
        for item in &module.source_file.items {
            let Item::ExternFn(extern_fn) = item else {
                continue;
            };
            let symbol = extern_fn
                .symbol
                .clone()
                .unwrap_or_else(|| extern_fn.name.name.clone());
            names.entry(symbol).or_insert(Definition {
                file: module.path.clone(),
                span: extern_fn.name.span,
            });
        }
        stack.extend(module.submodules.iter());
    }
    names
}

/// D8 of 29.9.26g, applied across the whole graph (3.10.26m D6) and narrowed
/// to externs (ADR 4.10.26j D6): a C symbol declared by two packages, refused
/// up front and spanned onto the key that introduced the later package, in
/// the manifest holding it. Each package is passed once, so a diamond's
/// shared one never meets itself; symbols duplicated inside one package are
/// not this check's business.
pub(crate) fn cross_package_collisions(
    app: &Manifest,
    app_names: &BTreeMap<String, Definition>,
    packages: &[(&PlannedPackage, BTreeMap<String, Definition>)],
) -> Vec<ElabError> {
    let mut owners: Vec<(&str, &BTreeMap<String, Definition>)> = vec![(&app.name, app_names)];
    let mut errors = Vec::new();
    for (package, names) in packages {
        let (manifest, key_span) = &package.introduced_by;
        for (name, definition) in names {
            let Some((owner, earlier)) = owners
                .iter()
                .find_map(|(owner, map)| map.get(name).map(|d| (*owner, d)))
            else {
                continue;
            };
            let kind = PackageErrorKind::CrossPackageNameCollision {
                name: name.clone(),
                packages: [owner.to_string(), package.name.clone()],
            };
            errors.push(
                ElabError::new(span_of(*key_span), ElabErrorKind::Package(kind))
                    .with_file_path(manifest)
                    .with_cross_file_note(
                        earlier.span,
                        earlier.file.clone(),
                        format!("`{name}` defined in `{owner}` here"),
                    )
                    .with_cross_file_note(
                        definition.span,
                        definition.file.clone(),
                        format!("and in `{}` here", package.name),
                    )
                    .with_help(format!(
                        "declare `{name}` once, in one package, and import it from there"
                    )),
            );
        }
        owners.push((&package.name, names));
    }
    errors
}

/// The package table a plan describes (3.10.26m §2.3): every package mounted
/// at the root under its own name, with each one's declarations.
pub(crate) fn package_table(app: &Manifest, plan: &DependencyPlan) -> PackageTable {
    let mut table = PackageTable::default();
    table.set_application_name(app.name.clone());
    for package in &plan.packages {
        table.add_package(
            package.name.clone(),
            ModulePath::from_name(package.name.clone()),
        );
    }
    for &index in &plan.application_declares {
        table.declare(PackageId::Application, index);
    }
    for (from, package) in plan.packages.iter().enumerate() {
        for &index in &package.declares {
            table.declare(PackageId::Dependency(from), index);
        }
    }
    table
}

/// The plan's non-fatal notes (ADR 20.10.26i D2) — a manifest still named
/// `tungsten.toml` — rendered once planning succeeded, in the selected
/// `--error-format`. The planner returns them as data; only the driver prints.
fn report_plan_notes(plan: &DependencyPlan) {
    let format = super::diagnostics::error_format();
    let (stdout, stderr) = (std::io::stdout(), std::io::stderr());
    let mut sinks = Sinks {
        stdout: &mut stdout.lock(),
        stderr: &mut stderr.lock(),
    };
    for note in &plan.notes {
        report_note(&note.message, format, &mut sinks);
    }
}

/// Mount every package of the graph governed by `entry`'s manifest into
/// `tree`, once each (3.10.26m D4), after checking them (D2–D6). Refusals
/// are rendered here, like parse errors, and end the run with exit 1.
pub(crate) fn mount_packages(
    entry: &Path,
    manifest: Result<Option<Manifest>, Vec<ElabError>>,
    tree: &mut ParsedModule,
    siblings: &[ParsedModule],
    cache: Option<&Mutex<BuildCache>>,
) -> Result<MountedPackages, PipelineError> {
    let report = |errors: Vec<ElabError>| {
        let mut map = super::SourceMap::new();
        // Errors span into manifests not yet parsed and notes into `.tg` files
        // the tree has not yet mapped (the source map is built after mounting).
        let spanned = errors.iter().flat_map(|e| {
            let notes = e.notes.iter().filter_map(|n| n.file_path.as_ref());
            e.file_path.iter().chain(notes)
        });
        for file in spanned {
            if map.get(file).is_none() {
                if let Ok(text) = std::fs::read_to_string(file) {
                    map.insert(file.clone(), text);
                }
            }
        }
        let text = std::fs::read_to_string(entry).unwrap_or_default();
        super::render_diagnostics_with_source_map(
            &text,
            &entry.to_string_lossy(),
            &map,
            &errors,
            &[],
        );
        PipelineError::ElabFailed("parse errors".to_string())
    };
    let app = match manifest {
        Ok(Some(app)) => app,
        Ok(None) => return Ok(MountedPackages::default()),
        Err(errors) => return Err(report(errors)),
    };
    let app_modules: HashSet<String> = tree
        .submodules
        .iter()
        .chain(siblings)
        .map(get_module_name_from_parsed)
        .collect();
    let probes = Probes {
        canonical: &|p| std::fs::canonicalize(p).map_err(|e| e.to_string()),
        exists: &|p| p.exists(),
        read: &|p| std::fs::read_to_string(p).ok(),
        reserved: ALL_KEYWORDS,
    };
    let plan = plan_graph(&app, &app_modules, &probes)
        .map_err(|problems| report(plan_problem_errors(problems)))?;
    report_plan_notes(&plan);

    let mut libraries = Vec::new();
    for package in &plan.packages {
        let (mut visited, mut chain) = (HashSet::new(), Vec::new());
        let mut library =
            parse_module_tree(&package.library_root, &mut visited, &mut chain, cache)?;
        library.mount_name = Some(package.name.clone());
        library.visibility = crate::ast::Visibility::Public;
        libraries.push(library);
    }
    let own_modules: Vec<HashSet<String>> = libraries
        .iter()
        .map(|library| {
            library
                .submodules
                .iter()
                .map(get_module_name_from_parsed)
                .collect()
        })
        .collect();
    let clashes = library_module_clashes(&plan, &own_modules);
    if !clashes.is_empty() {
        return Err(report(clashes));
    }
    let app_trees: Vec<&ParsedModule> = std::iter::once(&*tree).chain(siblings).collect();
    let app_names = extern_names(&app_trees);
    let package_names: Vec<_> = plan
        .packages
        .iter()
        .zip(&libraries)
        .map(|(package, library)| (package, extern_names(&[library])))
        .collect();
    let collisions = cross_package_collisions(&app, &app_names, &package_names);
    if !collisions.is_empty() {
        return Err(report(collisions));
    }

    let table = package_table(&app, &plan);
    tree.submodules.extend(libraries);
    let mut manifests = vec![app];
    manifests.extend(plan.packages.into_iter().map(|p| p.manifest));
    Ok(MountedPackages { table, manifests })
}
