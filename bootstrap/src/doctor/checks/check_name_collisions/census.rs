//! The collision census: invert the per-module value tables into a
//! name → defining-modules multimap and classify what comes back (ADR 13.8.26c).
//!
//! Everything here is a pure function over injected data — a [`ModuleInfo`] and
//! a set of `extern "C"` definition sites — so the classification is assertable
//! without parsing a file, let alone elaborating one.

use std::collections::{BTreeMap, HashSet};

use crate::ast::Visibility;
use crate::driver::modules::ModuleInfo;
use crate::elaborate::{ModulePath, PackageId};

/// One place a value name is defined, and the two facts that classify it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionSite {
    /// The module that defines it.
    pub module: ModulePath,
    /// Whether a call site outside that module can name it at all.
    pub visibility: Visibility,
    /// Whether the definition is an `extern "C" fn`, whose Tungsten identifier
    /// *is* its C symbol (ADR 7.8.26b D3) — so a pair is a duplicate link
    /// symbol as well as a shadowed binding.
    pub is_extern_c: bool,
    /// The Core name the definition elaborates under: scoped when the
    /// elaborator scopes this name (ADR 4.10.26j D1, D8).
    pub core_name: String,
}

/// Why a collision matters, and therefore what its fix is (D3; ADR 4.10.26j D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CollisionClass {
    /// Two or more `extern "C"` definitions: also a duplicate link symbol, and
    /// no visibility change can fix it — one of them has to be renamed. Across
    /// packages the build refuses it as E0123's extern arm.
    ExternSymbol,
    /// Any other value defined in two modules. Since values are keyed by
    /// (module, name), each definition has its own Core name and a qualified
    /// path names exactly one; this is information, never a finding. The
    /// `private-shadowed` and `latent-public` classes it replaced described
    /// the bare-keyed table, where the last registration won.
    Scoped,
}

impl CollisionClass {
    /// The label a report prints for this class.
    pub fn label(self) -> &'static str {
        match self {
            CollisionClass::ExternSymbol => "extern-symbol",
            CollisionClass::Scoped => "scoped",
        }
    }

    /// Whether this class is an error *today* rather than information.
    pub fn is_live(self) -> bool {
        match self {
            CollisionClass::ExternSymbol => true,
            CollisionClass::Scoped => false,
        }
    }
}

/// One name bound in more than one module of the reachable tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    /// The bare name the definitions share.
    pub name: String,
    /// Every module defining it, ordered by module path.
    pub sites: Vec<DefinitionSite>,
    /// Whether the sites sit in two or more packages, where an
    /// `extern-symbol` pair is the build's E0123 rather than advice.
    pub crosses_packages: bool,
    /// What kind of problem this is.
    pub class: CollisionClass,
}

/// The whole run, reach line included.
///
/// `modules_examined` and `names_considered` sit beside the findings so that
/// `0 collisions` and `0 modules examined` cannot render alike (§2.1) — the
/// failure ADR 11.8.26c shipped and repaired in its own retrospective.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    /// Modules in the reachable tree.
    pub modules_examined: usize,
    /// Distinct value names bound across them.
    pub names_considered: usize,
    /// Definition sites summed over all names — the multimap's edge count.
    pub definitions_considered: usize,
    /// Every name bound in ≥2 modules, ordered by name.
    pub collisions: Vec<Collision>,
    /// Manifest packages mounted into the tree before counting (ADR 15.10.26n
    /// D4) — what `mount_packages` mounted, not what was declared. Zero
    /// without a manifest, and the reach line is then unchanged.
    pub packages_mounted: usize,
}

/// Whether the `pub use` pass's synthesized copies are subtracted (D1b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReexportHandling {
    /// The shipped behaviour: a re-exported name is one definition reachable by
    /// several paths, which is `reexport-completeness`'s question, not this one.
    Subtract,
    /// Measurement only (AC 1): keep them, so the difference between the two
    /// runs *is* the re-export class.
    Keep,
}

/// Which severity classes a run reports.
///
/// `ValueEnum` is derived here rather than mirrored in the clap layer: a second
/// copy would be one more thing to keep in step, and this ADR's D4 is a whole
/// decision about not making second copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Severity {
    /// Every class, including the informational `scoped` one.
    All,
    /// Only the classes that are errors today.
    Live,
}

impl Severity {
    /// Whether a finding of this class survives the filter.
    pub fn admits(self, class: CollisionClass) -> bool {
        match self {
            Severity::All => true,
            Severity::Live => class.is_live(),
        }
    }
}

/// Invert `modules[*].values` into the multimap and classify every name bound
/// more than once.
///
/// Reads `modules`, **not** `item_modules`: the flat map's unconditional insert
/// destroys the collision before `ModuleInfo` is finished, so it cannot be its
/// own diagnostic (D1a). No definition "wins" any more: each has its own Core
/// name (ADR 4.10.26j D8), which [`DefinitionSite::core_name`] reports.
#[allow(clippy::implicit_hasher)] // Reason: callers all use the default hasher
pub fn census(
    info: &ModuleInfo,
    extern_sites: &HashSet<(ModulePath, String)>,
    reexports: ReexportHandling,
) -> Census {
    // Keyed on `segments` rather than on `ModulePath`, which implements no
    // ordering: the inner map both deduplicates a module and fixes the report
    // order, so two runs over the same tree cannot print the sites differently.
    let mut multimap: BTreeMap<&str, BTreeMap<&[String], &ModulePath>> = BTreeMap::new();
    let mut definitions_considered = 0;

    for (module, contents) in &info.modules {
        for name in &contents.values {
            if reexports == ReexportHandling::Subtract
                && contents.reexported_value_sources.contains_key(name)
            {
                continue;
            }
            definitions_considered += 1;
            multimap
                .entry(name.as_str())
                .or_default()
                .insert(module.segments.as_slice(), module);
        }
    }

    let collisions = multimap
        .iter()
        .filter(|(_, modules)| modules.len() > 1)
        .map(|(name, modules)| collision(info, extern_sites, name, modules))
        .collect();

    Census {
        modules_examined: info.modules.len(),
        names_considered: multimap.len(),
        definitions_considered,
        collisions,
        packages_mounted: 0,
    }
}

/// Build one finding from the multimap entry for `name`.
fn collision(
    info: &ModuleInfo,
    extern_sites: &HashSet<(ModulePath, String)>,
    name: &str,
    modules: &BTreeMap<&[String], &ModulePath>,
) -> Collision {
    let core_names = info.core_names();
    let sites: Vec<DefinitionSite> = modules
        .values()
        .map(|module| DefinitionSite {
            module: (*module).clone(),
            visibility: info
                .modules
                .get(*module)
                .and_then(|contents| contents.value_visibility.get(name))
                .copied()
                .unwrap_or_default(),
            is_extern_c: extern_sites.contains(&((*module).clone(), name.to_string())),
            core_name: core_names.core_name(&info.canonicalize_path(module), name),
        })
        .collect();
    let packages: HashSet<PackageId> = sites
        .iter()
        .map(|site| info.packages.package_of(&site.module))
        .collect();

    Collision {
        class: classify(&sites),
        crosses_packages: packages.len() > 1,
        name: name.to_string(),
        sites,
    }
}

/// Classify a set of definition sites (D3; ADR 4.10.26j D8).
///
/// Only a pair of `extern "C"` definitions is still a problem: their shared
/// identifier is one C symbol, which module scoping cannot separate. Every
/// other collision — private, `pub(crate)` or `pub` alike — is scoped.
fn classify(sites: &[DefinitionSite]) -> CollisionClass {
    if sites.iter().filter(|site| site.is_extern_c).count() > 1 {
        CollisionClass::ExternSymbol
    } else {
        CollisionClass::Scoped
    }
}
