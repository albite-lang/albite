//! `syn`-based extraction of what each function body does (ADR 5.10.26f D2,
//! D4, D5).
//!
//! Each crate is parsed whole (`walk`), so a brace or a call inside a
//! comment, a string literal or a signature is never a fact. Every `fn` the
//! parsed files define goes into an index keyed by module path and name, with
//! the raw [`Observation`]s of its body (`observe`); `reach` turns those into
//! facts.
//!
//! [`SourceFile::parse`] and [`CrateIndex::build`] are pure over source text,
//! so the extraction is tested over inline snippets.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use syn::visit::Visit;

use super::observe::{macro_name, thread_local_names, BodyObserver};

/// One parsed source file and where it sits in its crate.
pub(crate) struct SourceFile {
    /// The path as the recogniser spells it (`<root>/ffi/check.rs`).
    pub(crate) file: String,
    /// Module path inside the crate (`["ffi", "check"]`); empty for `lib.rs`.
    pub(crate) module: Vec<String>,
    /// The `cfg` inherited from the `mod` declaration, if any.
    pub(crate) cfg: Option<String>,
    pub(crate) ast: syn::File,
}

impl SourceFile {
    /// Parse `text`; a file that does not parse is an error naming it.
    #[cfg(test)]
    pub(crate) fn parse(file: &str, module: &[&str], text: &str) -> Result<SourceFile, String> {
        let ast = syn::parse_file(text).map_err(|error| format!("parsing {file}: {error}"))?;
        Ok(SourceFile {
            file: file.to_string(),
            module: module.iter().map(|&segment| segment.to_owned()).collect(),
            cfg: None,
            ast,
        })
    }
}

/// What one body was seen to do, before any call is resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Observation {
    /// A call through a path: `fs::read(p)`, `helper(x)`, `Self::new()`.
    Call(Vec<String>),
    /// A method call, by name: `x.get_ctx(h)`.
    Method(String),
    /// A macro, and whether its arguments parsed as expressions.
    Macro { name: String, readable: bool },
    /// A path used as a value: `ARENA`, `CLI_ARGS`.
    PathRef(Vec<String>),
}

/// One `fn` the parsed files define, or one a foreign block declares.
pub(crate) struct FnDef {
    pub(crate) name: String,
    pub(crate) crate_name: String,
    pub(crate) module: Vec<String>,
    /// The `impl` or `trait` it belongs to, by its last path segment.
    pub(crate) self_ty: Option<String>,
    pub(crate) file: String,
    pub(crate) cfg: Option<String>,
    /// Declared in an `extern "C" { … }` block: a call into the OS or libc.
    pub(crate) foreign: bool,
    /// `#[no_mangle] extern "C"`: an export.
    pub(crate) exported: bool,
    pub(crate) uses: Rc<UseMap>,
    pub(crate) observations: Vec<Observation>,
}

/// A file's `use` items: each imported name's full path, and glob prefixes.
#[derive(Default, Debug)]
pub(crate) struct UseMap {
    pub(crate) names: HashMap<String, Vec<String>>,
    pub(crate) globs: Vec<Vec<String>>,
}

/// Every function, `static` and constructor name the parsed crates define.
#[derive(Default)]
pub(crate) struct CrateIndex {
    pub(crate) fns: Vec<FnDef>,
    pub(crate) by_name: HashMap<String, Vec<usize>>,
    pub(crate) statics: HashSet<String>,
    pub(crate) constructors: HashSet<String>,
    pub(crate) modules: HashSet<Vec<String>>,
    pub(crate) crate_names: Vec<String>,
}

impl CrateIndex {
    /// Index each crate's files.
    pub(crate) fn build(crates: &[(&str, &[SourceFile])]) -> CrateIndex {
        let mut index = CrateIndex::default();
        for (crate_name, files) in crates {
            index.crate_names.push((*crate_name).to_string());
            for source in *files {
                let mut uses = UseMap::default();
                collect_uses(&source.ast.items, &mut uses);
                let scope = Scope {
                    crate_name,
                    module: source.module.clone(),
                    file: &source.file,
                    cfg: source.cfg.clone(),
                    uses: Rc::new(uses),
                };
                index.modules.insert(source.module.clone());
                index.add_items(&source.ast.items, &scope);
            }
        }
        for (id, def) in index.fns.iter().enumerate() {
            index.by_name.entry(def.name.clone()).or_default().push(id);
        }
        index
    }

    /// The export items named `symbol`.
    pub(crate) fn exports_named(&self, symbol: &str) -> Vec<usize> {
        self.by_name
            .get(symbol)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&id| self.fns[id].exported)
            .collect()
    }

    fn add_items(&mut self, items: &[syn::Item], scope: &Scope<'_>) {
        for item in items {
            self.add_item(item, scope);
        }
    }

    fn add_item(&mut self, item: &syn::Item, scope: &Scope<'_>) {
        match item {
            syn::Item::Fn(item) => {
                let exported = has_no_mangle(&item.attrs) && item.sig.abi.is_some();
                let def = scope.fn_def(&item.sig, &item.attrs, None, Some(&item.block));
                self.fns.push(FnDef { exported, ..def });
            }
            syn::Item::Impl(item) => {
                let self_ty = last_type_ident(&item.self_ty);
                for member in &item.items {
                    if let syn::ImplItem::Fn(method) = member {
                        let def = scope.fn_def(
                            &method.sig,
                            &method.attrs,
                            self_ty.clone(),
                            Some(&method.block),
                        );
                        self.fns.push(def);
                    }
                }
            }
            syn::Item::Trait(item) => self.add_trait(item, scope),
            syn::Item::ForeignMod(block) => {
                for member in &block.items {
                    if let syn::ForeignItem::Fn(foreign) = member {
                        let def = scope.fn_def(&foreign.sig, &foreign.attrs, None, None);
                        self.fns.push(FnDef {
                            foreign: true,
                            ..def
                        });
                    }
                }
            }
            syn::Item::Mod(module) => self.add_inline_module(module, scope),
            syn::Item::Static(item) => {
                self.statics.insert(item.ident.to_string());
            }
            syn::Item::Macro(item) if macro_name(&item.mac) == "thread_local" => {
                self.statics.extend(thread_local_names(&item.mac));
            }
            syn::Item::Enum(item) => {
                let variants = item
                    .variants
                    .iter()
                    .map(|variant| variant.ident.to_string());
                self.constructors.extend(variants);
            }
            syn::Item::Struct(item) => {
                self.constructors.insert(item.ident.to_string());
            }
            _ => {}
        }
    }

    fn add_trait(&mut self, item: &syn::ItemTrait, scope: &Scope<'_>) {
        let self_ty = Some(item.ident.to_string());
        for member in &item.items {
            if let syn::TraitItem::Fn(method) = member {
                let body = method.default.as_ref();
                let def = scope.fn_def(&method.sig, &method.attrs, self_ty.clone(), body);
                self.fns.push(def);
            }
        }
    }

    fn add_inline_module(&mut self, module: &syn::ItemMod, scope: &Scope<'_>) {
        let Some((_, items)) = &module.content else {
            return;
        };
        if is_test_only(&module.attrs) {
            return;
        }
        let mut path = scope.module.clone();
        path.push(module.ident.to_string());
        self.modules.insert(path.clone());
        let inner = Scope {
            module: path,
            cfg: cfg_of(&module.attrs).or_else(|| scope.cfg.clone()),
            ..scope.clone()
        };
        self.add_items(items, &inner);
    }
}

#[derive(Clone)]
struct Scope<'a> {
    crate_name: &'a str,
    module: Vec<String>,
    file: &'a str,
    cfg: Option<String>,
    uses: Rc<UseMap>,
}

impl Scope<'_> {
    fn fn_def(
        &self,
        sig: &syn::Signature,
        attrs: &[syn::Attribute],
        self_ty: Option<String>,
        body: Option<&syn::Block>,
    ) -> FnDef {
        let mut observer = BodyObserver::default();
        if let Some(block) = body {
            observer.visit_block(block);
        }
        FnDef {
            name: sig.ident.to_string(),
            crate_name: self.crate_name.to_string(),
            module: self.module.clone(),
            self_ty,
            file: self.file.to_string(),
            cfg: cfg_of(attrs).or_else(|| self.cfg.clone()),
            foreign: false,
            exported: false,
            uses: Rc::clone(&self.uses),
            observations: observer.observations,
        }
    }
}

fn last_type_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

fn has_no_mangle(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("no_mangle"))
}

/// The `#[cfg(…)]` predicate, whitespace removed: `not(unix)`.
pub(crate) fn cfg_of(attrs: &[syn::Attribute]) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident("cfg"))
        .and_then(|attr| {
            let list = attr.meta.require_list().ok()?;
            let text: String = list.tokens.to_string().split_whitespace().collect();
            Some(text)
        })
}

/// A `#[cfg(test)]` (or `all(test, …)`) item, never a `not(test)` one.
pub(crate) fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    cfg_of(attrs).is_some_and(|cfg| cfg.contains("test") && !cfg.contains("not(test"))
}

fn collect_uses(items: &[syn::Item], uses: &mut UseMap) {
    for item in items {
        if let syn::Item::Use(item) = item {
            flatten_use(&item.tree, &mut Vec::new(), uses);
        }
    }
}

fn flatten_use(tree: &syn::UseTree, prefix: &mut Vec<String>, uses: &mut UseMap) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use(&path.tree, prefix, uses);
            prefix.pop();
        }
        syn::UseTree::Name(name) if name.ident == "self" => {
            if let Some(last) = prefix.last() {
                uses.names.insert(last.clone(), prefix.clone());
            }
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix.clone();
            full.push(name.ident.to_string());
            uses.names.insert(name.ident.to_string(), full);
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix.clone();
            full.push(rename.ident.to_string());
            uses.names.insert(rename.rename.to_string(), full);
        }
        syn::UseTree::Glob(_) => uses.globs.push(prefix.clone()),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                flatten_use(tree, prefix, uses);
            }
        }
    }
}
