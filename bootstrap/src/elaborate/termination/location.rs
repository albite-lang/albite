//! Where a rejected definition lives (ADR 3.10.26r).
//!
//! The engine is pure over names, so a failure says *which* definition and,
//! sometimes, *which term* — never which file. The renderer attributes an
//! `ElabError` with no `file_path` to the entry file, so a rejection of a proof
//! in `b.tg` used to print under `main.tg` with `b.tg`'s byte offsets. This is
//! the name → (span, file) lookup the adapter renders against: fresh
//! definitions from the accumulator's per-module groups (D1), and cache-served
//! proofs from the sites their entry carried (D2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::elaborate::CoreDef;
use crate::span::Span;

use super::CachedTermination;

/// A proof-relevant definition's place in source, carried by a cache entry
/// because a cache-served module contributes no `CoreDef` to read it from
/// (ADR 3.10.26r D2).
///
/// The entry is content-addressed, so `file` is not known when it is written:
/// the walk that serves the hit supplies it ([`CachedTermination::locate_in`]),
/// the way it supplies a kernel obligation's module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofSite {
    /// The definition's name — the project-wide key the engine reports by.
    pub name: String,
    /// The whole definition's span, the fallback a cold run would use.
    pub span: Span,
    /// The file it was elaborated from, once a walk has served it.
    pub file: Option<PathBuf>,
}

/// Definition name → where it was defined.
#[derive(Debug, Default)]
pub struct DefinitionSites<'a> {
    by_name: BTreeMap<&'a str, (Span, Option<&'a Path>)>,
}

impl<'a> DefinitionSites<'a> {
    /// Spans only, for a definition set with no file attached (one module's
    /// own check, or a test).
    #[must_use]
    pub fn of_defs(defs: &'a [CoreDef]) -> Self {
        let mut sites = DefinitionSites::default();
        for def in defs {
            sites.by_name.insert(def.name.as_str(), (def.span, None));
        }
        sites
    }

    /// Record one module's definitions as defined in `file` (D1). Overrides a
    /// file-less entry for the same name.
    pub fn add_module(&mut self, file: &'a Path, defs: &'a [CoreDef]) {
        for def in defs {
            self.by_name
                .insert(def.name.as_str(), (def.span, Some(file)));
        }
    }

    /// Record the sites cache hits carried (D2). A name already known from a
    /// fresh definition keeps that location.
    pub fn add_carried(&mut self, carried: &'a CachedTermination) {
        for site in &carried.sites {
            self.by_name
                .entry(site.name.as_str())
                .or_insert((site.span, site.file.as_deref()));
        }
    }

    /// Where `name` was defined, if anywhere this lookup knows.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<(Span, Option<&'a Path>)> {
        self.by_name.get(name).copied()
    }
}
