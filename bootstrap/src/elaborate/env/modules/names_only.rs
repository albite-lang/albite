//! A constructor whose parent is an undeclared sibling's type stub (ADR
//! 2.10.26g D4).
//!
//! The workspace merge gives the elaborator a sibling file's *names*, never
//! its definitions (30.9.26i D1), so a constructor imported from one resolves
//! to a parent that stays a `TypeDefKind::Stub` for the whole check. The three
//! sites that need the parent's constructor list used to report that as an
//! internal error, which blames the compiler for a program that is simply
//! wrong. This asks first, and turns it into the user's "cannot find
//! constructor" — the verdict is unchanged, only the code and the words move.

use std::collections::HashSet;

use crate::elaborate::env::{TypeDef, TypeDefKind};
use crate::elaborate::error::ElabError;
use crate::span::Span;

use super::path::ModulePath;

/// `E0003` for constructor `ctor` when its parent `type_def` is a stub whose
/// defining module is in `names_only`; `None` for every other parent, which
/// keeps the caller's own (internal) error.
///
/// A stub's `defining_module` is the FIRST module to register that bare type
/// name, so with two modules exporting `Box` it may name the wrong one. That
/// can only change whether this site says `E0003` or keeps `E9998`: the site
/// is reached solely where a refusal is already certain.
pub fn sibling_constructor_refusal<S: std::hash::BuildHasher>(
    ctor: &str,
    type_def: &TypeDef,
    names_only: &HashSet<ModulePath, S>,
    span: Span,
) -> Option<ElabError> {
    if !matches!(type_def.kind, TypeDefKind::Stub) {
        return None;
    }
    let module = type_def
        .defining_module
        .as_ref()
        .filter(|module| names_only.contains(*module))?;
    Some(
        ElabError::undefined_constructor(span, ctor).with_note(format!(
            "`{ctor}` comes from module `{module}`, a file in this directory that no `mod` \
         declares; its names are visible but its definitions are not compiled"
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Visibility;
    use crate::elaborate::error::ElabErrorKind;

    fn type_def(kind: TypeDefKind, defining_module: Option<&str>) -> TypeDef {
        TypeDef {
            name: "Box".to_string(),
            params: Vec::new(),
            kind,
            visibility: Visibility::Public,
            span: Span::new(0, 0),
            defining_module: defining_module.map(|m| ModulePath::from_segments(&[m.to_string()])),
            encoded_type: None,
            field_visibilities: Vec::new(),
        }
    }

    fn boxes_marked() -> HashSet<ModulePath> {
        HashSet::from([ModulePath::from_segments(&["boxes".to_string()])])
    }

    // ADR 2.10.26g AC3: a stub parent from a names-only module is E0003,
    // naming the constructor, with a note naming the module.
    #[test]
    fn sibling_parent_stub_is_an_undefined_constructor() {
        let def = type_def(TypeDefKind::Stub, Some("boxes"));
        let err = sibling_constructor_refusal("MkBox", &def, &boxes_marked(), Span::new(3, 8))
            .expect("a names-only stub parent is refused");
        assert!(
            matches!(&err.kind, ElabErrorKind::UndefinedConstructor(name) if name == "MkBox"),
            "{:?}",
            err.kind
        );
        assert_eq!(err.span, Span::new(3, 8));
        assert_eq!(err.notes.len(), 1);
        assert!(
            err.notes[0].message.contains("`boxes`"),
            "{}",
            err.notes[0].message
        );
        assert!(
            err.notes[0].message.contains("`MkBox`"),
            "{}",
            err.notes[0].message
        );
    }

    // ADR 2.10.26g AC3: a stub from an UNMARKED module — or one with no
    // recorded module — keeps the caller's internal error.
    #[test]
    fn unmarked_parent_stub_stays_internal() {
        let declared = type_def(TypeDefKind::Stub, Some("shapes"));
        assert!(
            sibling_constructor_refusal("MkBox", &declared, &boxes_marked(), Span::new(0, 1))
                .is_none()
        );
        let unknown = type_def(TypeDefKind::Stub, None);
        assert!(
            sibling_constructor_refusal("MkBox", &unknown, &boxes_marked(), Span::new(0, 1))
                .is_none()
        );
        let stub = type_def(TypeDefKind::Stub, Some("boxes"));
        assert!(
            sibling_constructor_refusal("MkBox", &stub, &HashSet::new(), Span::new(0, 1)).is_none()
        );
    }

    // ADR 2.10.26g AC3: only a Stub is refused here — an ADT, alias or
    // record parent from a marked module is not this rule's business.
    #[test]
    fn adt_parent_is_not_refused() {
        for kind in [
            TypeDefKind::ADT(Vec::new()),
            TypeDefKind::Alias(albite_core::Type::Nat),
            TypeDefKind::Record(Vec::new()),
        ] {
            let def = type_def(kind, Some("boxes"));
            assert!(
                sibling_constructor_refusal("MkBox", &def, &boxes_marked(), Span::new(0, 1))
                    .is_none()
            );
        }
    }
}
