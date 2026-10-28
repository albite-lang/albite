//! Module path resolution for use declarations.

use crate::elaborate::env::{ModulePath, PackageId};
use crate::elaborate::error::ElabError;
use crate::elaborate::ElabResult;
use crate::elaborate::Elaborator;

use super::{emit_relative_fallback_failure, emit_relative_fallback_warning};

impl<'a> Elaborator<'a> {
    pub(crate) fn resolve_use_module_path(
        &self,
        segments: &[String],
        use_module: &ModulePath,
        span: crate::span::Span,
    ) -> ElabResult<ModulePath> {
        if segments.is_empty() {
            return Err(ElabError::module_not_found(span, "<empty>"));
        }

        let package = self.env.package_of(use_module);
        let package_root = self.env.package_root(package);

        // Handle `super::` prefix - navigate up from current module
        if segments[0] == "super" {
            // `super` never crosses a mount (ADR 29.9.26g D5): a library's root
            // has no parent it may name. Defensive: since 29.9.26f `super` is a
            // reserved word the parser refuses in a `use` (E0116), so no parsed
            // source reaches this branch today.
            if super_escapes_package(package, use_module, &package_root) {
                return Err(ElabError::module_not_found(
                    span,
                    "super (a package's root module has no parent)",
                ));
            }
            let parent = use_module
                .parent()
                .ok_or_else(|| ElabError::module_not_found(span, "super (no parent module)"))?;

            if segments.len() == 1 {
                // Just `super` - resolve to parent
                return Ok(parent);
            }

            // Recursively resolve the rest relative to parent
            return self.resolve_use_module_path(&segments[1..], &parent, span);
        }

        // Build the raw path from segments
        let raw_path = ModulePath::from_segments(segments);
        // Try child resolution first (current module + path)
        // This allows `use common::*` inside `ast/mod.tg` to find `ast::common`
        // A candidate outside the current package is discarded unless the
        // current package declared it (ADR 29.9.26g D5, 3.10.26m D5).
        let admissible =
            |p: &ModulePath| self.env.has_module(p) && self.env.package_may_reach(use_module, p);
        let child_path = use_module.join(&raw_path);
        let child_exists = admissible(&child_path);

        // Try sibling resolution (relative to use_module's parent)
        let sibling_path = use_module.parent().map(|parent| parent.join(&raw_path));
        let sibling_exists = sibling_path.as_ref().is_some_and(admissible);

        // Try absolute resolution — from the root when the path starts with a
        // declared dependency, else from the current package's root (D5).
        let absolute_path = self.env.absolute_candidate(use_module, &raw_path);
        let absolute_exists = admissible(&absolute_path);

        let chosen = choose_use_candidate(
            (&child_path, child_exists),
            sibling_path.as_ref().map(|p| (p, sibling_exists)),
            (&absolute_path, absolute_exists),
        );
        match chosen {
            Some(UseResolution {
                module,
                relative_via: Some(strategy),
            }) => {
                emit_relative_fallback_warning(&raw_path, &module, use_module, strategy);
                Ok(module)
            }
            Some(UseResolution { module, .. }) => Ok(module),
            None => {
                emit_relative_fallback_failure(&raw_path, use_module);
                // Only candidates this package may name are worth listing.
                let sibling_path =
                    sibling_path.filter(|p| self.env.package_may_reach(use_module, p));
                Err(self.build_module_not_found_error(
                    (&absolute_path, &child_path, sibling_path.as_ref()),
                    use_module,
                    span,
                ))
            }
        }
    }

    /// Build a module-not-found error with tried paths and a suggestion —
    /// only ever a module `use_module` may reach, so an undeclared package
    /// is never offered as the fix (ADR 3.10.26m D5).
    fn build_module_not_found_error(
        &self,
        (raw_path, child_path, sibling_path): (&ModulePath, &ModulePath, Option<&ModulePath>),
        use_module: &ModulePath,
        span: crate::span::Span,
    ) -> ElabError {
        use crate::utils::find_best_suggestion;

        let tried_paths = tried_paths_text(
            &[
                Some(child_path.to_string()),
                sibling_path.map(|p| p.to_string()),
                Some(raw_path.to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        );

        let all_modules: Vec<String> = self
            .env
            .all_module_paths()
            .filter(|p| self.env.package_may_reach(use_module, p))
            .map(|p| p.to_string())
            .collect();

        let suggestion = find_best_suggestion(
            &raw_path.to_string(),
            all_modules.iter().map(|s| s.as_str()),
        );

        ElabError::module_not_found_with_suggestion(
            span,
            tried_paths,
            suggestion.map(|s| s.to_string()),
        )
    }
}

/// The module-not-found message's path list: each tried path once, in the
/// order first tried (ADR 2.10.26g D6). At the root the child and the raw
/// path coincide, which printed `nosuch or nosuch`.
pub(crate) fn tried_paths_text(paths: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::with_capacity(paths.len());
    for path in paths {
        if !seen.contains(&path.as_str()) {
            seen.push(path);
        }
    }
    seen.join(" or ")
}

/// Where a `use` path resolved, and — when it was reached relatively — the
/// strategy the fallback warning names (ADR 10.5.26m, 29.9.26g D5).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UseResolution {
    pub(crate) module: ModulePath,
    pub(crate) relative_via: Option<&'static str>,
}

/// Pick a `use` path's module from its three candidates, each paired with
/// whether it exists and the current package may name it. Priority is
/// child > sibling > absolute; a child or sibling that IS the package-rooted
/// absolute path (a library root's own `use shapes::…`) counts as absolute
/// and warns about nothing — the rule the application already gets from
/// `raw == absolute`. `None` when no candidate is admissible.
pub(crate) fn choose_use_candidate(
    child: (&ModulePath, bool),
    sibling: Option<(&ModulePath, bool)>,
    absolute: (&ModulePath, bool),
) -> Option<UseResolution> {
    let resolve = |path: &ModulePath, strategy: &'static str| UseResolution {
        module: path.clone(),
        relative_via: (path != absolute.0).then_some(strategy),
    };
    if child.1 {
        return Some(resolve(child.0, "child"));
    }
    if let Some((sibling_path, true)) = sibling {
        return Some(resolve(sibling_path, "sibling"));
    }
    absolute.1.then(|| resolve(absolute.0, "absolute"))
}

/// Whether a leading `super` would leave the package: true in a dependency's
/// root module, whose parent is the application (ADR 29.9.26g D5).
pub(crate) fn super_escapes_package(
    package: PackageId,
    use_module: &ModulePath,
    package_root: &ModulePath,
) -> bool {
    package != PackageId::Application && use_module == package_root
}

// Tests: resolve_tests.rs — candidate choice and the absolute candidate (ADR 29.9.26g, 3.10.26m)
#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
