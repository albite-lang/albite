//! Workspace-aware module discovery.
//!
//! When checking a single file that's part of a larger project, we need to know
//! about sibling modules for cross-module imports. This module provides functions
//! to discover and parse sibling modules at the workspace root level.

use std::collections::hash_map::Entry;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::cache::BuildCache;
use crate::elaborate::ModulePath;

use super::info::{build_module_info_recursive, get_module_name, ModuleInfo};
use super::info_reexports::process_pub_use_reexports;
use super::parse::parse_module_tree;
use super::ParsedModule;

/// Find the workspace root directory for a given file.
///
/// The workspace root is determined by walking up from the file's directory
/// until we find a directory that looks like a project root:
/// 1. Contains `main.tg` (entry point file)
/// 2. OR is the immediate parent of the file being checked
///
/// This allows checking files like `elab/env/mod.tg` to find sibling modules
/// like `lexer` and `parser` at the workspace root level.
///
/// **Sibling re-parse caveat:** Test entry points (e.g., `test_string_utils.tg`)
/// may declare `mod driver; mod parser; mod elab;`, causing the same source files
/// to be re-parsed under different path prefixes. The resulting `ModuleInfo` is
/// merged via [`merge_module_info`], where the priority argument wins for
/// duplicate keys. See ADR 8.5.26a for details.
pub fn find_workspace_root(file_path: &Path) -> PathBuf {
    let file_dir = file_path.parent().unwrap_or(Path::new("."));

    // Walk up from file's directory looking for main.tg
    let mut current = file_dir;
    loop {
        let main_path = current.join("main.tg");
        if main_path.exists() {
            return current.to_path_buf();
        }

        // Move up one directory
        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => break, // Reached filesystem root
        }
    }

    // No main.tg found - use the file's immediate parent directory
    file_dir.to_path_buf()
}

/// Discover all module entry points at the workspace root level.
///
/// Returns paths to all `.tg` files and `*/mod.tg` files at the workspace root.
/// These represent top-level modules that can be imported.
pub fn discover_sibling_modules(workspace_root: &Path) -> Vec<PathBuf> {
    let mut modules = Vec::new();

    // Read directory contents
    let entries = match fs::read_dir(listing_dir(workspace_root)) {
        Ok(e) => e,
        Err(_) => return modules,
    };

    for entry in entries.flatten() {
        let path = sibling_path(workspace_root, &entry.file_name());

        if path.is_file() {
            // Include .tg files (but not mod.tg at root level)
            if let Some(ext) = path.extension() {
                if ext == "tg" {
                    let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    // Skip mod.tg at root - it's not a standalone module
                    if file_name != "mod" {
                        modules.push(path);
                    }
                }
            }
        } else if path.is_dir() {
            // Check for subdirectory with mod.tg
            let mod_path = path.join("mod.tg");
            if mod_path.exists() {
                modules.push(mod_path);
            }
        }
    }

    // A test companion is part of its sibling, never a module of its own, under
    // every command (ADR 2.10.26j D7). The listing is the scan's own: the
    // workspace root may be an ancestor of the entry's directory.
    let companions: std::collections::HashSet<PathBuf> =
        super::companion::companion_index(&modules)
            .into_values()
            .collect();
    modules.retain(|path| !companions.contains(path));
    modules.sort();
    modules
}

/// The directory to list for a workspace root (ADR 30.9.26i D4).
///
/// `find_workspace_root` probes the working directory correctly — `""`
/// joined with `main.tg` is cwd-relative — but then returns `""` as the root
/// for a bare entry, and `fs::read_dir("")` fails, so no sibling was found.
/// The working directory is listed as `.`; every other root is unchanged.
pub fn listing_dir(workspace_root: &Path) -> &Path {
    if workspace_root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        workspace_root
    }
}

/// A sibling's path, spelled from the root rather than taken from
/// `DirEntry::path()`: for the empty root that keeps a bare name (`lexer.tg`,
/// never `./lexer.tg`), so its `PathBuf` key still meets the main tree's
/// (ADR 30.9.26i D4). For a non-empty root the two spellings are identical.
pub fn sibling_path(workspace_root: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    workspace_root.join(file_name)
}
///
/// This is used for workspace-aware resolution: when checking a single file,
/// we also parse its sibling modules so cross-module imports can be resolved.
///
/// **Important:** Sibling modules may re-declare overlapping `mod` subtrees,
/// causing the same source files to be parsed multiple times under different
/// path prefixes. The resulting module info must be merged with
/// [`merge_module_info`], where the main file's info takes priority.
///
/// Returns a vector of parsed module trees, one for each sibling module.
pub fn parse_workspace_modules(
    workspace_root: &Path,
    cache: Option<&Mutex<BuildCache>>,
) -> Vec<ParsedModule> {
    let module_paths = discover_sibling_modules(workspace_root);
    let mut modules = Vec::new();

    for path in module_paths {
        // Parse each module tree independently
        let mut visited = HashSet::new();
        let mut chain = Vec::new();

        match parse_module_tree(&path, &mut visited, &mut chain, cache) {
            Ok(module) => modules.push(module),
            Err(_) => {
                // Skip modules that fail to parse - they might have syntax errors
                // but we still want to be able to check other modules
            }
        }
    }

    modules
}

/// Build module info from multiple module trees.
///
/// This merges module information from all sibling modules, allowing
/// cross-module imports to be resolved. Each module tree is registered
/// at the root level (e.g., `lexer`, `parser`, `elab` as top-level modules).
pub fn build_workspace_module_info(modules: &[ParsedModule]) -> ModuleInfo {
    let mut info = ModuleInfo::default();

    // First pass: register all modules and items
    for module in modules {
        // Each module becomes a top-level module (e.g., "lexer", "parser")
        let module_name = get_module_name(module);
        let module_path = ModulePath::from_segments(&[module_name]);

        // Use the internal recursive function with the correct path
        build_module_info_recursive(module, &module_path, None, &mut info);
    }

    // Second pass: process pub use re-exports
    for module in modules {
        let module_name = get_module_name(module);
        let module_path = ModulePath::from_segments(&[module_name]);
        process_pub_use_reexports(module, &module_path, &mut info);
    }

    info
}

/// Merge two ModuleInfo structures.
///
/// Used to combine the file-specific module info with workspace-wide module info.
/// `priority` (typically the main file's module info) wins for all duplicate keys
/// via `or_insert` — entries from `fallback` are only added when not already
/// present in `priority`. This ensures canonical paths come from the main
/// module registration, not from workspace siblings that may re-parse the
/// same source files under different path prefixes (ADR 8.5.26a).
pub fn merge_module_info(mut priority: ModuleInfo, fallback: ModuleInfo) -> ModuleInfo {
    // The merged registry is a new one: a Core-name table fixed on either
    // half would be stale (ADR 4.10.26j D1).
    priority.core_names = std::sync::OnceLock::new();
    // Main's own files, read BEFORE `file_to_module` absorbs the fallback's.
    let main_files: HashSet<PathBuf> = priority.file_to_module.keys().cloned().collect();

    // Merge file_to_module FIRST (priority/main wins - canonical paths)
    // This is critical for canonicalization: main's paths are the canonical ones
    for (file, path) in fallback.file_to_module {
        priority.file_to_module.entry(file).or_insert(path);
    }

    // Merge modules registry. Each module the merge inserts is marked
    // names-only unless main already owns its file (ADR 2.10.26g D4): this
    // merge has no same-file drop, so such a module is inserted, unmarked.
    // `fallback.names_only` is ignored — the mark is this merge's to make.
    for (path, contents) in fallback.modules {
        if let Entry::Vacant(slot) = priority.modules.entry(path) {
            if !owned_by_main(fallback.module_files.get(slot.key()), &main_files) {
                priority.names_only.insert(slot.key().clone());
            }
            slot.insert(contents);
        }
    }

    // Merge item_modules (item name → module path)
    // Sort by name for deterministic merge order when same item exists in
    // multiple modules (prevents non-deterministic E0016 diagnostic paths).
    let mut sorted_item_modules: Vec<_> = fallback.item_modules.into_iter().collect();
    sorted_item_modules.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (name, path) in sorted_item_modules {
        priority.item_modules.entry(name).or_insert(path);
    }

    // Merge module_visibility
    for (path, vis) in fallback.module_visibility {
        priority.module_visibility.entry(path).or_insert(vis);
    }

    // Merge use_statement_modules (keyed by (file_path, span_start))
    for (key, path) in fallback.use_statement_modules {
        priority.use_statement_modules.entry(key).or_insert(path);
    }

    // Merge use_statement_by_span (keyed by full span)
    for (span, path) in fallback.use_statement_by_span {
        priority.use_statement_by_span.entry(span).or_insert(path);
    }

    // Merge module_files
    for (path, file_path) in fallback.module_files {
        priority.module_files.entry(path).or_insert(file_path);
    }

    // `packages` is kept from `priority` alone: dependencies are mounted into
    // the main tree, and siblings never carry a table (ADR 29.9.26g §2.3).

    // Note: item_index_to_file is not merged here - it's set directly from build_combined_ast
    // The index→file mapping is specific to the combined AST being elaborated

    priority
}

/// Whether a fallback module's source `file` is one main's tree already owns.
/// A module with no recorded file is not main's.
fn owned_by_main(file: Option<&PathBuf>, main_files: &HashSet<PathBuf>) -> bool {
    file.is_some_and(|file| main_files.contains(file))
}

/// Public wrapper for get_module_name (for verbose logging in driver).
pub fn get_module_name_from_parsed(module: &ParsedModule) -> String {
    get_module_name(module)
}
