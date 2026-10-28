//! The top-level names a file defines, and the ones it defines twice
//! (ADR 13.10.26f D1).
//!
//! Moved out of `companion.rs`, where 2.10.26j D5 introduced the namespaces
//! for its sibling-vs-companion check: that check is now the straddling case
//! of [`duplicate_definitions`], so the two cannot drift.
//!
//! Tests: `driver/modules/tests/duplicates/mod.rs`.

use std::collections::HashMap;
use std::path::Path;

use super::companion::{collision_error, straddling_collision};
use super::{ParsedModule, SourceMap};

use crate::ast::{Ident, Item, TypeBody, UseTree};
use crate::elaborate::ElabError;
use crate::span::Span;

/// Which namespace a top-level name lives in. An imported name's namespace is
/// unknown before resolution, so it is its own kind and meets both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Namespace {
    Value,
    Type,
    Import,
}

impl Namespace {
    /// Whether a name in `self` at one site and in `other` at another is a
    /// collision. Import against import is not: that is the existing
    /// duplicate-import error, E0007, which a shared scope already raises.
    pub(crate) fn collides_with(self, other: Namespace) -> bool {
        match (self, other) {
            (Namespace::Import, Namespace::Import) => false,
            (Namespace::Import, _) | (_, Namespace::Import) => true,
            (a, b) => a == b,
        }
    }
}

/// The names a `use` brings into scope: each path's last segment, or its
/// alias. A glob names nothing it can be checked by here.
pub(crate) fn imported_names(tree: &UseTree) -> Vec<&Ident> {
    match tree {
        UseTree::Path(path) => path.segments.last().into_iter().collect(),
        UseTree::Alias { alias, .. } => vec![alias],
        UseTree::Group { items, .. } => items.iter().flat_map(imported_names).collect(),
        UseTree::Glob { .. } => Vec::new(),
    }
}

/// One top-level definition: its namespace, its name's span, and the type
/// that declares it when it is a constructor.
pub(crate) type Definition<'a> = (Namespace, &'a str, Span, Option<&'a str>);

/// Every top-level name an item defines — the kinds `info error-sites E0004`
/// lists, plus a type's constructors, which share the value namespace.
pub(crate) fn definitions(item: &Item) -> Vec<Definition<'_>> {
    fn named(ns: Namespace, ident: &Ident) -> Definition<'_> {
        (ns, ident.name.as_str(), ident.span, None)
    }
    let value_name = |ident| named(Namespace::Value, ident);
    let type_name = |ident| named(Namespace::Type, ident);
    match item {
        Item::Function(f) => vec![value_name(&f.name)],
        Item::Theorem(t) | Item::Lemma(t) => vec![value_name(&t.name)],
        Item::Axiom(a) => vec![value_name(&a.name)],
        Item::ExternFn(e) => vec![value_name(&e.name)],
        Item::TypeAlias(a) => vec![type_name(&a.name)],
        Item::TypeDef(t) => {
            let mut names = vec![type_name(&t.name)];
            if let TypeBody::Sum(variants) = &t.body {
                names.extend(variants.iter().map(|v| {
                    let owner = Some(t.name.name.as_str());
                    (Namespace::Value, v.name.name.as_str(), v.name.span, owner)
                }));
            }
            names
        }
        // ADR 2.10.26j P0 item 8: an import against a definition was silently
        // accepted in both directions, so it counts.
        Item::Use(u) => imported_names(&u.tree)
            .into_iter()
            .map(|ident| named(Namespace::Import, ident))
            .collect(),
        Item::Mod(_) | Item::Error(_) => Vec::new(),
    }
}

/// A name defined a second time in one item list (ADR 13.10.26f D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// The doubly defined name.
    pub name: String,
    /// The earliest colliding definition: where the note points.
    pub first: Span,
    /// The later definition: where E0004 is reported.
    pub second: Span,
    /// For a constructor at both sites, the types declaring it:
    /// (the second site's, the first site's).
    pub owners: Option<(String, String)>,
}

impl Duplicate {
    /// Whether the pair straddles `base` — the first site in the sibling and
    /// the second in its companion (ADR 2.10.26j D5).
    #[must_use]
    pub fn straddles(&self, base: Option<u32>) -> bool {
        base.is_some_and(|b| self.first.start < b && self.second.start >= b)
    }
}

/// Every definition in `items` whose name an **earlier** definition in a
/// colliding namespace already holds, paired with the earliest such
/// definition, in source order (ADR 13.10.26f D1).
///
/// `base` is where an attached companion's text starts, so a caller can tell
/// a straddling pair ([`Duplicate::straddles`]) from a within-file one; it
/// does not change which pairs are found.
#[must_use]
pub fn duplicate_definitions(items: &[Item]) -> Vec<Duplicate> {
    let mut seen: HashMap<&str, Vec<Definition<'_>>> = HashMap::new();
    let mut found = Vec::new();
    for definition in items.iter().flat_map(definitions) {
        let (ns, name, span, owner) = definition;
        let earlier = seen.entry(name).or_default();
        if let Some(&(_, _, first, first_owner)) = earlier.iter().find(|e| ns.collides_with(e.0)) {
            found.push(Duplicate {
                name: name.to_string(),
                first,
                second: span,
                owners: owner
                    .zip(first_owner)
                    .map(|(second, first)| (second.to_string(), first.to_string())),
            });
        }
        earlier.push(definition);
    }
    found
}

/// E0004 at a within-file second definition, with a note at the first, both
/// spelled against `module` (ADR 13.10.26f D2). A pair inside an attached
/// companion is still spelled against its sibling; `relocate_error` moves it.
pub(crate) fn within_file_error(duplicate: &Duplicate, module: &Path) -> ElabError {
    let first = match &duplicate.owners {
        Some((_, first_owner)) => format!(
            "`{}` is first defined here, as a constructor of `{first_owner}`",
            duplicate.name
        ),
        None => format!("`{}` is first defined here", duplicate.name),
    };
    ElabError::duplicate(duplicate.second, duplicate.name.clone())
        .with_file_path(module)
        .with_span_note(duplicate.first, first)
        .with_help("a name is defined once per file; rename or remove one of them")
}

/// One E0004 per duplicate in every module of `tree`, in tree order
/// (ADR 13.10.26f D2): a pair straddling an attached companion's split in
/// 2.10.26j's companion form, every other in the within-file form. Spelled
/// against each module's path and not yet relocated.
pub(crate) fn duplicate_errors(tree: &ParsedModule, source_map: &SourceMap) -> Vec<ElabError> {
    let base = source_map
        .companions
        .iter()
        .find(|split| split.sibling == tree.path)
        .map(|split| split.base);
    let mut errors: Vec<ElabError> = duplicate_definitions(&tree.source_file.items)
        .iter()
        .map(
            |duplicate| match base.and_then(|b| straddling_collision(duplicate, b)) {
                Some(collision) => collision_error(&collision, &tree.path),
                None => within_file_error(duplicate, &tree.path),
            },
        )
        .collect();
    for submodule in &tree.submodules {
        errors.extend(duplicate_errors(submodule, source_map));
    }
    errors
}
