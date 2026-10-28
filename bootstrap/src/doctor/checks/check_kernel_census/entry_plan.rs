//! Which files the corpus census elaborates (ADR 2.10.26o D2) and, under
//! `--proof-bearing`, which of those it may skip (ADR 3.10.26s D2, D5).
//!
//! - **The walk**: every `.tg` under the roots, hidden and build directories
//!   out, no symlink followed.
//! - **The plan**: companions out, and every non-root node of some
//!   candidate's `mod` tree out; each remaining entry keeps its tree.
//! - **The selection**: decided from a parse of each tree file, once per run
//!   — a theorem, lemma, axiom or proof-carrying `fn` (ADR 14.10.26b) — by
//!   coverage — an entry is kept only when it reaches a proof-bearing file
//!   no entry kept before it reaches, and kept whenever that cannot be
//!   established.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::{CACHE_DIR, LEGACY_CACHE_DIR};
use crate::driver::{self, companion_index};
use crate::parser::Parser;

mod proof_types;

use proof_types::proof_carrying_types;
pub use proof_types::ProofFacts;

/// Directories the walk never enters, beside every hidden one — the set
/// `albite cache clean` skips (D2).
const SKIPPED_DIRS: [&str; 3] = ["target", CACHE_DIR, LEGACY_CACHE_DIR];

/// The entry set and what the module filter learned on the way (D2).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EntryPlan {
    pub entries: Vec<PathBuf>,
    pub companions: Vec<PathBuf>,
    /// Each entry's `mod`-declared tree, itself included.
    pub trees: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Entries whose tree could not be built — an unresolved `mod`, a cycle,
    /// an unreadable file — so their `trees` row is the entry alone
    /// (3.10.26s D2: never mistaken for a proof-free tree).
    pub unbuilt: BTreeSet<PathBuf>,
}

/// D2's filter over canonical candidates in path order: companions out, and
/// every non-root node of some candidate's `mod` tree out — except a
/// `main.tg`, which is the root of a tree of its own.
#[must_use]
pub fn plan_entries(candidates: &[PathBuf]) -> EntryPlan {
    let companions: BTreeSet<PathBuf> = companion_index(candidates).into_values().collect();
    let mut trees = BTreeMap::new();
    let mut unbuilt = BTreeSet::new();
    let mut modules = BTreeSet::new();
    for candidate in candidates.iter().filter(|c| !companions.contains(*c)) {
        let tree = mod_tree_files(candidate).unwrap_or_else(|| {
            unbuilt.insert(candidate.clone());
            vec![candidate.clone()]
        });
        modules.extend(tree.iter().skip(1).cloned());
        trees.insert(candidate.clone(), tree);
    }
    let entries = candidates
        .iter()
        .filter(|c| !companions.contains(*c))
        .filter(|c| !modules.contains(*c) || c.file_name().is_some_and(|n| n == "main.tg"))
        .cloned()
        .collect::<Vec<_>>();
    trees.retain(|path, _| entries.contains(path));
    unbuilt.retain(|path| entries.contains(path));
    EntryPlan {
        entries,
        companions: companions.into_iter().collect(),
        trees,
        unbuilt,
    }
}

/// `entry`'s `mod`-declared tree, root first, canonical; `None` when the tree
/// cannot be built (the entry then reads *does not elaborate*).
fn mod_tree_files(entry: &Path) -> Option<Vec<PathBuf>> {
    let parsed = driver::parse_module_tree(entry, &mut HashSet::new(), &mut Vec::new(), None);
    let tree = parsed.ok()?;
    let mut files = Vec::new();
    let mut pending = vec![&tree];
    while let Some(module) = pending.pop() {
        files.push(fs::canonicalize(&module.path).unwrap_or_else(|_| module.path.clone()));
        pending.extend(module.submodules.iter().rev());
    }
    Some(files)
}

/// What one parse said of a file (3.10.26s D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileParse {
    /// Parses, and declares a theorem, lemma, axiom or proof-carrying `fn`
    /// (ADR 14.10.26b D1).
    ProofBearing,
    /// Parses, and declares none.
    ProofFree,
    /// Unreadable, or a parse error: its entry is kept.
    Failed,
}

/// Parse `file` for what it declares; `None` when it is unreadable or does
/// not parse.
#[must_use]
pub fn parse_for_proofs(file: &Path) -> Option<ProofFacts> {
    let source = fs::read_to_string(file).ok()?;
    let (ast, errors) = Parser::new(&source).parse();
    errors.is_empty().then(|| ProofFacts::of_items(&ast.items))
}

/// Every file of every entry's tree, parsed once (3.10.26s D5): entries share
/// most of their trees, so the parses are keyed by path, failures included.
/// A file is then judged against each tree that reaches it, with that tree's
/// proof-carrying types (14.10.26b D2), and is proof-bearing when any tree
/// says so.
#[must_use]
pub fn parse_tree_files(
    plan: &EntryPlan,
    parse: impl Fn(&Path) -> Option<ProofFacts>,
) -> BTreeMap<PathBuf, FileParse> {
    let mut facts = BTreeMap::new();
    for file in plan.trees.values().flatten() {
        if !facts.contains_key(file) {
            facts.insert(file.clone(), parse(file));
        }
    }
    let mut parses = BTreeMap::new();
    for tree in plan.trees.values() {
        let tree_facts = tree.iter().filter_map(|file| facts.get(file)?.as_ref());
        let carrying = proof_carrying_types(tree_facts);
        for file in tree {
            let verdict = match facts.get(file).and_then(Option::as_ref) {
                None => FileParse::Failed,
                Some(found) if found.bears_proof(&carrying) => FileParse::ProofBearing,
                Some(_) => FileParse::ProofFree,
            };
            let slot = parses.entry(file.clone()).or_insert(verdict);
            if verdict == FileParse::ProofBearing {
                *slot = verdict;
            }
        }
    }
    parses
}

/// The entries to elaborate, in the plan's order (3.10.26s D2). An entry is
/// kept when its tree reaches a proof-bearing file no entry kept so far
/// reaches, and — failing open — when its tree was not built or holds a file
/// that did not parse or was never parsed.
#[must_use]
pub fn select_proof_entries(
    plan: &EntryPlan,
    parses: &BTreeMap<PathBuf, FileParse>,
) -> BTreeSet<PathBuf> {
    let mut covered = BTreeSet::new();
    let mut kept = BTreeSet::new();
    for entry in &plan.entries {
        let tree = plan
            .trees
            .get(entry)
            .filter(|_| !plan.unbuilt.contains(entry));
        let Some(tree) = tree else {
            kept.insert(entry.clone());
            continue;
        };
        let reached = |file: &PathBuf| parses.get(file).copied().unwrap_or(FileParse::Failed);
        let keep = tree.iter().any(|file| match reached(file) {
            FileParse::ProofBearing => !covered.contains(file),
            FileParse::ProofFree => false,
            FileParse::Failed => true,
        });
        if keep {
            kept.insert(entry.clone());
            covered.extend(
                tree.iter()
                    .filter(|file| reached(file) == FileParse::ProofBearing)
                    .cloned(),
            );
        }
    }
    kept
}

/// Every `.tg` under `roots`, canonical, sorted and deduplicated (D2).
///
/// # Errors
///
/// A root that is missing or unreadable, or a directory the walk cannot read.
pub fn walk_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut found = BTreeSet::new();
    for root in roots {
        let canonical = fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
        if !canonical.is_dir() {
            return Err(format!("{} is not a directory", root.display()));
        }
        walk_dir(&canonical, &mut found)?;
    }
    Ok(found.into_iter().collect())
}

/// One directory: no symlink is followed, file or directory, so a link cycle
/// cannot loop the walk.
fn walk_dir(dir: &Path, found: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() && !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
            walk_dir(&path, found)?;
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "tg") {
            found.insert(path);
        }
    }
    Ok(())
}
