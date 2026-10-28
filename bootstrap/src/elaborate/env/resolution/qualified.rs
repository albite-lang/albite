//! Resolution inside a named module, and the bare-name fall-through for a
//! value two modules define (ADR 4.10.26j D2, D3).
//!
//! A qualified path answers from the target module's own binding: the
//! qualifier selects the definition, it does not merely gate a lookup of the
//! program-wide bare slot.

use crate::elaborate::env::{
    ConstructorInfo, Env, ModulePath, PathResolutionError, ResolvedValue, TypeDef, ValueDef,
};

/// How many `pub use` hops a qualified value lookup follows to a scoped
/// definition before giving up; a chain this long is a cycle.
const MAX_REEXPORT_HOPS: usize = 32;

/// `module::name` as a user would write it: the bare name for the root.
pub fn qualified_value_path(module: &ModulePath, name: &str) -> String {
    if module.is_root() {
        name.to_string()
    } else {
        format!("{module}::{name}")
    }
}

impl Env {
    /// The module a qualified path's module segments name: the child of
    /// `current_module`, else its sibling, else the path rooted at the
    /// current package — the crate root for the application (ADR 29.9.26g D5)
    /// — or at the root for a declared dependency (3.10.26m D5). A child or
    /// sibling in another package is skipped unless the current one declared it.
    pub(super) fn resolve_qualifier(
        &self,
        raw_path: &ModulePath,
        current_module: &ModulePath,
    ) -> ModulePath {
        let admissible =
            |p: &ModulePath| self.has_module(p) && self.package_may_reach(current_module, p);
        let child_path = current_module.join(raw_path);
        if admissible(&child_path) {
            return child_path;
        }
        if let Some(sibling_path) = current_module.parent().map(|parent| parent.join(raw_path)) {
            if admissible(&sibling_path) {
                return sibling_path;
            }
        }
        self.absolute_candidate(current_module, raw_path)
    }

    /// `module_path`, unless the current package may not reach it — every
    /// package is mounted at the root, so the absolute fallback can land in
    /// one the current package never declared, and to it that module does
    /// not exist (ADR 3.10.26m D5).
    pub(super) fn reachable_module(
        &self,
        module_path: ModulePath,
        current_module: &ModulePath,
    ) -> Result<ModulePath, PathResolutionError> {
        if self.package_may_reach(current_module, &module_path) {
            Ok(module_path)
        } else {
            Err(PathResolutionError::ModuleNotFound(module_path))
        }
    }

    /// The value `local` names in `module`: the module's own definition
    /// under its Core name, following a `pub use` chain to a scoped one
    /// (ADR 4.10.26j D2). A name no module scopes is the bare slot, as before.
    pub(super) fn value_in_module(&self, module: &ModulePath, local: &str) -> Option<&ValueDef> {
        let mut module = module.clone();
        let mut name = local.to_string();
        for _ in 0..MAX_REEXPORT_HOPS {
            let core_name = self.core_name_in(&module, &name);
            if core_name != name {
                return self.lookup_value(&core_name);
            }
            let source = self
                .get_module(&module)
                .and_then(|contents| contents.reexported_value_sources.get(&name));
            match source {
                Some((source_module, original)) if self.core_names.is_scoped(original) => {
                    module = source_module.clone();
                    name = original.clone();
                }
                _ => break,
            }
        }
        self.lookup_value(local)
    }

    /// A bare `name` that two or more modules define, or a reserved link
    /// name one module defines, not bound by a local or a `use` (ADR
    /// 4.10.26j D3, 9.10.26f D1): the current module's own definition, else
    /// the one definition the caller may reach and see. Two such are
    /// ambiguous. With none, a reachable private one is returned so the
    /// caller reports it as private (E0016), as before.
    pub(super) fn resolve_colliding_bare_value(
        &self,
        name: &str,
        current_module: &ModulePath,
    ) -> Result<Option<ResolvedValue>, PathResolutionError> {
        let global = |module: &ModulePath| {
            self.lookup_value(&self.core_names.core_name(module, name))
                .map(|def| ResolvedValue::Global(def.name.clone(), def.ty.clone()))
        };
        let current = self.canonicalize_path(current_module);
        let definers = self.core_names.scoped_definers(name);
        if definers.contains(&current) {
            return Ok(global(&current));
        }
        let reachable: Vec<&ModulePath> = definers
            .iter()
            .filter(|module| self.package_may_reach(current_module, module))
            .collect();
        let visible: Vec<&ModulePath> = reachable
            .iter()
            .copied()
            .filter(|module| {
                self.lookup_value(&self.core_names.core_name(module, name))
                    .is_some_and(|def| {
                        self.is_item_accessible(def.visibility, module, current_module)
                    })
            })
            .collect();
        match visible.as_slice() {
            [only] => Ok(global(only)),
            [] => Ok(reachable.first().and_then(|module| global(module))),
            _ => Err(PathResolutionError::Ambiguous {
                item: name.to_string(),
                candidates: visible
                    .iter()
                    .map(|module| qualified_value_path(module, name))
                    .collect(),
            }),
        }
    }

    /// Resolve an item in a specific module.
    pub(super) fn resolve_in_module(
        &self,
        module_path: &ModulePath,
        item_name: &str,
    ) -> Result<Option<ResolvedValue>, PathResolutionError> {
        // Check if module exists
        if !self.has_module(module_path) {
            return Err(PathResolutionError::ModuleNotFound(module_path.clone()));
        }

        // First, check if item is listed in module contents
        // This is more reliable than item_modules which can have collisions
        let has_ctor = self.has_constructor_in_module(module_path, item_name);
        let has_value = self.has_value_in_module(module_path, item_name);

        if has_ctor {
            if let Some(info) = self.lookup_constructor(item_name) {
                return Ok(Some(ResolvedValue::Constructor(info.clone())));
            }
        }

        if has_value {
            if let Some(def) = self.value_in_module(module_path, item_name) {
                return Ok(Some(ResolvedValue::Global(
                    def.name.clone(),
                    def.ty.clone(),
                )));
            }
        }

        // Fallback: Check if item_modules says it's in this module (handles items not yet in module contents)
        if let Some(item_module) = self.get_item_module(item_name) {
            if item_module == module_path {
                // Item exists and is in the right module - look it up
                if let Some(def) = self.lookup_value(item_name) {
                    return Ok(Some(ResolvedValue::Global(
                        def.name.clone(),
                        def.ty.clone(),
                    )));
                }
                if let Some(info) = self.lookup_constructor(item_name) {
                    return Ok(Some(ResolvedValue::Constructor(info.clone())));
                }
            }
        }

        // Item not found in module
        Ok(None)
    }

    /// Resolve a type in a specific module.
    pub(super) fn resolve_type_in_module(
        &self,
        module_path: &ModulePath,
        item_name: &str,
    ) -> Result<Option<&TypeDef>, PathResolutionError> {
        // Check if module exists
        if !self.has_module(module_path) {
            return Err(PathResolutionError::ModuleNotFound(module_path.clone()));
        }

        // Check if item is listed in that module's contents
        // This is more reliable than checking item_modules because item_modules
        // can have collisions when multiple modules define types with the same name.
        let has_type = self.has_type_in_module(module_path, item_name);
        if has_type {
            return Ok(self.lookup_type(item_name));
        }

        // Fallback: Check if item_modules says it's in this module
        // This handles items not yet in module contents (shouldn't happen normally)
        if let Some(item_module) = self.get_item_module(item_name) {
            if item_module == module_path {
                return Ok(self.lookup_type(item_name));
            }
        }

        Ok(None)
    }

    /// Resolve a constructor in a specific module.
    pub(super) fn resolve_constructor_in_module(
        &self,
        module_path: &ModulePath,
        item_name: &str,
    ) -> Result<Option<&ConstructorInfo>, PathResolutionError> {
        // Check if module exists
        if !self.has_module(module_path) {
            return Err(PathResolutionError::ModuleNotFound(module_path.clone()));
        }

        // Check if item is defined in that module using item_modules
        if let Some(item_module) = self.get_item_module(item_name) {
            if item_module == module_path {
                return Ok(self.lookup_constructor(item_name));
            }
        }

        // Fallback: check if constructor is in module's constructor list
        // This is more reliable than item_modules which can have collisions
        if let Some(contents) = self.get_module(module_path) {
            if contents.constructors.iter().any(|c| c == item_name) {
                return Ok(self.lookup_constructor(item_name));
            }
        }

        Ok(None)
    }
}
