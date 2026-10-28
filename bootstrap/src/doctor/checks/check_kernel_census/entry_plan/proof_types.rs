//! What a parse can see of the proofs a file declares (ADR 14.10.26b): the
//! proof items, the return type of every non-`#[partial]` `fn`, and the
//! propositions each declared `type` reaches — read syntactically, and erring
//! wide, because a false negative hides a gated definition from the census.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Item, SortAnnot, TypeBody, TypeExpr};

/// What one type expression mentions (D1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mentions {
    /// Names a proposition directly: `Eq<…>`, `a == b`, `Void`.
    pub proposition: bool,
    /// Every type name it mentions, by its last path segment.
    pub names: BTreeSet<String>,
}

impl Mentions {
    fn merge(&mut self, other: &Mentions) {
        self.proposition |= other.proposition;
        self.names.extend(other.names.iter().cloned());
    }

    /// A proposition directly, or through a proof-carrying type.
    fn carries_proof(&self, carrying: &BTreeSet<String>) -> bool {
        self.proposition || !self.names.is_disjoint(carrying)
    }
}

/// One parsed file's contribution to the selection (D1, D2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProofFacts {
    /// Declares a `theorem`, `lemma` or `axiom`.
    pub declares_proof_item: bool,
    /// The return type of each non-`#[partial]` `fn` that declares one.
    pub fn_returns: Vec<Mentions>,
    /// Each declared `type`, by name, and what its body mentions.
    pub types: BTreeMap<String, Mentions>,
}

impl ProofFacts {
    /// Read the facts from a file's items.
    #[must_use]
    pub fn of_items(items: &[Item]) -> Self {
        let mut facts = ProofFacts::default();
        for item in items {
            match item {
                Item::Theorem(_) | Item::Lemma(_) | Item::Axiom(_) => {
                    facts.declares_proof_item = true;
                }
                Item::Function(function) if !function.attrs.partial => {
                    if let Some(return_type) = &function.return_type {
                        facts.fn_returns.push(mentions_of(return_type));
                    }
                }
                Item::TypeAlias(alias) => {
                    let mut body = mentions_of(&alias.ty);
                    // A statement family, `type Le(n: Nat, m: Nat): Prop`.
                    body.proposition |= alias.sort == Some(SortAnnot::Prop);
                    facts.declare_type(&alias.name.name, &body);
                }
                Item::TypeDef(def) => {
                    let mut body = Mentions::default();
                    for field_type in field_types(&def.body) {
                        walk(field_type, &mut body);
                    }
                    facts.declare_type(&def.name.name, &body);
                }
                _ => {}
            }
        }
        facts
    }

    /// A name declared twice in one tree keeps both bodies' mentions.
    fn declare_type(&mut self, name: &str, body: &Mentions) {
        self.types.entry(name.to_string()).or_default().merge(body);
    }

    /// Whether this file declares something the kernel gate checks, given
    /// the tree's proof-carrying type names.
    #[must_use]
    pub fn bears_proof(&self, carrying: &BTreeSet<String>) -> bool {
        self.declares_proof_item || self.fn_returns.iter().any(|r| r.carries_proof(carrying))
    }
}

/// The type names of one tree that reach a proposition, transitively (D2).
/// The set only grows and is bounded by the declared names, so a cycle —
/// `type Loop = Loop` — reaches its fixed point and stops.
#[must_use]
pub fn proof_carrying_types<'a>(
    files: impl IntoIterator<Item = &'a ProofFacts>,
) -> BTreeSet<String> {
    let mut types: BTreeMap<&str, Mentions> = BTreeMap::new();
    for facts in files {
        for (name, body) in &facts.types {
            types.entry(name.as_str()).or_default().merge(body);
        }
    }
    let mut carrying = BTreeSet::new();
    loop {
        let found: Vec<&str> = types
            .iter()
            .filter(|(name, body)| !carrying.contains(**name) && body.carries_proof(&carrying))
            .map(|(name, _)| *name)
            .collect();
        if found.is_empty() {
            return carrying;
        }
        carrying.extend(found.into_iter().map(str::to_string));
    }
}

/// Every field type of an ADT's variants, or of a record.
fn field_types(body: &TypeBody) -> Vec<&TypeExpr> {
    match body {
        TypeBody::Sum(variants) => variants
            .iter()
            .flat_map(|v| v.fields.iter().map(|f| &f.ty))
            .collect(),
        TypeBody::Record(fields) => fields.iter().map(|f| &f.ty).collect(),
    }
}

fn mentions_of(type_expr: &TypeExpr) -> Mentions {
    let mut mentions = Mentions::default();
    walk(type_expr, &mut mentions);
    mentions
}

/// D1's walk: every position a value of the type can hold a proof in. A
/// function type's domain is not one — taking an `Eq` is not returning one.
fn walk(type_expr: &TypeExpr, found: &mut Mentions) {
    match type_expr {
        TypeExpr::Eq(..) | TypeExpr::EqExplicit(..) | TypeExpr::Void(_) => found.proposition = true,
        TypeExpr::Path(path) => {
            found.names.insert(path.item_name().name.clone());
        }
        TypeExpr::App(head, args, _) => {
            walk(head, found);
            for arg in args {
                walk(arg, found);
            }
        }
        TypeExpr::Product(left, right, _)
        | TypeExpr::Sum(left, right, _)
        | TypeExpr::Exists(_, left, right, _) => {
            walk(left, found);
            walk(right, found);
        }
        TypeExpr::Arrow(_, codomain, _) | TypeExpr::Pi(_, _, codomain, _) => walk(codomain, found),
        TypeExpr::Forall(_, inner, _)
        | TypeExpr::Paren(inner, _)
        | TypeExpr::Ptr(inner, _)
        | TypeExpr::Ref(inner, _) => walk(inner, found),
        TypeExpr::Prop(_) | TypeExpr::Unit(_) | TypeExpr::Error(_) | TypeExpr::Term(_) => {}
    }
}
