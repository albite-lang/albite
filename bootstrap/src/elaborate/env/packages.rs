//! Which package each module belongs to, and what each package may name
//! (ADR 29.9.26g §2.3, 3.10.26m D5 / §2.3).
//!
//! Every package is mounted once at the module root under its own name
//! (3.10.26m D4), and records the packages it declared. A package reaches its
//! own modules and its declared direct dependencies, and nothing else — the
//! application included. An empty table (no manifest) answers every query
//! as the single-package rules always did.

use serde::{Deserialize, Serialize};

use super::{ConstructorInfo, Env, ModulePath};

/// A package: the application (mounted at the root) or one of its path
/// dependencies, direct or transitive (ADR 29.9.26g, 3.10.26m).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageId {
    /// The project being built, mounted at the module root.
    Application,
    /// A mounted dependency: an index into [`PackageTable`]'s mounts.
    Dependency(usize),
}

/// Where each dependency is mounted, and what every package declared.
///
/// The application is implicit — it owns every module no mount prefixes — so
/// an empty table makes every query reduce to today's rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageTable {
    /// `(package name, mount path)` per dependency.
    mounts: Vec<(String, ModulePath)>,
    /// The application's `[package] name`, for diagnostics; `None` without one.
    application_name: Option<String>,
    /// The application's declared dependencies, as indices into `mounts`.
    application_declares: Vec<usize>,
    /// Per mount, that package's declared dependencies (indices into `mounts`).
    declares: Vec<Vec<usize>>,
}

impl PackageTable {
    /// Record a direct dependency of the application, mounted at `mount`
    /// (slice 1's shape: the application declares it). A test convenience:
    /// `mount_packages` builds the table from a plan with the two calls below.
    #[cfg(test)]
    pub fn add_dependency(&mut self, name: impl Into<String>, mount: ModulePath) {
        let index = self.add_package(name, mount);
        self.declare(PackageId::Application, index);
    }

    /// Mount a package nobody has declared yet; returns its index.
    pub fn add_package(&mut self, name: impl Into<String>, mount: ModulePath) -> usize {
        self.mounts.push((name.into(), mount));
        self.declares.push(Vec::new());
        self.mounts.len() - 1
    }

    /// Record that `from` declared the mounted package `to` (3.10.26m D5).
    pub fn declare(&mut self, from: PackageId, to: usize) {
        let list = match from {
            PackageId::Application => &mut self.application_declares,
            PackageId::Dependency(i) => &mut self.declares[i],
        };
        if !list.contains(&to) {
            list.push(to);
        }
    }

    /// Name the application, for diagnostics that speak of it.
    pub fn set_application_name(&mut self, name: impl Into<String>) {
        self.application_name = Some(name.into());
    }

    /// Whether any dependency is mounted.
    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }

    /// The package owning `path`: the dependency whose mount is its longest
    /// prefix, else the application. `path` should already be canonical.
    pub fn package_of(&self, path: &ModulePath) -> PackageId {
        self.mounts
            .iter()
            .enumerate()
            .filter(|(_, (_, mount))| path.starts_with(mount))
            .max_by_key(|(_, (_, mount))| mount.segments.len())
            .map_or(PackageId::Application, |(i, _)| PackageId::Dependency(i))
    }

    /// The module path a package's own absolute paths are rooted at.
    pub fn package_root(&self, id: PackageId) -> ModulePath {
        match id {
            PackageId::Application => ModulePath::root(),
            PackageId::Dependency(i) => self.mounts[i].1.clone(),
        }
    }

    /// A package's display name: a dependency's always, the application's
    /// when its manifest named it.
    pub fn package_name(&self, id: PackageId) -> Option<&str> {
        match id {
            PackageId::Application => self.application_name.as_deref(),
            PackageId::Dependency(i) => Some(self.mounts[i].0.as_str()),
        }
    }

    /// The packages `from` declared, as mount indices.
    fn declared(&self, from: PackageId) -> &[usize] {
        match from {
            PackageId::Application => &self.application_declares,
            PackageId::Dependency(i) => &self.declares[i],
        }
    }

    /// Whether code in `from` may name anything in `to`: the same package,
    /// or a package `from` declared (3.10.26m D5).
    pub fn may_reach(&self, from: PackageId, to: PackageId) -> bool {
        match to {
            _ if from == to => true,
            PackageId::Application => false,
            PackageId::Dependency(i) => self.declared(from).contains(&i),
        }
    }

    /// Where an absolute path written in `from` starts: the root when its
    /// first segment is a dependency `from` declared — every package is
    /// mounted there under its own name — else `from`'s own root (D5).
    pub fn absolute_root(&self, from: PackageId, first_segment: &str) -> ModulePath {
        let names_a_dependency = self
            .declared(from)
            .iter()
            .any(|&i| self.mounts[i].0 == first_segment);
        if names_a_dependency {
            ModulePath::root()
        } else {
            self.package_root(from)
        }
    }

    /// The names `from` declared, sorted: the reach fingerprint a module's
    /// cache key carries (3.10.26m D12). Empty without a manifest.
    pub fn declared_names(&self, from: PackageId) -> Vec<String> {
        let mut names: Vec<String> = self
            .declared(from)
            .iter()
            .map(|&i| self.mounts[i].0.clone())
            .collect();
        names.sort();
        names
    }
}

impl Env {
    /// The package owning `path`, after canonicalization (ADR 29.9.26g).
    pub fn package_of(&self, path: &ModulePath) -> PackageId {
        self.packages.package_of(&self.canonicalize_path(path))
    }

    /// The root a package's absolute paths resolve against (ADR 29.9.26g D5).
    pub fn package_root(&self, id: PackageId) -> ModulePath {
        self.packages.package_root(id)
    }

    /// Whether code in `from` may name `target` at all: the same package, or
    /// a package `from`'s package declared (ADR 3.10.26m D5).
    pub fn package_may_reach(&self, from: &ModulePath, target: &ModulePath) -> bool {
        self.packages
            .may_reach(self.package_of(from), self.package_of(target))
    }

    /// The absolute candidate for `raw` written in `current_module`: from the
    /// root when `raw` starts with a dependency the current package declared,
    /// else from the current package's root (ADR 3.10.26m D5). The one place
    /// the `use` resolver, the qualifier and the constructor fallback build it.
    pub fn absolute_candidate(&self, current_module: &ModulePath, raw: &ModulePath) -> ModulePath {
        let first = raw.segments.first().map_or("", String::as_str);
        self.packages
            .absolute_root(self.package_of(current_module), first)
            .join(raw)
    }

    /// The package `defining` belongs to, when `current_module` may not reach
    /// it (ADR 3.10.26m D5); `None` when it may, or without a manifest.
    fn unreachable_from(
        &self,
        current_module: &ModulePath,
        defining: &ModulePath,
    ) -> Option<PackageId> {
        if self.packages.is_empty() {
            return None;
        }
        let target = self.package_of(defining);
        let from = self.package_of(current_module);
        (!self.packages.may_reach(from, target)).then_some(target)
    }

    /// When the global value `name` — a bare name the flat table resolved, not
    /// bound by a `use` — is defined in a package `current_module` may not
    /// reach, that package (D5). `None` when it is reachable or has no
    /// recorded module. A value's recorded module is where it is defined.
    pub fn unreachable_value(&self, current_module: &ModulePath, name: &str) -> Option<PackageId> {
        self.unreachable_from(current_module, self.value_module(name)?)
    }

    /// [`Self::unreachable_value`] for a constructor, judged on its defining
    /// module: a re-export copies a constructor's name, never its details.
    pub fn unreachable_constructor(
        &self,
        current_module: &ModulePath,
        info: &ConstructorInfo,
    ) -> Option<PackageId> {
        let defining = info
            .defining_module
            .as_ref()
            .or_else(|| self.get_item_module(&info.type_name))?;
        self.unreachable_from(current_module, defining)
    }

    /// [`Self::unreachable_value`] for a bare type name, judged on its
    /// defining module, which is never a re-exporter (ADR 4.10.26j D5).
    pub fn unreachable_type(&self, current_module: &ModulePath, name: &str) -> Option<PackageId> {
        let defining = self
            .lookup_type(name)
            .and_then(|def| def.defining_module.as_ref())
            .or_else(|| self.get_item_module(name))?;
        self.unreachable_from(current_module, defining)
    }

    /// The package a bare name refused as unreachable lives in — a value,
    /// else a constructor — for the not-found error's help line.
    pub fn unreachable_bare_name(
        &self,
        current_module: &ModulePath,
        name: &str,
    ) -> Option<PackageId> {
        if self.has_value(name) {
            return self.unreachable_value(current_module, name);
        }
        let info = self.lookup_constructor(name)?;
        self.unreachable_constructor(current_module, info)
    }

    /// The help line for a bare name refused by [`Self::unreachable_bare_name`].
    pub fn undeclared_package_help(
        &self,
        current_module: &ModulePath,
        item: &str,
        target: PackageId,
    ) -> String {
        let name_of = |id| self.packages.package_name(id).unwrap_or("the application");
        let from = name_of(self.package_of(current_module));
        if target == PackageId::Application {
            // No library can declare the application: that is a cycle (D3).
            return format!(
                "`{item}` is defined in the application, which package `{from}` cannot name"
            );
        }
        format!(
            "`{item}` is defined in package `{}`, which `{from}` does not declare in `[dependencies]`",
            name_of(target),
        )
    }
}
