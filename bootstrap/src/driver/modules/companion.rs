//! Test companion files (ADR 2.10.26j, roadmap T10).
//!
//! `<dir>/<stem>_test.tg` beside `<dir>/<stem>.tg` is that module's
//! *companion*: under `albite test`, and only there, the module is parsed as
//! its file followed by the companion's, so the companion's items are the
//! module's items — same path, same scope, private helpers and `use` imports
//! included. Every other command ignores the file.
//!
//! The decisions are pure functions over injected data ([`companion_index`],
//! [`policy_for`], [`combined_source`], [`companion_collisions`],
//! [`cache_eligible`], [`companions_in_tree`]); [`attach_companions`] is the
//! one effectful step, and it only reads the two files and applies them.
//!
//! Tests: `driver/modules/tests/companion.rs`.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::Item;
use crate::driver::{Mode, PipelineError};
use crate::elaborate::ElabError;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::span::Span;

use super::definitions::{duplicate_definitions, Duplicate};
use super::{CompanionSplit, ParsedModule, SourceMap};

/// The suffix that makes a file a companion candidate.
const COMPANION_SUFFIX: &str = "_test.tg";

/// Whether a file stem names an entry file the `albite test` runner drives.
///
/// A free predicate rather than a closure inside discovery, so the convention
/// is assertable without a filesystem: `test_*` is the runner's own discovery
/// prefix, and `mustfail_*` is the must-fail twin convention (ADR 29.6.26f),
/// which is an entry file the sweep deliberately skips and which still imports
/// real modules. An entry already holds tests, so it gets no companion
/// (ADR 2.10.26j D1).
#[must_use]
pub fn is_test_entry_stem(stem: &str) -> bool {
    // `test_`, not `test`: `tester.tg` would be an ordinary module.
    stem.starts_with("test_") || stem.starts_with("mustfail_")
}

/// Pair every companion in `files` with its sibling: sibling → companion
/// (ADR 2.10.26j D1).
///
/// `<dir>/<stem>_test.tg` pairs with `<dir>/<stem>.tg` when the sibling is in
/// the same list, `<stem>` is non-empty and holds no `.`, and `<stem>` is not
/// a test-entry stem. Paths are compared exactly as given, so the caller picks
/// one spelling for both sides — the driver passes canonical paths.
#[must_use]
pub fn companion_index(files: &[PathBuf]) -> BTreeMap<PathBuf, PathBuf> {
    let present: HashSet<&Path> = files.iter().map(PathBuf::as_path).collect();
    files
        .iter()
        .filter_map(|companion| {
            let name = companion.file_name()?.to_str()?;
            let stem = name.strip_suffix(COMPANION_SUFFIX)?;
            if stem.is_empty() || stem.contains('.') || is_test_entry_stem(stem) {
                return None;
            }
            let sibling = companion.with_file_name(format!("{stem}.tg"));
            present
                .contains(sibling.as_path())
                .then(|| (sibling, companion.clone()))
        })
        .collect()
}

/// Whether a run attaches companions (ADR 2.10.26j D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionPolicy {
    /// Join each paired module's companion to it — `albite test` only.
    Attach,
    /// Leave every companion out — `check`, `run`, `compile` and the tools.
    Ignore,
}

/// Only `test` attaches (ADR 2.10.26j D2).
#[must_use]
pub fn policy_for(mode: Mode) -> CompanionPolicy {
    match mode {
        Mode::Test => CompanionPolicy::Attach,
        Mode::Check | Mode::Run => CompanionPolicy::Ignore,
    }
}

/// The one source a paired module is parsed from, and `base`, the byte offset
/// at which the companion's text starts in it (ADR 2.10.26j D3).
///
/// The newline keeps the sibling's last line, a trailing `//` comment
/// included, from running into the companion's first.
#[must_use]
pub fn combined_source(sibling: &str, companion: &str) -> (String, u32) {
    let base = u32::try_from(sibling.len() + 1).unwrap_or(u32::MAX);
    (format!("{sibling}\n{companion}"), base)
}

/// A top-level name defined both in a sibling and in its companion
/// (ADR 2.10.26j D5). Spans are into the combined source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The doubly defined name.
    pub name: String,
    /// The companion's definition: where E0004 is reported.
    pub companion_span: Span,
    /// The sibling's definition: where the note points.
    pub sibling_span: Span,
    /// For a constructor, the types declaring it: (companion's, sibling's).
    pub owners: Option<(String, String)>,
}

/// The names defined on both sides of `base` in one combined item list
/// (ADR 2.10.26j D5), one per companion-side definition, in source order:
/// the straddling pairs of [`duplicate_definitions`] (ADR 13.10.26f D1).
/// A pair on one side is a within-file duplicate, reported by the driver.
#[must_use]
pub fn companion_collisions(items: &[Item], base: u32) -> Vec<Collision> {
    duplicate_definitions(items)
        .iter()
        .filter_map(|duplicate| straddling_collision(duplicate, base))
        .collect()
}

/// The companion form of a duplicate that straddles `base`, else `None`.
pub(crate) fn straddling_collision(duplicate: &Duplicate, base: u32) -> Option<Collision> {
    duplicate.straddles(Some(base)).then(|| Collision {
        name: duplicate.name.clone(),
        companion_span: duplicate.second,
        sibling_span: duplicate.first,
        owners: duplicate.owners.clone(),
    })
}

/// Whether a module may be read from or written to the elaboration cache
/// (ADR 2.10.26j D6): not when a companion is attached to it, because every
/// cache key hashes the sibling's bytes alone.
#[must_use]
pub fn cache_eligible(path: &Path, source_map: &SourceMap) -> bool {
    !source_map
        .companions
        .iter()
        .any(|split| split.sibling == path)
}

/// The companions of the modules in a tree (ADR 2.10.26j D8): `module_paths`
/// and `index` in the same spelling, the result in the tree's order.
#[must_use]
pub fn companions_in_tree(
    module_paths: &[PathBuf],
    index: &BTreeMap<PathBuf, PathBuf>,
) -> Vec<PathBuf> {
    module_paths
        .iter()
        .filter_map(|path| index.get(path).cloned())
        .collect()
}

/// The companions of every module in `entry`'s tree, for the tier guard
/// (ADR 2.10.26j D8), or `None` when the tree does not build — the guard then
/// scans the entry alone and the pipeline reports the error in its usual form.
/// A second, uncached parse of the tree, before any elaboration.
pub fn tree_companions(entry: &Path) -> Option<Vec<PathBuf>> {
    let tree = super::parse_module_tree(entry, &mut HashSet::new(), &mut Vec::new(), None).ok()?;
    let canonical = |paths: Vec<PathBuf>| -> Vec<PathBuf> {
        paths.iter().filter_map(|p| p.canonicalize().ok()).collect()
    };
    let files = super::parse::discover_tg_files(super::parse::entry_dir(entry));
    let index = companion_index(&canonical(files));
    Some(companions_in_tree(&canonical(tree_paths(&tree)), &index))
}

/// Every module path in a tree, depth first.
pub(crate) fn tree_paths(tree: &ParsedModule) -> Vec<PathBuf> {
    let mut paths = vec![tree.path.clone()];
    for submodule in &tree.submodules {
        paths.extend(tree_paths(submodule));
    }
    paths
}

/// A module the run will attach a companion to, both paths spelled from the
/// tree (so they meet the source map's keys and the errors' `file_path`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompanionPair {
    pub(crate) sibling: PathBuf,
    pub(crate) companion: PathBuf,
}

/// The pairs to attach in `tree` (ADR 2.10.26j D3). `index` is keyed by
/// canonical path (P0 item 4: the tree spells a bare entry `main.tg` where
/// discovery spells it `./main.tg`). A companion the tree itself declares as
/// a module is skipped: the `mod` declaration wins.
pub(crate) fn pairs_in_tree(
    tree: &ParsedModule,
    index: &BTreeMap<PathBuf, PathBuf>,
) -> Vec<CompanionPair> {
    let spelled = tree_paths(tree);
    let canonical: Vec<Option<PathBuf>> = spelled.iter().map(|p| p.canonicalize().ok()).collect();
    let declared: HashSet<&PathBuf> = canonical.iter().flatten().collect();
    spelled
        .iter()
        .zip(&canonical)
        .filter_map(|(sibling, canonical)| {
            let companion = index.get(canonical.as_ref()?)?;
            if declared.contains(companion) {
                return None;
            }
            Some(CompanionPair {
                sibling: sibling.clone(),
                companion: sibling.with_file_name(companion.file_name()?),
            })
        })
        .collect()
}

/// A companion that failed to parse on its own: its path, text and errors,
/// for rendering in its own lines (ADR 2.10.26j D3).
pub(crate) type CompanionParseFailure = (PathBuf, String, Vec<ParseError>);

/// Parse each companion alone, before anything is attached, and return the
/// ones that fail — so a parse error names the companion's file and line.
pub(crate) fn companion_parse_failures(pairs: &[CompanionPair]) -> Vec<CompanionParseFailure> {
    pairs
        .iter()
        .filter_map(|pair| {
            let text = fs::read_to_string(&pair.companion).ok()?;
            let (_, errors) = Parser::new(&text).parse();
            (!errors.is_empty()).then(|| (pair.companion.clone(), text, errors))
        })
        .collect()
}

/// Attach every pair (ADR 2.10.26j D3): re-parse the sibling as its text
/// followed by the companion's and record the split in `source_map`. The
/// sibling-vs-companion collisions (D5) are raised afterwards, with every
/// other duplicate, by `definitions::duplicate_errors` (ADR 13.10.26f D2).
pub(crate) fn attach_companions(
    tree: &mut ParsedModule,
    pairs: &[CompanionPair],
    source_map: &mut SourceMap,
) -> Result<(), PipelineError> {
    for pair in pairs {
        let read = |path: &Path| {
            fs::read_to_string(path)
                .map_err(|e| PipelineError::IoError(path.display().to_string(), e.to_string()))
        };
        let (sibling_text, companion_text) = (read(&pair.sibling)?, read(&pair.companion)?);
        let (combined, base) = combined_source(&sibling_text, &companion_text);
        let (source_file, _) = Parser::new(&combined).parse();
        if let Some(name) = declared_module(&source_file.items, base) {
            return Err(PipelineError::CompanionDeclaresModule {
                companion: pair.companion.clone(),
                name,
            });
        }
        if let Some(module) = find_module_mut(tree, &pair.sibling) {
            module.source_file = source_file;
        }
        source_map.insert(pair.companion.clone(), companion_text);
        source_map.companions.push(CompanionSplit {
            sibling: pair.sibling.clone(),
            base,
            companion: pair.companion.clone(),
        });
    }
    Ok(())
}

/// The first `mod` a companion declares — refused, because the tree is
/// already built and the submodule would never be resolved.
fn declared_module(items: &[Item], base: u32) -> Option<String> {
    items.iter().find_map(|item| match item {
        Item::Mod(decl) if decl.span.start >= base => Some(decl.name.name.clone()),
        _ => None,
    })
}

/// E0004 at the companion's definition with a note at the sibling's, both
/// against the sibling's combined spans; `relocate_error` moves the primary.
pub(crate) fn collision_error(collision: &Collision, sibling: &Path) -> ElabError {
    let first = match &collision.owners {
        Some((companion_owner, sibling_owner)) => format!(
            "`{}` is first defined here, as a constructor of `{sibling_owner}`; \
             the companion redefines it as a constructor of `{companion_owner}`",
            collision.name
        ),
        None => format!("`{}` is first defined here", collision.name),
    };
    ElabError::duplicate(collision.companion_span, collision.name.clone())
        .with_file_path(sibling)
        .with_cross_file_note(collision.sibling_span, sibling.to_path_buf(), first)
        .with_help(
            "a test companion shares its module's scope, so it cannot redefine \
             a name the module defines; rename one of them",
        )
}

fn find_module_mut<'a>(module: &'a mut ParsedModule, path: &Path) -> Option<&'a mut ParsedModule> {
    if module.path == path {
        return Some(module);
    }
    module
        .submodules
        .iter_mut()
        .find_map(|submodule| find_module_mut(submodule, path))
}
