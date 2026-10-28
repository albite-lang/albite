//! Affine resource handles, `B4.1` (ADR 7.10.26t): a `StringBuilder` or a
//! `Bytes` is used at most once on every path through a function.
//!
//! Elaboration erases the single-field wrapper — a builder reaches Core as a
//! `Nat` binding — so nothing here reads a semantic type. Everything is read
//! from **declarations** in the parsed tree (D2): the fixed handle list by
//! declared name, the "contains a handle" walk over declared type
//! expressions, and D3's function-kind table keyed by bare name and a
//! signature that mentions a handle. The use rule itself is in [`uses`].
//!
//! The self-host computes the same rule over its own parsed tree
//! (`src/compiler/elab/items/handles/`), so the two compilers walk the same
//! representation and raise the same refusals at the same spans (M8).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::ast::{Item, TypeBody, TypeExpr};
use crate::driver::modules::ParsedModule;
use crate::elaborate::error::handle_reason;
use crate::elaborate::{ElabError, ElabErrorKind, HandleErrorKind, RegionErrorKind};

pub mod policy;
pub mod uses;

#[cfg(test)]
mod forms_tests;
#[cfg(test)]
mod region_tests;
#[cfg(test)]
mod tests;

pub use policy::{ArmPolicy, HandleFinding, HandleFindingKind, UsePolicy};
pub use uses::check_function;

/// The fixed handle list (D2): each handle type with its constructor.
pub const HANDLE_TYPES: [(&str, &str); 2] =
    [("StringBuilder", "MkStringBuilder"), ("Bytes", "MkBytes")];

/// What a call does to a handle argument (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleFunctionKind {
    /// Returns a fresh handle.
    Maker,
    /// Reads a handle and leaves it live.
    Observer,
    /// Consumes a handle and returns its successor.
    Threader,
    /// Consumes a handle and returns none.
    Consumer,
}

/// D3's table, by bare function name.
const FUNCTION_KINDS: [(&str, HandleFunctionKind); 14] = [
    ("string_builder_new", HandleFunctionKind::Maker),
    ("string_builder_with_capacity", HandleFunctionKind::Maker),
    ("bytes_new", HandleFunctionKind::Maker),
    ("bytes_from_string_grown", HandleFunctionKind::Maker),
    ("string_builder_len", HandleFunctionKind::Observer),
    ("bytes_len", HandleFunctionKind::Observer),
    ("bytes_get", HandleFunctionKind::Observer),
    ("push_str", HandleFunctionKind::Threader),
    ("push_char", HandleFunctionKind::Threader),
    ("bytes_set", HandleFunctionKind::Threader),
    ("bytes_blit_string", HandleFunctionKind::Threader),
    ("to_string", HandleFunctionKind::Consumer),
    ("bytes_into_string", HandleFunctionKind::Consumer),
    ("bytes_into_string_unchecked", HandleFunctionKind::Consumer),
];

/// Whether `name`, a declared type's last path segment, is on the list.
#[must_use]
pub fn is_handle_type_name(name: &str) -> bool {
    HANDLE_TYPES.iter().any(|(ty, _)| *ty == name)
}

/// Whether a declared type is itself a listed handle — what makes a
/// parameter a handle variable (D4), unlike [`HandleDeclarations::contains_handle`].
#[must_use]
pub fn is_listed_handle(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Path(path) => path
            .segments
            .last()
            .is_some_and(|segment| is_handle_type_name(&segment.name)),
        TypeExpr::Paren(inner, _) => is_listed_handle(inner),
        _ => false,
    }
}

/// One declared function signature: its parameter types and result type.
#[derive(Debug, Clone)]
struct DeclaredSignature {
    params: Vec<TypeExpr>,
    result: Option<TypeExpr>,
}

/// The declarations the rule reads, gathered from every module of a program
/// and keyed by bare name. Two declarations of one name are both kept, and a
/// question holds when it holds of any of them.
#[derive(Debug, Clone, Default)]
pub struct HandleDeclarations {
    functions: BTreeMap<String, Vec<DeclaredSignature>>,
    types: BTreeMap<String, Vec<TypeExpr>>,
}

impl HandleDeclarations {
    /// Gather the function signatures and type bodies of `items`.
    #[must_use]
    pub fn from_items<'a>(items: impl IntoIterator<Item = &'a Item>) -> Self {
        let mut declarations = Self::default();
        for item in items {
            declarations.add_item(item);
        }
        declarations
    }

    fn add_item(&mut self, item: &Item) {
        match item {
            Item::Function(function) => {
                let signature = DeclaredSignature {
                    params: function
                        .params
                        .iter()
                        .map(|param| param.ty.clone())
                        .collect(),
                    result: function.return_type.clone(),
                };
                self.functions
                    .entry(function.name.name.clone())
                    .or_default()
                    .push(signature);
            }
            Item::TypeDef(def) => {
                self.types
                    .entry(def.name.name.clone())
                    .or_default()
                    .extend(body_parts(&def.body));
            }
            Item::TypeAlias(alias) => {
                let body = alias.ty.clone();
                self.types
                    .entry(alias.name.name.clone())
                    .or_default()
                    .push(body);
            }
            _ => {}
        }
    }

    /// Whether `ty` contains a handle (D2): it names a listed type, names a
    /// declared type whose fields or body contain one, or applies a type to
    /// an argument that does. A function type contains none — a closure is
    /// not a handle — and a recursive type ends on its visited set.
    #[must_use]
    pub fn contains_handle(&self, ty: &TypeExpr) -> bool {
        self.contains_visiting(ty, &mut BTreeSet::new())
    }

    fn contains_visiting(&self, ty: &TypeExpr, visited: &mut BTreeSet<String>) -> bool {
        match ty {
            TypeExpr::Path(path) => {
                let Some(name) = path.segments.last().map(|segment| segment.name.as_str()) else {
                    return false;
                };
                if is_handle_type_name(name) {
                    return true;
                }
                if !visited.insert(name.to_string()) {
                    return false;
                }
                self.types.get(name).is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| self.contains_visiting(part, visited))
                })
            }
            TypeExpr::App(base, args, _) => {
                self.contains_visiting(base, visited)
                    || args.iter().any(|arg| self.contains_visiting(arg, visited))
            }
            TypeExpr::Product(left, right, _) | TypeExpr::Sum(left, right, _) => {
                self.contains_visiting(left, visited) || self.contains_visiting(right, visited)
            }
            TypeExpr::Forall(_, inner, _)
            | TypeExpr::Ptr(inner, _)
            | TypeExpr::Ref(inner, _)
            | TypeExpr::Paren(inner, _) => self.contains_visiting(inner, visited),
            _ => false,
        }
    }

    /// Whether some declaration of `function` returns a type that contains
    /// a handle — what makes a `let` of a call to it a handle variable.
    #[must_use]
    pub fn returns_handle(&self, function: &str) -> bool {
        self.functions.get(function).is_some_and(|signatures| {
            signatures.iter().any(|signature| {
                signature
                    .result
                    .as_ref()
                    .is_some_and(|ty| self.contains_handle(ty))
            })
        })
    }

    /// `function`'s D3 kind: its bare name is in the table and some declared
    /// signature of it mentions a handle. A same-named function over `String`
    /// has none.
    #[must_use]
    pub fn kind_of(&self, function: &str) -> Option<HandleFunctionKind> {
        let (_, kind) = FUNCTION_KINDS.iter().find(|(name, _)| *name == function)?;
        self.mentions_handle(function).then_some(*kind)
    }

    /// Whether some declared signature of `function` mentions a handle, in a
    /// parameter or its result — what E0095 refuses inside a region.
    #[must_use]
    pub fn mentions_handle(&self, function: &str) -> bool {
        self.functions.get(function).is_some_and(|signatures| {
            signatures.iter().any(|signature| {
                signature.params.iter().any(|ty| self.contains_handle(ty))
                    || signature
                        .result
                        .as_ref()
                        .is_some_and(|ty| self.contains_handle(ty))
            })
        })
    }
}

/// A declared type's constructor field types or record field types.
fn body_parts(body: &TypeBody) -> Vec<TypeExpr> {
    match body {
        TypeBody::Sum(variants) => variants
            .iter()
            .flat_map(|variant| variant.fields.iter().map(|field| field.ty.clone()))
            .collect(),
        TypeBody::Record(fields) => fields.iter().map(|field| field.ty.clone()).collect(),
    }
}

/// Every module of `tree`, in pre-order, with its file.
fn modules_of(tree: &ParsedModule) -> Vec<(&Path, &[Item])> {
    let mut modules = vec![(tree.path.as_path(), tree.source_file.items.as_slice())];
    for submodule in &tree.submodules {
        modules.extend(modules_of(submodule));
    }
    modules
}

/// The `B4.1` check over a whole program (D6): every function of every
/// module, against the declarations of all of them. It reads the parsed
/// tree, which every module has at admission — a module served from the
/// elaboration cache contributes no Core terms there.
#[must_use]
pub fn check_module_tree(tree: &ParsedModule) -> Vec<ElabError> {
    let modules = modules_of(tree);
    let declarations =
        HandleDeclarations::from_items(modules.iter().flat_map(|(_, items)| items.iter()));
    let mut errors = Vec::new();
    for (file, items) in modules {
        for item in items {
            let Item::Function(function) = item else {
                continue;
            };
            for finding in check_function(function, &declarations, UsePolicy::AFFINE) {
                errors.push(finding_error(&function.name.name, finding).with_file_path(file));
            }
        }
    }
    errors
}

/// A finding as the diagnostic it is reported as, at the offending use.
#[must_use]
pub fn finding_error(definition: &str, finding: HandleFinding) -> ElabError {
    let handle = finding.handle;
    let definition = definition.to_string();
    let kind = match finding.kind {
        HandleFindingKind::UsedAfterConsumed => {
            ElabErrorKind::Handle(HandleErrorKind::UsedAfterConsumed { handle, definition })
        }
        HandleFindingKind::CapturedByClosure => {
            ElabErrorKind::Handle(HandleErrorKind::CapturedByClosure { handle, definition })
        }
        HandleFindingKind::ReachedFromRegion => {
            ElabErrorKind::Region(RegionErrorKind::ReachesOutside {
                name: handle,
                reason: handle_reason(&definition),
            })
        }
    };
    ElabError::new(finding.span, kind)
}

/// Admission's verdict with the handle refusals added after its own errors.
pub fn with_handle_errors(
    admitted: Result<(), Vec<ElabError>>,
    handle_errors: Vec<ElabError>,
) -> Result<(), Vec<ElabError>> {
    let mut errors = admitted.err().unwrap_or_default();
    errors.extend(handle_errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
