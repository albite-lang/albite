//! Reach: an export's facts, followed through the helpers it calls in the
//! same crates (ADR 5.10.26f D5).
//!
//! 1.10.26k's D2 classes a body "by its worst path", so a call to a function
//! defined in `albite_core` or `albite_runtime` merges that function's
//! facts into the caller's, marked with the call it went through, and cycles
//! are cut. A path call that resolves to more than one definition, or to none
//! in these crates and none in `std`/`core`/`alloc`, is unreadable. A method
//! call cannot be resolved without types: it is followed when exactly one
//! non-kernel `impl` defines that name, is a kernel fact when only the kernel
//! arena does, and is otherwise `std`'s — a filesystem query among them is
//! still an OS fact.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::extract::{CrateIndex, FnDef, Observation};
use super::{
    is_os_crate, last_two, os_fact_for_method, os_fact_for_std_call, Fact, Facts,
    ALLOCATOR_ENTRIES, ALLOCATOR_MODULE, CONSOLE_MACROS, KERNEL_MACROS, KERNEL_MODULES,
    KERNEL_STATICS, KERNEL_TYPES,
};

const STD_ROOTS: [&str; 3] = ["std", "core", "alloc"];

/// Primitive types: `u64::from(x)` is a call into `core`.
const PRIMITIVES: [&str; 17] = [
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64", "bool", "char", "str",
];

/// What a path call names once its imports are expanded.
#[derive(Debug, PartialEq, Eq)]
enum Callee {
    /// `std`, `core`, `alloc`, a prelude name, a constructor or a local.
    Std(Vec<String>),
    /// Functions these crates define (`cfg` twins count as one).
    Defined(Vec<usize>),
    /// A crate outside the workspace that the rule names as the OS.
    OsCrate(String),
    /// Nothing the rule can follow.
    Unresolved(String),
}

/// Facts by function, memoised; a function still being walked is a cycle.
pub(crate) struct Reach<'a> {
    index: &'a CrateIndex,
    done: RefCell<HashMap<usize, Facts>>,
    walking: RefCell<HashSet<usize>>,
}

impl<'a> Reach<'a> {
    pub(crate) fn new(index: &'a CrateIndex) -> Reach<'a> {
        Reach {
            index,
            done: RefCell::default(),
            walking: RefCell::default(),
        }
    }

    /// The facts of export `symbol`: one item, or a complementary `cfg`
    /// pair merged. Anything else is an error naming the symbol.
    pub(crate) fn export_facts(&self, symbol: &str) -> Result<Facts, String> {
        let items = self.index.exports_named(symbol);
        match items.as_slice() {
            [] => Err(format!(
                "`{symbol}`: no `extern \"C\" fn` item has that name"
            )),
            [one] => Ok(self.facts_of(*one)),
            [a, b] if complementary(&self.index.fns[*a], &self.index.fns[*b]) => {
                let mut facts = self.facts_of(*a);
                facts.absorb(self.facts_of(*b));
                Ok(facts)
            }
            many => Err(format!(
                "`{symbol}`: {} items, not a complementary `cfg` pair",
                many.len()
            )),
        }
    }

    /// The files the items of `symbol` sit in.
    pub(crate) fn export_files(&self, symbol: &str) -> Vec<&'a str> {
        let index = self.index;
        let items = index.exports_named(symbol);
        items
            .iter()
            .map(|&id| index.fns[id].file.as_str())
            .collect()
    }

    fn facts_of(&self, id: usize) -> Facts {
        if let Some(known) = self.done.borrow().get(&id) {
            return known.clone();
        }
        self.walking.borrow_mut().insert(id);
        let def = &self.index.fns[id];
        let mut facts = Facts::default();
        for observation in &def.observations {
            self.observe(def, observation, &mut facts);
        }
        self.walking.borrow_mut().remove(&id);
        self.done.borrow_mut().insert(id, facts.clone());
        facts
    }

    fn observe(&self, def: &FnDef, observation: &Observation, facts: &mut Facts) {
        match observation {
            Observation::Call(path) => self.observe_call(self.resolve(def, path), facts),
            Observation::Method(name) => {
                let methods = self.methods_named(name);
                if methods.is_empty() && self.is_kernel_arena_method(name) {
                    facts.kernel.push(Fact::direct(name.clone()));
                } else if methods.is_empty() {
                    facts.runtime.extend(os_fact_for_method(name));
                } else if one_definition(&self.index.fns, &methods) {
                    self.follow(&methods, facts);
                }
            }
            Observation::Macro { name, readable } => {
                if KERNEL_MACROS.contains(&name.as_str()) {
                    facts.kernel.push(Fact::direct(name.clone()));
                } else if CONSOLE_MACROS.contains(&name.as_str()) {
                    facts.runtime.push(Fact::direct(format!("{name}!")));
                } else if !readable {
                    facts.unreadable.push(Fact::direct(format!("{name}!")));
                }
            }
            Observation::PathRef(path) => {
                let Some(name) = path.last() else { return };
                if !self.index.statics.contains(name) {
                    return;
                }
                if KERNEL_STATICS.contains(&name.as_str()) {
                    facts.kernel.push(Fact::direct(name.clone()));
                } else {
                    facts.runtime.push(Fact::direct(name.clone()));
                }
            }
        }
    }

    fn observe_call(&self, callee: Callee, facts: &mut Facts) {
        match callee {
            Callee::Std(rest) => facts.runtime.extend(os_fact_for_std_call(&rest)),
            Callee::Defined(ids) => self.follow(&ids, facts),
            Callee::OsCrate(token) => facts.runtime.push(Fact::direct(token)),
            Callee::Unresolved(token) => facts.unreadable.push(Fact::direct(token)),
        }
    }

    /// Merge each callee's facts: a kernel module is a fact and a stop, the
    /// allocator is neither, a foreign function is the OS, anything else is
    /// walked.
    fn follow(&self, ids: &[usize], facts: &mut Facts) {
        for &id in ids {
            let callee = &self.index.fns[id];
            if is_allocator(callee) {
                // No fact, and not followed: 1.10.26k's D2.
            } else if is_kernel_module(callee) {
                facts.kernel.push(Fact::direct(callee.name.clone()));
            } else if callee.foreign {
                facts.runtime.push(Fact::direct(callee.name.clone()));
            } else if !self.walking.borrow().contains(&id) {
                let reached = self.facts_of(id);
                facts.absorb_through(&reached, &callee.name);
            }
        }
    }

    fn resolve(&self, def: &FnDef, path: &[String]) -> Callee {
        let absolute = self.absolutize(def, path);
        let (Some(root), Some(name)) = (absolute.first(), absolute.last()) else {
            return Callee::Std(Vec::new());
        };
        if STD_ROOTS.contains(&root.as_str()) {
            return Callee::Std(absolute[1..].to_vec());
        }
        if self.index.crate_names.contains(root) {
            return self.resolve_in_crate(&absolute);
        }
        if absolute.len() == 1 {
            return self.resolve_unqualified(def, name);
        }
        if is_os_crate(root) {
            return Callee::OsCrate(last_two(&absolute));
        }
        if starts_uppercase(root) || PRIMITIVES.contains(&root.as_str()) {
            // A prelude or primitive type (`Vec::new`, `String::from`, `u64::from`).
            return Callee::Std(absolute);
        }
        Callee::Unresolved(absolute.join("::"))
    }

    /// `[crate, m1, …, name]`: the definitions whose module (and `impl`)
    /// path matches, else any of that name in the crate — a re-export.
    fn resolve_in_crate(&self, absolute: &[String]) -> Callee {
        let (crate_name, name) = (&absolute[0], &absolute[absolute.len() - 1]);
        let qualifiers = &absolute[1..absolute.len() - 1];
        let in_crate: Vec<usize> = self
            .candidates(name)
            .filter(|&id| &self.index.fns[id].crate_name == crate_name)
            .collect();
        if let Some(self_ty) = qualifiers
            .last()
            .filter(|segment| starts_uppercase(segment))
        {
            return self.resolve_associated(&in_crate, self_ty);
        }
        let exact: Vec<usize> = in_crate
            .iter()
            .copied()
            .filter(|&id| owner_path(&self.index.fns[id]).ends_with(qualifiers))
            .collect();
        let chosen = if exact.is_empty() { in_crate } else { exact };
        if chosen.is_empty() {
            return if self.index.constructors.contains(name) {
                Callee::Std(Vec::new())
            } else {
                Callee::Unresolved(absolute.join("::"))
            };
        }
        if one_definition(&self.index.fns, &chosen) {
            Callee::Defined(chosen)
        } else {
            Callee::Unresolved(format!(
                "{} ({} definitions)",
                absolute.join("::"),
                chosen.len()
            ))
        }
    }

    /// `Type::name`: the `impl Type` functions of that name, wherever the
    /// `impl` block sits. None is a derived or `std` trait method
    /// (`Type::default`) or an enum variant (`TermNode::App`): no fact.
    fn resolve_associated(&self, in_crate: &[usize], self_ty: &str) -> Callee {
        let owned: Vec<usize> = in_crate
            .iter()
            .copied()
            .filter(|&id| self.index.fns[id].self_ty.as_deref() == Some(self_ty))
            .collect();
        if owned.is_empty() {
            Callee::Std(Vec::new())
        } else {
            Callee::Defined(owned)
        }
    }

    /// A bare name: this module's own function, else one a glob brings in,
    /// else a prelude name, a constructor or a local closure — no fact.
    fn resolve_unqualified(&self, def: &FnDef, name: &str) -> Callee {
        let mut scopes = vec![def.module.clone()];
        scopes.extend(def.uses.globs.iter().map(|glob| self.absolutize(def, glob)));
        for scope in scopes {
            let found: Vec<usize> = self
                .candidates(name)
                .filter(|&id| {
                    let candidate = &self.index.fns[id];
                    candidate.self_ty.is_none()
                        && candidate.crate_name == def.crate_name
                        && (candidate.module == scope
                            || prefixed(&candidate.crate_name, &candidate.module) == scope)
                })
                .collect();
            if !found.is_empty() && one_definition(&self.index.fns, &found) {
                return Callee::Defined(found);
            }
        }
        Callee::Std(Vec::new())
    }

    /// The methods a `.name()` call could reach outside the kernel. A
    /// receiver's type is not known, so a checker or evaluator method is
    /// never inferred from a name alone: a kernel body is decided by the
    /// arena it opens.
    fn methods_named(&self, name: &str) -> Vec<usize> {
        self.candidates(name)
            .filter(|&id| {
                let def = &self.index.fns[id];
                def.self_ty.is_some() && !is_kernel_module(def)
            })
            .collect()
    }

    /// `.alloc_ctx()`: a name only the kernel arena defines. With no other
    /// crate method of that name, the receiver is the arena.
    fn is_kernel_arena_method(&self, name: &str) -> bool {
        self.candidates(name).any(|id| {
            let def = &self.index.fns[id];
            def.crate_name == "albite_core"
                && def
                    .self_ty
                    .as_deref()
                    .is_some_and(|ty| KERNEL_TYPES.contains(&ty))
        })
    }

    fn candidates(&self, name: &str) -> impl Iterator<Item = usize> + '_ {
        self.index.by_name.get(name).into_iter().flatten().copied()
    }

    /// Expand an import and `crate`/`self`/`super`/`Self` into a path rooted
    /// at a crate name; a path rooted elsewhere is returned as it stands.
    fn absolutize(&self, def: &FnDef, path: &[String]) -> Vec<String> {
        let mut path = path.to_vec();
        if let Some(imported) = path.first().and_then(|head| def.uses.names.get(head)) {
            path.splice(0..1, imported.iter().cloned());
        }
        match self.path_in_crate(def, &path) {
            Some(rest) => prefixed(&def.crate_name, &rest),
            None => path,
        }
    }

    /// The crate-relative path `path` names from inside `def`'s module, or
    /// `None` when it is rooted outside the crate.
    fn path_in_crate(&self, def: &FnDef, path: &[String]) -> Option<Vec<String>> {
        let head = path.first()?;
        let mut module = def.module.clone();
        let tail = |from: usize| path[from..].iter().cloned();
        match head.as_str() {
            "crate" => Some(tail(1).collect()),
            "self" => Some(module.into_iter().chain(tail(1)).collect()),
            "super" => {
                let supers = path
                    .iter()
                    .take_while(|segment| *segment == "super")
                    .count();
                module.truncate(module.len().saturating_sub(supers));
                Some(module.into_iter().chain(tail(supers)).collect())
            }
            "Self" => {
                let self_ty = def.self_ty.clone()?;
                Some(module.into_iter().chain([self_ty]).chain(tail(1)).collect())
            }
            _ => {
                let mut relative = module.clone();
                relative.push(head.clone());
                self.index
                    .modules
                    .contains(&relative)
                    .then(|| module.into_iter().chain(tail(0)).collect())
            }
        }
    }
}

fn prefixed(crate_name: &str, rest: &[String]) -> Vec<String> {
    std::iter::once(crate_name.to_string())
        .chain(rest.iter().cloned())
        .collect()
}

/// A definition's module path and, for a method, its `impl` type.
fn owner_path(def: &FnDef) -> Vec<String> {
    def.module
        .iter()
        .cloned()
        .chain(def.self_ty.clone())
        .collect()
}

/// All in one module and `impl`: one function, perhaps in `cfg` arms.
fn one_definition(fns: &[FnDef], ids: &[usize]) -> bool {
    let first = owner_path(&fns[ids[0]]);
    ids.iter().all(|&id| owner_path(&fns[id]) == first)
}

/// The checker, conversion, the evaluator, or a method of the kernel arena.
fn is_kernel_module(def: &FnDef) -> bool {
    let in_kernel_module = def
        .module
        .first()
        .is_some_and(|top| KERNEL_MODULES.contains(&top.as_str()));
    let on_kernel_type = def
        .self_ty
        .as_deref()
        .is_some_and(|self_ty| KERNEL_TYPES.contains(&self_ty));
    def.crate_name == "albite_core" && (in_kernel_module || on_kernel_type)
}

fn is_allocator(def: &FnDef) -> bool {
    def.module
        .last()
        .is_some_and(|module| module == ALLOCATOR_MODULE)
        || (ALLOCATOR_ENTRIES.contains(&def.name.as_str()) && def.crate_name == "albite_runtime")
}

/// Two arms of one export: one's `cfg` is the other's negation.
fn complementary(a: &FnDef, b: &FnDef) -> bool {
    match (&a.cfg, &b.cfg) {
        (Some(x), Some(y)) => *y == format!("not({x})") || *x == format!("not({y})"),
        _ => false,
    }
}

fn starts_uppercase(segment: &str) -> bool {
    segment.chars().next().is_some_and(char::is_uppercase)
}
