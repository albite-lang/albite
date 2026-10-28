//! Collision-scoped Core names (ADR 4.10.26j D1).
//!
//! A top-level value's identity is **(module, name)**. Its Core name — the
//! string `CoreDef.name`, `Term::Global`, the evaluator, the kernel and
//! codegen all key on — is the bare name when exactly one module defines
//! it, and a **scoped** name built from the module path when two or more
//! do. A program with no collision therefore elaborates byte-identically.
//!
//! A name a program defines once is scoped too when it is a **reserved link
//! name** (ADR 9.10.26f D1) — a C symbol the runtime archive or the native
//! core imports, so that a program's `fn write` cannot capture the runtime's
//! `write` calls at link time. An `extern "C"` declaration of such a name is
//! left bare: it calls the C symbol on purpose.
//!
//! The table is a pure function of the set of (canonical module, value
//! name) pairs, so module order, file order and hash order cannot change it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use crate::elaborate::env::{canonicalize_module_path, ModuleContents, ModulePath};

/// The scoped spelling of `bare` in `module`: the length-prefixed form
/// codegen's `mangle_symbol` produces (`x::helper` → `_tg_1x_6helper`, a
/// root-module `helper` → `_tg_6helper`). It is an ordinary C identifier —
/// no `::`, which codegen's `<unit>::<def>` keys split on, and no `$`, which
/// its `$direct` suffixes append.
pub fn scoped_spelling(module: &ModulePath, bare: &str) -> String {
    let mut spelling = String::from("_tg_");
    for segment in &module.segments {
        spelling.push_str(&segment.len().to_string());
        spelling.push_str(segment);
        spelling.push('_');
    }
    spelling.push_str(&bare.len().to_string());
    spelling.push_str(bare);
    spelling
}

/// A user-written bare name that equals a scoped Core name (D1's refusal):
/// the two would be one Core name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeltScopedName {
    /// The clashing spelling.
    pub spelling: String,
    /// The module and bare name the scoped spelling belongs to.
    pub scoped_from: (ModulePath, String),
}

/// `(module, bare) -> Core name`, with the collision set it was scoped by.
#[derive(Debug, Clone, Default)]
pub struct CoreNames {
    /// Bare name → the modules defining it, for names defined in two or more.
    collisions: BTreeMap<String, Vec<ModulePath>>,
    /// Reserved link name → the one module defining it with a body (ADR
    /// 9.10.26f D1). Disjoint from `collisions`, which keeps its meaning.
    reserved: BTreeMap<String, ModulePath>,
    /// Scoped Core name → the (module, bare name) it stands for.
    origins: HashMap<String, (ModulePath, String)>,
    /// Bare names a user wrote that equal some scoped Core name.
    spelt: BTreeMap<String, SpeltScopedName>,
}

impl CoreNames {
    /// Build the table from (module, bare value name) definitions, and the
    /// subset of them that are `extern "C"` declarations. Duplicate pairs
    /// count once; module paths should already be canonical.
    pub fn from_definitions(
        definitions: impl IntoIterator<Item = (ModulePath, String)>,
        externs: impl IntoIterator<Item = (ModulePath, String)>,
    ) -> Self {
        let mut definers: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
        for (module, bare) in definitions {
            definers.entry(bare).or_default().insert(module.segments);
        }
        let externs: HashSet<(Vec<String>, String)> = externs
            .into_iter()
            .map(|(module, bare)| (module.segments, bare))
            .collect();
        let mut table = Self::default();
        for (bare, modules) in &definers {
            if modules.len() < 2 {
                table.reserve_single_definer(bare, modules, &externs);
                continue;
            }
            let modules: Vec<ModulePath> = modules.iter().cloned().map(ModulePath::new).collect();
            for module in &modules {
                let spelling = scoped_spelling(module, bare);
                table
                    .origins
                    .insert(spelling, (module.clone(), bare.clone()));
            }
            table.collisions.insert(bare.clone(), modules);
        }
        for (spelling, origin) in &table.origins {
            if definers.contains_key(spelling) {
                table.spelt.insert(
                    spelling.clone(),
                    SpeltScopedName {
                        spelling: spelling.clone(),
                        scoped_from: origin.clone(),
                    },
                );
            }
        }
        table
    }

    /// Scope `bare`'s one definer when the name is a reserved link name and
    /// the definition is not an `extern "C"` declaration (ADR 9.10.26f D1).
    fn reserve_single_definer(
        &mut self,
        bare: &str,
        modules: &BTreeSet<Vec<String>>,
        externs: &HashSet<(Vec<String>, String)>,
    ) {
        let Some(segments) = modules.first() else {
            return;
        };
        if !albite_core::link_names::is_reserved_link_name(bare)
            || externs.contains(&(segments.clone(), bare.to_string()))
        {
            return;
        }
        let module = ModulePath::new(segments.clone());
        self.origins.insert(
            scoped_spelling(&module, bare),
            (module.clone(), bare.to_string()),
        );
        self.reserved.insert(bare.to_string(), module);
    }

    /// Build the table from the driver's module registry: every module's
    /// own value names — not the names a `pub use` copied in, which carry a
    /// `reexported_value_sources` entry — under the module's canonical path,
    /// so one file registered under two prefixes is one definer. Modules an
    /// undeclared sibling only lends names to are never elaborated, so they
    /// define nothing.
    #[allow(clippy::implicit_hasher)] // Reason: callers all use the default hasher
    pub fn from_module_registry(
        modules: &HashMap<ModulePath, ModuleContents>,
        module_files: &HashMap<ModulePath, PathBuf>,
        file_to_module: &HashMap<PathBuf, ModulePath>,
        names_only: &HashSet<ModulePath>,
    ) -> Self {
        let defining = || {
            modules
                .iter()
                .filter(|(path, _)| !names_only.contains(*path))
                .map(|(path, contents)| {
                    let canonical = canonicalize_module_path(path, module_files, file_to_module);
                    (canonical, contents)
                })
        };
        let definitions = defining().flat_map(|(canonical, contents)| {
            contents
                .values
                .iter()
                .filter(|name| !contents.reexported_value_sources.contains_key(*name))
                .map(move |name| (canonical.clone(), name.clone()))
        });
        let externs = defining().flat_map(|(canonical, contents)| {
            contents
                .extern_values
                .iter()
                .map(move |name| (canonical.clone(), name.clone()))
        });
        Self::from_definitions(definitions, externs)
    }

    /// Whether `bare` is defined in two or more modules.
    pub fn is_colliding(&self, bare: &str) -> bool {
        self.collisions.contains_key(bare)
    }

    /// The modules defining a colliding `bare`, in path order; empty when
    /// it does not collide.
    pub fn definers(&self, bare: &str) -> &[ModulePath] {
        self.collisions.get(bare).map_or(&[], Vec::as_slice)
    }

    /// The modules where `bare` has a scoped Core name: its definers when it
    /// collides, a reserved name's one definer, else none.
    pub fn scoped_definers(&self, bare: &str) -> &[ModulePath] {
        match self.reserved.get(bare) {
            Some(module) => std::slice::from_ref(module),
            None => self.definers(bare),
        }
    }

    /// Whether `bare` has a scoped Core name in some module: it collides,
    /// or it is a reserved link name one module defines (ADR 9.10.26f D1).
    pub fn is_scoped(&self, bare: &str) -> bool {
        self.is_colliding(bare) || self.reserved.contains_key(bare)
    }

    /// The Core name of `bare` as defined in `module` (canonical): scoped
    /// when `bare` collides and `module` is one of its definers, or when
    /// `module` is a reserved name's one definer; else bare.
    pub fn core_name(&self, module: &ModulePath, bare: &str) -> String {
        if self.definers(bare).contains(module) || self.reserved.get(bare) == Some(module) {
            scoped_spelling(module, bare)
        } else {
            bare.to_string()
        }
    }

    /// The (module, bare name) a scoped Core name stands for; `None` for a
    /// Core name that is its own bare name.
    pub fn origin(&self, core_name: &str) -> Option<&(ModulePath, String)> {
        self.origins.get(core_name)
    }

    /// The source spelling of a Core name.
    pub fn source_name<'a>(&'a self, core_name: &'a str) -> &'a str {
        self.origin(core_name)
            .map_or(core_name, |(_, bare)| bare.as_str())
    }

    /// A user-written `bare` that equals a scoped Core name (D1's refusal).
    pub fn spelt_scoped_name(&self, bare: &str) -> Option<&SpeltScopedName> {
        self.spelt.get(bare)
    }

    /// The sorted bare names that were scoped — the collision set the cache
    /// keys fold in (ADR 4.10.26j D7). Empty for a program with no collision.
    pub fn collision_set(&self) -> impl Iterator<Item = &str> {
        self.collisions.keys().map(String::as_str)
    }

    /// Every bare name that was scoped, for the cache keys to fold (ADR
    /// 9.10.26f D1): the collision set, then each reserved single definer
    /// tagged `reserved:`, so a reserved `write` and a colliding `write` key
    /// apart. Empty — and every key unchanged — for a program that scopes
    /// nothing.
    pub fn scoped_set(&self) -> Vec<String> {
        self.collision_set()
            .map(str::to_string)
            .chain(self.reserved.keys().map(|bare| format!("reserved:{bare}")))
            .collect()
    }
}

// Tests: core_names_tests.rs — collision and reserved-name scoping (ADR 4.10.26j AC1, 9.10.26f AC3)
#[cfg(test)]
#[path = "core_names_tests.rs"]
mod tests;
