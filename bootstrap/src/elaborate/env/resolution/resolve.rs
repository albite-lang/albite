//! Path resolution logic for types, values, and constructors.

use crate::ast::Path;

use crate::elaborate::env::{
    ConstructorInfo, Env, ModulePath, PathResolutionError, ResolvedValue, TypeDef,
};

impl Env {
    // ─────────────────────────────────────────────────────────────────────────
    // Qualified path resolution
    // ─────────────────────────────────────────────────────────────────────────

    /// Resolve a qualified path to a value.
    ///
    /// For simple names (single segment), this behaves like `resolve_value`.
    /// For qualified names (multiple segments), this looks up the item
    /// in the specified module.
    ///
    /// Returns: Ok(Some(resolved)) on success, Ok(None) if not found,
    /// Err((module_path, item_name)) if module not found.
    pub fn resolve_value_path(
        &self,
        path: &Path,
        current_depth: usize,
        current_module: &ModulePath,
    ) -> Result<Option<ResolvedValue>, PathResolutionError> {
        if path.is_simple() {
            // Simple name - use existing resolution (includes locals and imports)
            let name = &path.item_name().name;

            // Check module-scoped imports first
            if let Some(import_info) = self.lookup_value_import(current_module, name) {
                return self
                    .resolve_in_module(&import_info.source_module, &import_info.original_name);
            }

            // Check for imported constructor
            if let Some(import_info) = self.lookup_constructor_import(current_module, name) {
                return self
                    .resolve_in_module(&import_info.source_module, &import_info.original_name);
            }

            // If this name was aliased away (imported under a different name),
            // don't fall through to the global table — the alias replaces, not supplements.
            if self.is_name_aliased_away(current_module, name) {
                return Ok(None);
            }

            // A value two modules define has no bare slot: the current
            // module's own, else the one definition the caller may reach and
            // see, else ambiguous (ADR 4.10.26j D3). Nor has a reserved link
            // name's one definition (ADR 9.10.26f D1). A local still shadows
            // it, and a constructor of that name is still reached below.
            if self.core_names.is_scoped(name) && self.lookup_local(name).is_none() {
                if let Some(resolved) = self.resolve_colliding_bare_value(name, current_module)? {
                    return Ok(Some(resolved));
                }
            }

            // Flat global table fallback — bypasses module-scoped import visibility.
            // All definitions from all modules are in `self.values` (populated during Signature Collection).
            // This means unqualified names resolve globally unless suppressed above
            // — or defined in a package this one did not declare, where the name
            // does not exist (ADR 3.10.26m D5). A `use` returned above, its path
            // already reach-checked, so only this fall-through is filtered.
            let resolved = self.resolve_value(name, current_depth);
            let unreachable = match &resolved {
                Some(ResolvedValue::Global(global, _)) => {
                    self.unreachable_value(current_module, global)
                }
                Some(ResolvedValue::Constructor(info)) => {
                    self.unreachable_constructor(current_module, info)
                }
                Some(ResolvedValue::Local(..)) | None => None,
            };
            Ok(resolved.filter(|_| unreachable.is_none()))
        } else {
            // Qualified path - could be Type::Constructor or module::item
            let item_name = &path.item_name().name;

            // For two-segment paths like "MyType::A", first check if the first segment
            // is a type name (imported or local) - this handles Type::Constructor syntax
            if path.segments.len() == 2 {
                let first_segment = &path.segments[0].name;

                // Check if this is an imported type - if so, look for constructor in source module
                if let Some(import_info) = self.lookup_type_import(current_module, first_segment) {
                    // The type is imported - look up constructor in its source module
                    return self.resolve_in_module(&import_info.source_module, item_name);
                }

                // Check if it's a local type - look for constructor belonging to that type
                if let Some(ctor_info) = self.lookup_constructor(item_name) {
                    if ctor_info.type_name == *first_segment {
                        return Ok(Some(ResolvedValue::Constructor(ctor_info.clone())));
                    }
                }
            }

            // Fall back to module path resolution
            // Try relative resolution first (child module of current_module),
            // then fall back to absolute path
            let module_segments: Vec<String> = path
                .module_segments()
                .iter()
                .map(|s| s.name.clone())
                .collect();

            let raw_path = ModulePath::from_segments(&module_segments);
            let module_path = self.resolve_qualifier(&raw_path, current_module);
            let module_path = self.reachable_module(module_path, current_module)?;
            self.resolve_in_module(&module_path, item_name)
        }
    }

    /// Resolve a qualified path to a type.
    ///
    /// For simple names (single segment), checks imports then local types.
    /// For qualified names (multiple segments), looks up the type in the specified module.
    pub fn resolve_type_path(
        &self,
        path: &Path,
        current_module: &ModulePath,
    ) -> Result<Option<&TypeDef>, PathResolutionError> {
        if path.is_simple() {
            let name = &path.item_name().name;

            // Check module-scoped imports first
            if let Some(import_info) = self.lookup_type_import(current_module, name) {
                return self.resolve_type_in_module(
                    &import_info.source_module,
                    &import_info.original_name,
                );
            }

            // If this name was aliased away, don't fall through to the global table.
            if self.is_name_aliased_away(current_module, name) {
                return Ok(None);
            }

            // Then direct lookup, unless its definer sits in a package this
            // one did not declare (ADR 4.10.26j D5, as 3.10.26m D5 for values).
            Ok(self
                .lookup_type(name)
                .filter(|_| self.unreachable_type(current_module, name).is_none()))
        } else {
            // Qualified path - look up in the specified module
            // Try relative resolution first (child or sibling), then absolute
            let module_segments: Vec<String> = path
                .module_segments()
                .iter()
                .map(|s| s.name.clone())
                .collect();
            let item_name = &path.item_name().name;

            let raw_path = ModulePath::from_segments(&module_segments);
            let module_path = self.resolve_qualifier(&raw_path, current_module);
            let module_path = self.reachable_module(module_path, current_module)?;
            self.resolve_type_in_module(&module_path, item_name)
        }
    }

    /// Resolve a qualified path to a constructor.
    ///
    /// For simple paths (just a constructor name), checks imports then local constructors.
    /// For qualified paths (Type::Constructor), looks up the type and then finds the constructor.
    /// Also supports module-qualified paths (module::Constructor).
    pub fn resolve_constructor_path(
        &self,
        path: &Path,
        current_module: &ModulePath,
    ) -> Result<Option<&ConstructorInfo>, PathResolutionError> {
        if path.is_simple() {
            let name = &path.item_name().name;

            // Check module-scoped imports first
            if let Some(import_info) = self.lookup_constructor_import(current_module, name) {
                return self.resolve_constructor_in_module(
                    &import_info.source_module,
                    &import_info.original_name,
                );
            }

            // If this name was aliased away, don't fall through to the global table.
            if self.is_name_aliased_away(current_module, name) {
                return Ok(None);
            }

            // Then direct lookup, unless it lands in an undeclared package (D5).
            Ok(self
                .lookup_constructor(name)
                .filter(|info| self.unreachable_constructor(current_module, info).is_none()))
        } else {
            // Qualified path - could be Type::Constructor or module::Constructor
            let item_name = &path.item_name().name;

            // For two-segment paths like "MyType::A", first check if the first segment
            // is a type name (imported or local)
            if path.segments.len() == 2 {
                let first_segment = &path.segments[0].name;

                // Check if this is an imported type
                if let Some(import_info) = self.lookup_type_import(current_module, first_segment) {
                    // The type is imported - look up constructor in its source module
                    // Constructors from ADTs are registered with the same module as the type
                    return self
                        .resolve_constructor_in_module(&import_info.source_module, item_name);
                }

                // Check if it's a local type (check if there's a constructor with this type name)
                if let Some(ctor_info) = self.lookup_constructor(item_name) {
                    // Verify the constructor's type matches the first segment
                    if ctor_info.type_name == *first_segment {
                        return Ok(Some(ctor_info));
                    }
                }
            }

            // Fall back to module path resolution: absolute only, rooted at
            // the current package or a declared dependency (3.10.26m D5).
            let raw_path = ModulePath::new(
                path.module_segments()
                    .iter()
                    .map(|s| s.name.clone())
                    .collect(),
            );
            let module_path = self.absolute_candidate(current_module, &raw_path);
            let module_path = self.reachable_module(module_path, current_module)?;
            self.resolve_constructor_in_module(&module_path, item_name)
        }
    }
}
