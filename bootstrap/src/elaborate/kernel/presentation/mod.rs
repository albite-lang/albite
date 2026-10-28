//! Records reach the kernel by their encoding (ADR 3.10.26p D1–D3).
//!
//! Elaboration spells a record type as a bare name, `TyVar("Pt")`, which mono
//! discovery and codegen read as a concrete type (`concrete_type_names`). The
//! kernel has no type table, so the same name is a malformed type to it. A sum
//! already arrives as its structural encoding; a record does not, so this
//! module swaps each record name for its encoding — `Pt = { x: Nat, y: Nat }`
//! becomes `(Nat × Nat)` — just before the kernel runs, and nowhere else.
//!
//! Exact or not at all (D3). A record is left named when its encoding is not
//! closed over record names — a free `α_` marker (a non-entry member of a
//! mutual cluster, whose μ wraps only the entry), a type parameter, or no
//! encoding at all (a generic record) — and when it lies on a cycle of record
//! references, which an encoding spells by name rather than under a μ. An
//! unpresented name stays a `TyVar`, so its census row stays
//! `named-type-reference`.
//!
//! Keys are exact (D2): no `@` stripping and no `Display` round trip, since
//! `TyVar("@Pt")` and `TyVar("Pt")` print alike and are different keys.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use albite_core::{Term, Type};

use crate::elaborate::CoreDef;

#[cfg(test)]
mod tests;

/// Each record whose encoding is closed over record names: the encoding and
/// the records it mentions.
type Closed<'r> = BTreeMap<&'r str, (Type, BTreeSet<String>)>;

/// Why a record is left named (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unpresented {
    /// No encoding is held: a generic record, whose instances are already
    /// structural by the time they are used.
    NoEncoding,
    /// The encoding has a free `α_` marker: a non-entry cluster member.
    FreeMarker,
    /// The encoding names a free type variable that is no record — a type
    /// parameter, or a name this module does not present.
    OpenEncoding,
    /// The record reaches itself through record names.
    Cycle,
}

/// Every record's presentation, computed once per discharge (D2's memo) and
/// cloned at each use. Built eagerly: a corpus has at most a few hundred
/// records, and a value computed up front cannot depend on which obligation
/// asked first, so the census and the gate present alike.
#[derive(Debug, Clone, Default)]
pub struct RecordPresentation {
    presented: BTreeMap<String, Type>,
    unpresented: BTreeMap<String, Unpresented>,
}

impl RecordPresentation {
    /// Presents nothing: a path that holds no type metadata.
    pub const EMPTY: RecordPresentation = RecordPresentation {
        presented: BTreeMap::new(),
        unpresented: BTreeMap::new(),
    };

    /// The presentation of every record in `records` from `encoded_types`
    /// (Encoding Finalization's map; only record keys are read, D1).
    #[must_use]
    #[allow(clippy::implicit_hasher)] // Reason: every caller holds the elaborator's std `HashMap`
    pub fn new<'r>(
        encoded_types: &HashMap<String, Type>,
        records: impl IntoIterator<Item = &'r String>,
    ) -> Self {
        let records: BTreeSet<&str> = records.into_iter().map(String::as_str).collect();
        let mut unpresented = BTreeMap::new();
        let mut closed: Closed<'_> = BTreeMap::new();
        for &name in &records {
            match closure_of(encoded_types.get(name), &records) {
                Ok(entry) => {
                    closed.insert(name, entry);
                }
                Err(reason) => {
                    unpresented.insert(name.to_string(), reason);
                }
            }
        }
        for &name in closed.keys() {
            if reaches_itself(name, &closed) {
                unpresented.insert(name.to_string(), Unpresented::Cycle);
            }
        }
        let mut presentation = RecordPresentation {
            presented: BTreeMap::new(),
            unpresented,
        };
        for &name in closed.keys() {
            presentation.present_record(name, &closed);
        }
        presentation
    }

    /// `name`'s presentation, when it has one.
    #[must_use]
    pub fn presented(&self, name: &str) -> Option<&Type> {
        self.presented.get(name)
    }

    /// Why `name` is left named, when it is a record that is.
    #[must_use]
    pub fn unpresented(&self, name: &str) -> Option<Unpresented> {
        self.unpresented.get(name).copied()
    }

    /// How many records are left named, by reason — D4's measurement.
    #[cfg(test)]
    #[must_use]
    pub fn unpresented_counts(&self) -> BTreeMap<Unpresented, usize> {
        let mut counts = BTreeMap::new();
        for reason in self.unpresented.values() {
            *counts.entry(*reason).or_insert(0) += 1;
        }
        counts
    }

    /// How many records are presented — D4's measurement.
    #[cfg(test)]
    #[must_use]
    pub fn presented_count(&self) -> usize {
        self.presented.len()
    }

    /// Whether this presentation replaces no name at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.presented.is_empty()
    }

    /// `ty` with every free, presentable record name replaced by its encoding.
    /// A binder spelled like a record shadows it (`Type::substitute`).
    #[must_use]
    pub fn present_type(&self, ty: &Type) -> Type {
        let mut names = BTreeSet::new();
        collect_type_names(ty, &mut names);
        self.presentable(&names)
            .fold(ty.clone(), |ty, (name, encoding)| {
                ty.substitute(name, encoding)
            })
    }

    /// `term` with every type it embeds presented.
    #[must_use]
    pub fn present_term(&self, term: &Term) -> Term {
        let mut names = BTreeSet::new();
        collect_term_names(term, &mut names);
        self.presentable(&names)
            .fold(term.clone(), |term, (name, encoding)| {
                term.substitute_type(name, encoding)
            })
    }

    /// An elaborated type as the kernel sees it: in the elaboration
    /// boundary's spelling, then presented. The build's gate runs before that
    /// boundary, so its definitions still say `@Pt` where the census, which
    /// reads `ProjectOutput`, says `Pt` (P0.2); this applies the boundary's own
    /// rule (ADR 10.5.26d P7) so both present the same names. The key match
    /// itself stays exact.
    #[must_use]
    pub fn present_elaborated_type(&self, ty: &Type) -> Type {
        self.present_type(&ty.strip_tyvar_at_prefix())
    }

    /// [`Self::present_elaborated_type`] for every type a term embeds.
    #[must_use]
    pub fn present_elaborated_term(&self, term: &Term) -> Term {
        let collection_names: HashMap<String, Type> = term
            .free_type_vars()
            .into_iter()
            .filter_map(|var| {
                let name = var.strip_prefix('@')?.to_string();
                Some((var, Type::TyVar(name)))
            })
            .collect();
        self.present_term(&term.substitute_type_vars(&collection_names))
    }

    /// `def` with its type and its body presented, in the boundary's
    /// spelling.
    #[must_use]
    pub fn present_def(&self, def: &CoreDef) -> CoreDef {
        let mut presented = def.clone();
        presented.ty = self.present_elaborated_type(&def.ty);
        presented.term.term = self.present_elaborated_term(&def.term.term);
        presented
    }

    /// The presentable names among `names`, with their encodings. A
    /// presented encoding mentions no presentable name, so substituting them
    /// one after another never rewrites inside an earlier replacement.
    fn presentable<'s>(
        &'s self,
        names: &'s BTreeSet<String>,
    ) -> impl Iterator<Item = (&'s str, &'s Type)> + 's {
        names.iter().filter_map(|name| {
            self.presented
                .get(name)
                .map(|encoding| (name.as_str(), encoding))
        })
    }

    /// Present `name` and, first, every presentable record it mentions.
    /// Terminates: a record on a cycle is never presented, so the records
    /// reached from a presented one form a DAG.
    fn present_record(&mut self, name: &str, closed: &Closed<'_>) {
        if self.presented.contains_key(name) || self.unpresented.contains_key(name) {
            return;
        }
        let (encoding, mentions) = &closed[name];
        let mut ty = encoding.clone();
        for mention in mentions {
            self.present_record(mention, closed);
            if let Some(inner) = self.presented.get(mention.as_str()) {
                ty = ty.substitute(mention, inner);
            }
        }
        self.presented.insert(name.to_string(), ty);
    }
}

/// `encoding`, in the boundary's spelling, and the records it mentions free,
/// or why it cannot be presented at all. An encoding finalised while a
/// sibling was still being collected names it `@Pt` (P0.2), so it is read in
/// the spelling the definitions are.
fn closure_of(
    encoding: Option<&Type>,
    records: &BTreeSet<&str>,
) -> Result<(Type, BTreeSet<String>), Unpresented> {
    let encoding = encoding
        .ok_or(Unpresented::NoEncoding)?
        .strip_tyvar_at_prefix();
    let free: BTreeSet<String> = encoding.free_type_vars().into_iter().collect();
    if free.iter().any(|var| var.starts_with("α_")) {
        return Err(Unpresented::FreeMarker);
    }
    if free.iter().any(|var| !records.contains(var.as_str())) {
        return Err(Unpresented::OpenEncoding);
    }
    // A record naming itself keeps the mention: it is a one-step cycle.
    Ok((encoding, free))
}

/// Whether `start` is reachable from the records it mentions, walking only
/// records whose encodings are closed.
fn reaches_itself(start: &str, closed: &Closed<'_>) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<&str> = closed[start].1.iter().map(String::as_str).collect();
    while let Some(name) = stack.pop() {
        if name == start {
            return true;
        }
        if !seen.insert(name) {
            continue;
        }
        if let Some((_, mentions)) = closed.get(name) {
            stack.extend(mentions.iter().map(String::as_str));
        }
    }
    false
}

/// Every name a `TyVar` in `ty` spells, including inside an `Eq`'s terms. An
/// over-approximation (bound names too) is harmless: substitution respects
/// binders.
fn collect_type_names(ty: &Type, names: &mut BTreeSet<String>) {
    match ty {
        Type::TyVar(name) => {
            names.insert(name.clone());
        }
        Type::Eq(_, left, right) => {
            collect_term_names(left, names);
            collect_term_names(right, names);
        }
        _ => {}
    }
    for child in ty.children() {
        collect_type_names(child, names);
    }
}

fn collect_term_names(term: &Term, names: &mut BTreeSet<String>) {
    term.for_each_embedded_type(|ty| collect_type_names(ty, names));
    term.for_each_subterm(|child| collect_term_names(child, names));
}
