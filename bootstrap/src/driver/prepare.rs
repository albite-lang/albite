//! Module tree preparation shared by the run/test and compile paths.
//!
//! Moved out of `driver/mod.rs` for the 400-LOC file limit (ADR 2.10.26j);
//! the companion policy (D2) and the attach step (D3, D5) were added here.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::modules::companion::{self, CompanionPolicy};
use super::modules::{self, build_source_map, flatten_module_tree};
use super::{
    build_module_info, packages, parse_module_tree, pipeline, render_diagnostics,
    render_diagnostics_with_source_map, ModuleInfo, ParsedModule, PipelineError, SourceMap,
};
use crate::cache::BuildCache;

/// Shared module tree preparation used by both `run_file_with_options` and `elaborate_project`.
pub(super) struct PreparedProject {
    pub(super) module_tree: ParsedModule,
    pub(super) source: String,
    pub(super) source_map: SourceMap,
    pub(super) module_info: ModuleInfo,
}

/// Parse module tree, discover siblings, check for parse errors, and build
/// combined AST. Companions are ignored (ADR 2.10.26j D2).
pub(super) fn prepare_project(
    path: &Path,
    verbose: bool,
    cache: Option<&Mutex<BuildCache>>,
) -> Result<PreparedProject, PipelineError> {
    prepare_project_with(path, verbose, cache, CompanionPolicy::Ignore)
}

/// [`prepare_project`] under an explicit companion policy: `Attach` joins each
/// paired module's test companion to it before module info is built.
pub(super) fn prepare_project_with(
    path: &Path,
    verbose: bool,
    cache: Option<&Mutex<BuildCache>>,
    policy: CompanionPolicy,
) -> Result<PreparedProject, PipelineError> {
    // Reset the comparator request registry so stale entries from a prior
    // in-process invocation cannot leak (ADR 29.6.26f §T11.8). Shared by the
    // run/test (`run_file_with_options`) and compile (`elaborate_project`) paths.
    crate::comparator::requests::clear();

    // Parse the module tree (handles `mod foo;` declarations)
    // Use parallel pre-parsing when available (ADR 11.5.26b §P3)
    let mut visited = HashSet::new();
    let mut chain = Vec::new();

    let project_dir = modules::parse::entry_dir(path);
    let tg_files = modules::parse::discover_tg_files(project_dir);
    let mut module_tree = if tg_files.len() > 1 {
        let preparsed = modules::parse::parse_files_parallel(&tg_files);
        modules::parse::parse_module_tree_with_preparsed(
            path,
            &mut visited,
            &mut chain,
            cache,
            &preparsed,
        )?
    } else {
        parse_module_tree(path, &mut visited, &mut chain, cache)?
    };

    // Discover and parse sibling modules for cross-module imports — from the
    // manifest's directory when there is one (ADR 29.9.26g D1)
    let manifest = packages::load_manifest(path);
    let workspace_root = packages::sibling_root(path, &manifest);
    let sibling_modules = modules::parse_workspace_modules(&workspace_root, cache);
    let workspace_module_info = modules::build_workspace_module_info(&sibling_modules);
    let mounted =
        packages::mount_packages(path, manifest, &mut module_tree, &sibling_modules, cache)
            .map(|mounted| mounted.announce(verbose))?;

    if verbose && !sibling_modules.is_empty() {
        let sibling_names: Vec<_> = sibling_modules
            .iter()
            .map(|m| modules::get_module_name_from_parsed(m))
            .collect();
        eprintln!(
            "Discovered {} sibling module(s) at workspace root {}: {sibling_names:?}",
            sibling_modules.len(),
            workspace_root.display(),
        );
    }

    // Flatten all modules
    let all_items = flatten_module_tree(&module_tree);

    if verbose {
        eprintln!(
            "Parsed {} module(s) with {} total item(s)",
            pipeline::count_modules(&module_tree),
            all_items.len()
        );
    }

    // Read source for diagnostics
    let source = fs::read_to_string(path)
        .map_err(|e| PipelineError::IoError(path.display().to_string(), e.to_string()))?;

    // Build source map for multi-file error reporting
    let mut source_map = build_source_map(&module_tree);
    for manifest in mounted.manifests {
        source_map.insert(manifest.path, manifest.text);
    }

    // The reserved byte intrinsic is the native core's alone (ADR 4.10.26n
    // D5): refused here, where modules are still known, so `check`, `run`,
    // `test` and `compile` all refuse a user declaration of it.
    super::native_core::refuse_reserved_declarations(&module_tree, &|module_path| {
        source_map.sources.get(module_path).cloned()
    })
    .map_err(PipelineError::ElabFailed)?;

    let pairs = match policy {
        CompanionPolicy::Attach => {
            companion::pairs_in_tree(&module_tree, &canonical_index(&tg_files))
        }
        CompanionPolicy::Ignore => Vec::new(),
    };
    check_parse_and_attach(&mut module_tree, &pairs, &mut source_map, (path, &source))?;

    // Build module info (no combined AST — per-module elaboration, ADR 5.5.26c)
    // Main module info is base (priority) so its module paths, use_statement
    // mappings, and file_to_module entries take precedence over workspace
    // sibling duplicates (ADR 8.5.26a).
    let file_module_info = build_module_info(&module_tree);
    let mut module_info = modules::merge_module_info(file_module_info, workspace_module_info);
    module_info.packages = mounted.table;

    Ok(PreparedProject {
        module_tree,
        source,
        source_map,
        module_info,
    })
}

/// The Core-name table elaboration of `path` uses (ADR 13.10.26a D1): built
/// from the registry [`prepare_project`] assembles — workspace siblings
/// merged, packages mounted — so a name scoped only because a sibling also
/// defines it is scoped here too. Parse-level; elaborates nothing.
pub fn project_core_names(
    path: &Path,
    verbose: bool,
) -> Result<std::sync::Arc<crate::elaborate::CoreNames>, PipelineError> {
    Ok(prepare_project(path, verbose, None)?
        .module_info
        .core_names())
}

/// Report every parse error — each paired companion's too, parsed alone, so
/// its errors land in its own lines — then attach the companions (ADR
/// 2.10.26j D3). A name defined twice in one module, on both sides of a
/// companion (D5) or within one file (ADR 13.10.26f D2), is E0004, reported
/// like a parse failure, before any elaboration. `entry` is the entry's path
/// and text, the renderer's fallback.
fn check_parse_and_attach(
    module_tree: &mut ParsedModule,
    pairs: &[companion::CompanionPair],
    source_map: &mut SourceMap,
    entry: (&Path, &str),
) -> Result<(), PipelineError> {
    let mut source_map_vec = Vec::new();
    let parse_errors = modules::collect_parse_errors(module_tree, &mut source_map_vec);
    let companion_failures = companion::companion_parse_failures(pairs);
    if !parse_errors.is_empty() || !companion_failures.is_empty() {
        for (file_path, errors) in &parse_errors {
            if let Some((_, src)) = source_map_vec.iter().find(|(p, _)| p == file_path) {
                render_diagnostics(src, &file_path.display().to_string(), &[], errors);
            }
        }
        for (file_path, src, errors) in &companion_failures {
            render_diagnostics(src, &file_path.display().to_string(), &[], errors);
        }
        return Err(PipelineError::ElabFailed("parse errors".to_string()));
    }

    companion::attach_companions(module_tree, pairs, source_map)?;
    let mut collisions = modules::definitions::duplicate_errors(module_tree, source_map);
    if !collisions.is_empty() {
        for error in &mut collisions {
            modules::relocate_error(error, source_map);
        }
        let (path, source) = entry;
        let filename = path.to_string_lossy();
        render_diagnostics_with_source_map(source, &filename, source_map, &collisions, &[]);
        return Err(PipelineError::ElabFailed("parse errors".to_string()));
    }
    Ok(())
}

/// The companion index over canonical paths (ADR 2.10.26j P0 item 4): the
/// tree spells a bare entry `main.tg` where discovery spells it `./main.tg`,
/// so only canonical keys meet.
fn canonical_index(tg_files: &[PathBuf]) -> std::collections::BTreeMap<PathBuf, PathBuf> {
    let canonical: Vec<PathBuf> = tg_files
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    companion::companion_index(&canonical)
}

#[cfg(test)]
mod project_core_names_tests {
    use super::project_core_names;

    /// 13.10.26a D1 — the table is the registry's, read from a real module
    /// tree: a reserved name and a two-module collision are both scoped.
    #[test]
    fn the_table_scopes_a_reserved_name_and_a_collision_from_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
        file(
            "main.tg",
            "mod x;\nmod y;\nfn write(n: Nat) -> Nat { n }\n\
             fn main() -> Nat { x::helper(1) + y::helper(write(1)) }\n",
        );
        file("x.tg", "pub fn helper(n: Nat) -> Nat { n + 10 }\n");
        file("y.tg", "pub fn helper(n: Nat) -> Nat { n + 100 }\n");

        let table = project_core_names(&dir.path().join("main.tg"), false).unwrap();
        assert_eq!(table.source_name("_tg_5write"), "write");
        assert_eq!(table.source_name("_tg_1x_6helper"), "helper");
        assert_eq!(table.source_name("_tg_1y_6helper"), "helper");
        assert!(table.origin("main").is_none());
    }

    /// A file that cannot be prepared is an error, not an empty table.
    #[test]
    fn a_missing_entry_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_core_names(&dir.path().join("absent.tg"), false).is_err());
    }
}
