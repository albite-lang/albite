//! Module resolution and tree building.
//!
//! Resolves `mod foo;` declarations to files and builds a module tree.
//!
//! ## Workspace-Aware Resolution
//!
//! When checking a single file that's part of a larger project, we need to know
//! about sibling modules for cross-module imports. For example, when checking
//! `elab/env/mod.tg`, it may `use lexer::span::Span` - we need the `lexer` module
//! to be registered even though it's not a descendant of `elab`.
//!
//! The workspace discovery system:
//! 1. Finds the "workspace root" - the directory containing the file being checked
//! 2. Discovers all sibling modules (directories with `mod.tg` or `*.tg` files)
//! 3. Parses all sibling module trees
//! 4. Merges them into a combined `ModuleInfo` for resolution

pub(crate) mod companion;
pub(crate) mod definitions;
mod info;
mod info_reexports;
pub(crate) mod parse;
mod workspace;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{SourceFile, Visibility};
use crate::elaborate::ElabError;
use crate::span::Span;

// Re-export public items
pub use info::{build_module_info, ModuleInfo};
pub use info_reexports::resolve_pub_use_module;
pub use parse::{
    build_source_map, collect_parse_errors, extract_module_dependencies, flatten_module_tree,
    parse_module_tree, ModuleDependencyInfo,
};
pub use workspace::{
    build_workspace_module_info, discover_sibling_modules, find_workspace_root,
    get_module_name_from_parsed, merge_module_info, parse_workspace_modules,
};

/// Source map for multi-file error reporting.
/// Maps file paths to their source content.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// File path → source content
    pub(crate) sources: HashMap<PathBuf, String>,
    /// Main file path (for fallback)
    pub(crate) main_file: Option<PathBuf>,
    /// Modules parsed with a test companion appended (ADR 2.10.26j D4). Each
    /// file's own text is under its own path in `sources`; this says where
    /// the sibling's combined spans cross into the companion.
    pub(crate) companions: Vec<CompanionSplit>,
}

/// Whether `text` lexes a `region` keyword. `region` is reserved, so in a
/// program that parsed the token can only open a region (ADR 9.10.26o D2).
#[must_use]
pub fn source_uses_regions(text: &str) -> bool {
    text.contains("region")
        && crate::lexer::Lexer::new(text)
            .tokenize_all()
            .0
            .iter()
            .any(|token| token.kind == crate::token::TokenKind::Region)
}

/// Where a module's combined parse crosses from its own text into its
/// companion's (ADR 2.10.26j D3/D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionSplit {
    /// The module's file, which every span of the combined parse names.
    pub sibling: PathBuf,
    /// The companion's first byte in the combined source.
    pub base: u32,
    /// The companion's file.
    pub companion: PathBuf,
}

impl SourceMap {
    /// Whether any module's text holds a `region` (ADR 9.10.26o D7) — the
    /// program-level marker that selects the scoped arena mode. Read from the
    /// sources, which every module has on a warm run too, where a module the
    /// elaboration cache served contributes no Core to search.
    #[must_use]
    pub fn uses_regions(&self) -> bool {
        self.sources.values().any(|text| source_uses_regions(text))
    }

    /// Create a new empty source map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a source map with a single main file.
    pub fn single(path: PathBuf, source: String) -> Self {
        let mut sources = HashMap::new();
        sources.insert(path.clone(), source);
        Self {
            sources,
            main_file: Some(path),
            companions: Vec::new(),
        }
    }

    /// Map a span of a combined parse to the file it is really in
    /// (ADR 2.10.26j D4): a span on a sibling at or past its `base` moves to
    /// the companion, less `base`; every other span is returned unchanged.
    #[must_use]
    pub fn relocate(&self, file: &Path, span: Span) -> (PathBuf, Span) {
        match self
            .companions
            .iter()
            .find(|split| split.sibling == file && span.start >= split.base)
        {
            Some(split) => (
                split.companion.clone(),
                Span::new(span.start - split.base, span.end.saturating_sub(split.base)),
            ),
            None => (file.to_path_buf(), span),
        }
    }

    /// Add a source file to the map.
    pub fn insert(&mut self, path: PathBuf, source: String) {
        if self.main_file.is_none() {
            self.main_file = Some(path.clone());
        }
        self.sources.insert(path, source);
    }

    /// Get the source for a file path.
    pub fn get(&self, path: &Path) -> Option<&str> {
        self.sources.get(path).map(|s| s.as_str())
    }

    /// Get the main file's source (for fallback).
    pub fn main_source(&self) -> Option<&str> {
        self.main_file
            .as_ref()
            .and_then(|p| self.sources.get(p).map(|s| s.as_str()))
    }

    /// Get the main file path.
    pub fn main_file(&self) -> Option<&Path> {
        self.main_file.as_deref()
    }

    /// Check if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// Rewrite every location an error carries through [`SourceMap::relocate`]
/// (ADR 2.10.26j D4): its own span and file, each note, the expected-type
/// context and every trace frame. A location with no file of its own is read
/// against the error's original file, and keeps no file of its own when it
/// relocates to the same file the primary span did.
pub fn relocate_error(error: &mut ElabError, source_map: &SourceMap) {
    if source_map.companions.is_empty() {
        return;
    }
    let Some(original) = error
        .file_path
        .clone()
        .or_else(|| source_map.main_file.clone())
    else {
        return;
    };
    let (primary_file, primary_span) = source_map.relocate(&original, error.span);
    error.span = primary_span;
    error.file_path = Some(primary_file.clone());
    let place = |own: &Option<PathBuf>, span: Span| {
        let (file, span) = source_map.relocate(own.as_deref().unwrap_or(&original), span);
        let keep_own = own.is_some() || file != primary_file;
        (keep_own.then_some(file), span)
    };
    for note in &mut error.notes {
        if let Some(span) = note.span {
            (note.file_path, note.span) = {
                let (file, span) = place(&note.file_path, span);
                (file, Some(span))
            };
        }
    }
    if let Some(context) = &mut error.context {
        (context.file_path, context.span) = place(&context.file_path, context.span);
    }
    for frame in &mut error.trace {
        (frame.file_path, frame.span) = source_map.relocate(&frame.file_path, frame.span);
    }
}

/// A parsed module with its submodules.
#[derive(Debug)]
pub struct ParsedModule {
    /// Path to this module's source file
    pub path: PathBuf,
    /// Visibility of this module (Public or Private)
    pub visibility: Visibility,
    /// The parsed source file
    pub source_file: SourceFile,
    /// Submodules declared in this file
    pub submodules: Vec<ParsedModule>,
    /// The dependency key a mounted library's `lib.tg` is named by, which
    /// [`info::get_module_name`] returns before the file stem (ADR 29.9.26g D4).
    /// `None` for every module the source tree itself declares.
    pub mount_name: Option<String>,
}

impl ParsedModule {
    /// How many modules this tree contains, counting itself.
    ///
    /// The denominator for "how much of the project did the compiler actually
    /// look at" (ADR 7.8.26d retrospective): body elaboration stops at the
    /// first module that fails, so a diagnostic run can silently cover a
    /// fraction of the tree.
    #[must_use]
    pub fn module_count(&self) -> usize {
        1 + self
            .submodules
            .iter()
            .map(ParsedModule::module_count)
            .sum::<usize>()
    }
}
