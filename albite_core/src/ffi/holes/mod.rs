//! The self-hosted `check` line's holes seam (ADR 16.10.26g D2).
//!
//! The bootstrap's `check` counts a program's holes (`contains sorry (A
//! authored, S synthesised)`, ADR 18.9.26g) and the theorems that rest on an
//! axiom or sorry (ADR 13.10.26g). Both analyses already live in this crate —
//! `sorry_sites`, `RootClass` and the `rests_on_refs` closure — so the
//! self-host writes no second classifier: it streams each definition's name,
//! item kind and term handle here, and reads back the three numbers the
//! bootstrap's line prints.
//!
//! ## Why the stream reduces
//!
//! Like the termination mirror (`ffi/termination/`), each term is materialised
//! once and dropped before the call returns: holding every body measured
//! +587 MiB on `src/compiler/main.tg`. What is kept per definition is its hole
//! classes, whether it is a root and its `global_names` set.
//!
//! ## Why the item kind classifies an axiom
//!
//! A self-host term carries no span — `tg_term_sorry` builds a bare `Sorry` —
//! so `sorry_sites` would read an axiom's hole as `Unclassified`. The item kind
//! says who wrote it instead: an axiom's hole is authored and the axiom is a
//! root of class `Axiom`, which is what the bootstrap's spanned body and
//! `is_axiom` give it. Every other definition is classified by its term,
//! unchanged.
//!
//! Every failure path stores a message reachable through `tg_get_last_error`
//! and returns `false`: a dropped definition is a hole nobody counted.

#![allow(unsafe_code)]

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::ffi::{c_char, CStr};

use crate::conversion::delta::global_names;
use crate::terms::analysis::{rests_on_refs, RootClass, SorryCounts, SorrySite};
use crate::terms::Term;

use super::terms::nodes::materialize_term;
use super::{with_arena_ref, TermHandle};

/// What a definition's source item was (D2's `kind` codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HoleItemKind {
    /// A `fn`, `let` or anything else — and any name never noted.
    #[default]
    Program,
    /// A `theorem` or `lemma`: the population the resting count reads.
    Proof,
    /// An `axiom`: its hole is authored and it is a root.
    Axiom,
}

impl HoleItemKind {
    fn from_code(code: u64) -> Option<HoleItemKind> {
        match code {
            0 => Some(HoleItemKind::Program),
            1 => Some(HoleItemKind::Proof),
            2 => Some(HoleItemKind::Axiom),
            _ => None,
        }
    }
}

/// One program's reduced hole data.
///
/// Hole counts accumulate per `add_def` call, as the bootstrap's
/// `SorryCounts::of` sums over every definition; kind, root class and
/// references are keyed by name, last write wins, as its maps are (D2).
#[derive(Debug, Default)]
struct HolesRegistry {
    kinds: HashMap<String, HoleItemKind>,
    counts: SorryCounts,
    roots: HashMap<String, RootClass>,
    references: HashMap<String, BTreeSet<String>>,
    /// Each added name once, in first-added order, so the closure's input is
    /// deterministic.
    order: Vec<String>,
}

impl HolesRegistry {
    fn note_item(&mut self, name: &str, kind: HoleItemKind) {
        self.kinds.insert(name.to_string(), kind);
    }

    fn kind_of(&self, name: &str) -> HoleItemKind {
        self.kinds.get(name).copied().unwrap_or_default()
    }

    fn add_def(&mut self, name: &str, term: &Term) {
        let sites = term.sorry_sites();
        let root = if self.kind_of(name) == HoleItemKind::Axiom {
            // A spanless axiom hole is authored by construction (module doc).
            self.counts.authored += sites.len();
            Some(RootClass::Axiom)
        } else {
            self.counts.add_sites(&sites);
            sites
                .iter()
                .any(|site| !matches!(site, SorrySite::Synthesised(_)))
                .then_some(RootClass::Sorry)
        };
        match root {
            Some(class) => self.roots.insert(name.to_string(), class),
            None => self.roots.remove(name),
        };
        if self
            .references
            .insert(name.to_string(), global_names(term))
            .is_none()
        {
            self.order.push(name.to_string());
        }
    }

    /// Theorems and lemmas reaching a root; zero when there is no root, which
    /// skips the closure as the bootstrap's `resting_theorems` does.
    fn resting(&self) -> usize {
        if self.roots.is_empty() {
            return 0;
        }
        let refs: Vec<(&str, &BTreeSet<String>)> = self
            .order
            .iter()
            .map(|name| (name.as_str(), &self.references[name]))
            .collect();
        rests_on_refs(&refs, |name| self.roots.contains_key(name))
            .keys()
            .filter(|name| self.kind_of(name) == HoleItemKind::Proof)
            .count()
    }
}

thread_local! {
    /// Thread-local like [`super::ARENA`]: the FFI is single-threaded by contract.
    static HOLES: RefCell<HolesRegistry> = RefCell::new(HolesRegistry::default());
}

fn with_holes<T>(body: impl FnOnce(&mut HolesRegistry) -> T) -> T {
    HOLES.with(|cell| body(&mut cell.borrow_mut()))
}

/// Record `message` where `tg_get_last_error` will find it, and fail.
fn fail(message: String) -> bool {
    super::with_arena!(|arena| arena.set_error(message));
    false
}

/// Read a C string argument. `None` on null or non-UTF-8.
///
/// # Safety
/// `ptr` must be null or a valid null-terminated C string.
unsafe fn read_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(str::to_string)
}

/// Open a program, discarding the last one.
#[no_mangle]
pub extern "C" fn tg_holes_reset() {
    with_holes(|registry| *registry = HolesRegistry::default());
}

/// Record one item's kind: `0` a program definition, `1` a `theorem` or
/// `lemma`, `2` an `axiom`. Last write wins.
///
/// # Safety
/// `name` must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tg_holes_note_item(name: *const c_char, kind: u64) -> bool {
    let Some(name) = read_cstr(name) else {
        return fail("holes note_item: unreadable name".to_string());
    };
    let Some(kind) = HoleItemKind::from_code(kind) else {
        return fail(format!(
            "holes note_item: unknown item kind {kind} for '{name}'"
        ));
    };
    with_holes(|registry| registry.note_item(&name, kind));
    true
}

/// Reduce one definition's term to its hole classes, root class and global
/// references; the term itself is dropped before this returns.
///
/// # Safety
/// `name` must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tg_holes_add_def(name: *const c_char, term: TermHandle) -> bool {
    let Some(name) = read_cstr(name) else {
        return fail("holes add_def: unreadable name".to_string());
    };
    let Some(term) = with_arena_ref!(|arena| materialize_term(arena, term)) else {
        return fail(format!("holes add_def: unreadable term for '{name}'"));
    };
    with_holes(|registry| registry.add_def(&name, &term));
    true
}

/// Holes the authors wrote, over every definition added.
#[no_mangle]
pub extern "C" fn tg_holes_authored() -> u64 {
    with_holes(|registry| registry.counts.authored as u64)
}

/// Holes nobody wrote — synthesised plus unclassified — over every definition.
#[no_mangle]
pub extern "C" fn tg_holes_not_authored() -> u64 {
    with_holes(|registry| registry.counts.not_authored() as u64)
}

/// How many theorems and lemmas rest on an axiom or sorry.
#[no_mangle]
pub extern "C" fn tg_holes_resting() -> u64 {
    with_holes(|registry| registry.resting() as u64)
}
