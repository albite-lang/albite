//! δ in the kernel (ADR 2.10.26l D3, D4): which definitions the proof
//! judgment may unfold, decided by the kernel itself.
//!
//! A definition's body is admitted only if all three hold:
//!
//! 1. the caller's termination report says `Total` — trusted, and the only
//!    thing that keeps `#[partial]` and tainted bodies out (D3.1);
//! 2. it performs no effect — no `return`, reference cell or foreign call, so
//!    even an extern the elaborator treats as pure stays neutral (D3.2);
//! 3. the kernel types it with [`check_proof`] against its declared type, in
//!    a context holding only bodies already admitted (D3.3).
//!
//! The kernel classifies recursion itself, over the mention graph of the
//! closure it was asked about (D4): a definition that mentions only itself is
//! presented as `fix f'. body[name := f']`, with `f'` fresh, typed with no
//! binding for `name` and re-checked by `fix_admission`, whose certified
//! decreasing position guards conversion's unrolling; a member of a larger
//! strongly connected component enters as a self-bound global on the
//! report's `Total` marker, which is therefore trusted for it.
//!
//! An [`AdmittedBody`] can be made only here, and a [`Context`] accepts a body
//! only as one (`Context::extended`, crate-private), so no caller can hand
//! the kernel a body it has not admitted.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::context::{Binding, Context, GlobalAdmission};
use crate::conversion::{unfoldable_by_name, Unfoldings};
use crate::terms::substitution::capture::fresh_name;
use crate::terms::termination::{is_recursive, tarjan_scc, Adjacency};
use crate::terms::Term;
use crate::types::Type;

use super::gates::{fix_admission, term_mentions_global};
use super::rules::check_proof;

/// A body the kernel admitted, with a self-recursive one's certified
/// decreasing position.
///
/// It has no public constructor, and a context takes a body only as one of
/// these through a crate-private method, so code outside `albite_core`
/// cannot attach a body to a context:
///
/// ```compile_fail
/// use albite_core::context::Binding;
/// use albite_core::typecheck::AdmittedBody;
/// use albite_core::{Context, Type};
/// fn attach(body: AdmittedBody) -> Context {
///     Context::new().extended([Binding::Body("two".into(), Type::Nat, body)])
/// }
/// ```
///
/// ```compile_fail
/// use albite_core::typecheck::AdmittedBody;
/// use albite_core::Term;
/// let forged = AdmittedBody { term: Term::Zero, decreasing: None };
/// ```
///
/// Inside the crate the guarantee is module privacy: only this module builds
/// one, and only after the three checks above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedBody {
    /// Shared: every theorem whose closure reaches the body binds it, so a
    /// clone must not copy the term.
    term: Arc<Term>,
    decreasing: Option<usize>,
}

impl AdmittedBody {
    /// The term δ unfolds the global to: the body, or its `fix`.
    #[must_use]
    pub fn term(&self) -> &Term {
        &self.term
    }

    /// The certified decreasing term-parameter position of a `fix`.
    #[must_use]
    pub fn decreasing(&self) -> Option<usize> {
        self.decreasing
    }
}

/// One definition the kernel may be asked to unfold, as the caller hands it.
#[derive(Debug, Clone, Copy)]
pub struct DeltaCandidate<'a> {
    /// Its declared type.
    pub ty: &'a Type,
    /// Its elaborated body.
    pub body: &'a Term,
    /// Whether the caller's termination report says `Total` (D3.1, trusted).
    pub total: bool,
    /// Its `#[decreasing(arg)]`, a hint `fix_admission` re-checks (D4).
    pub decreasing: Option<&'a str>,
}

/// Every body the kernel admitted over a program, owned: what conversion
/// unfolds when a caller compares outside a context (the census's probe).
#[derive(Debug, Clone, Default)]
pub struct AdmittedUnfoldings {
    bodies: HashMap<String, AdmittedBody>,
}

impl AdmittedUnfoldings {
    /// The admitted names, sorted.
    #[must_use]
    pub fn names(&self) -> BTreeSet<&str> {
        self.bodies.keys().map(String::as_str).collect()
    }
}

impl Unfoldings for AdmittedUnfoldings {
    fn body(&self, name: &str) -> Option<&Term> {
        self.bodies.get(name).map(AdmittedBody::term)
    }

    fn decreasing(&self, name: &str) -> Option<usize> {
        self.bodies.get(name).and_then(AdmittedBody::decreasing)
    }
}

/// The kernel's δ admission over one program: candidates by name, and each
/// verdict once decided — admission depends only on a definition and the
/// closure below it, so it is the same for every theorem that reaches it.
pub struct DeltaAdmission<'a> {
    candidates: BTreeMap<String, DeltaCandidate<'a>>,
    /// Each candidate's mentioned names, computed once: the closure is asked
    /// for per theorem, and a body walk per ask would repeat for every one.
    mentions: HashMap<String, BTreeSet<String>>,
    decided: HashMap<String, Option<AdmittedBody>>,
}

impl<'a> DeltaAdmission<'a> {
    /// Admission over `candidates`, nothing decided yet.
    pub fn new(candidates: impl IntoIterator<Item = (String, DeltaCandidate<'a>)>) -> Self {
        DeltaAdmission {
            candidates: candidates.into_iter().collect(),
            mentions: HashMap::new(),
            decided: HashMap::new(),
        }
    }

    /// `base` extended with the admitted body of every definition in the
    /// mention closure of `roots` (D3.4). A later binding shadows, so each
    /// body overrides `base`'s plain marker for the same name.
    pub fn bind(&mut self, base: &Context, roots: &BTreeSet<String>) -> Context {
        let closure = self.closure(roots);
        self.decide(&closure);
        base.extended(closure.iter().filter_map(|name| {
            let body = self.decided.get(name)?.as_ref()?;
            Some(Binding::Body(
                name.clone(),
                self.candidates[name].ty.clone(),
                body.clone(),
            ))
        }))
    }

    /// Decide every candidate, and return what was admitted.
    pub fn admit_all(&mut self) -> AdmittedUnfoldings {
        let all: BTreeSet<String> = self.candidates.keys().cloned().collect();
        let closure = self.closure(&all);
        self.decide(&closure);
        AdmittedUnfoldings {
            bodies: self
                .decided
                .iter()
                .filter_map(|(name, body)| Some((name.clone(), body.clone()?)))
                .collect(),
        }
    }

    /// D3.1 and D3.2: the report says `Total` and the body is effect-free.
    fn admissible(&self, name: &str) -> bool {
        self.candidates
            .get(name)
            .is_some_and(|candidate| candidate.total && unfoldable_by_name(candidate.body))
    }

    /// Every name candidate `name`'s body and type mention, memoised.
    fn mentions_of(&mut self, name: &str) -> &BTreeSet<String> {
        if !self.mentions.contains_key(name) {
            let candidate = self.candidates[name];
            let found = mentioned_names(candidate.body, candidate.ty);
            self.mentions.insert(name.to_string(), found);
        }
        &self.mentions[name]
    }

    /// The admissible names candidate `name`'s body and type mention.
    fn admissible_mentions(&mut self, name: &str) -> BTreeSet<String> {
        let mentioned = self.mentions_of(name).clone();
        mentioned
            .into_iter()
            .filter(|mentioned| self.admissible(mentioned))
            .collect()
    }

    /// The admissible definitions reachable from `roots` through mentions.
    fn closure(&mut self, roots: &BTreeSet<String>) -> BTreeSet<String> {
        let mut closure = BTreeSet::new();
        let mut pending: Vec<String> = roots
            .iter()
            .filter(|root| self.admissible(root))
            .cloned()
            .collect();
        while let Some(name) = pending.pop() {
            if closure.insert(name.clone()) {
                pending.extend(self.admissible_mentions(&name));
            }
        }
        closure
    }

    /// Decide every undecided name in `closure`, dependencies first: Tarjan
    /// returns components leaves-first (`terms::termination::graph::scc`).
    fn decide(&mut self, closure: &BTreeSet<String>) {
        let undecided: Vec<&String> = closure
            .iter()
            .filter(|name| !self.decided.contains_key(name.as_str()))
            .collect();
        let graph: Adjacency = undecided
            .into_iter()
            .map(|name| (name.clone(), self.admissible_mentions(name)))
            .collect();
        for component in tarjan_scc(&graph) {
            let self_recursive = component.len() == 1 && is_recursive(&component, &graph);
            // A component's verdicts are recorded together, so a member never
            // sees a sibling's body while it is checked (D4).
            let verdicts: Vec<(String, Option<AdmittedBody>)> = component
                .iter()
                .map(|name| (name.clone(), self.admit_body(name, self_recursive)))
                .collect();
            self.decided.extend(verdicts);
        }
    }

    /// D3.3 and D4: the kernel's own check of one admissible body.
    fn admit_body(&self, name: &str, self_recursive: bool) -> Option<AdmittedBody> {
        let candidate = self.candidates[name];
        let ctx = self.checking_context(name, self_recursive);
        if !self_recursive {
            check_proof(&ctx, candidate.body, candidate.ty).ok()?;
            return Some(AdmittedBody {
                term: Arc::new(candidate.body.clone()),
                decreasing: None,
            });
        }
        let Term::Fix(binder, _, abstracted) =
            present_definition(name, candidate.ty, candidate.body)
        else {
            // A component that mentions itself only through a `Var` (a local
            // spelled like the definition) has no self-call to abstract.
            return None;
        };
        let fixpoint_ctx = ctx.with_term(binder.clone(), candidate.ty.clone());
        check_proof(&fixpoint_ctx, &abstracted, candidate.ty).ok()?;
        let position = fix_admission(
            &binder,
            candidate.ty,
            &abstracted,
            &[],
            candidate.decreasing,
        )
        .ok()?;
        Some(AdmittedBody {
            term: Arc::new(Term::Fix(binder, candidate.ty.clone(), abstracted)),
            decreasing: Some(position),
        })
    }

    /// The context `name`'s body is checked in: each definition it mentions,
    /// with its admitted body if it has one and its report marker otherwise.
    /// A self-recursive definition's own name is left unbound, so an
    /// occurrence the abstraction missed is refused, never resolved (D4).
    fn checking_context(&self, name: &str, self_recursive: bool) -> Context {
        let bindings = self.mentions[name]
            .iter()
            .filter(|mentioned| !(self_recursive && *mentioned == name))
            .cloned()
            .filter_map(|mentioned| {
                let other = self.candidates.get(&mentioned)?;
                let ty = other.ty.clone();
                Some(match self.decided.get(&mentioned) {
                    Some(Some(body)) => Binding::Body(mentioned, ty, body.clone()),
                    _ if other.total => Binding::Global(mentioned, ty, GlobalAdmission::Admitted),
                    _ => Binding::Global(mentioned, ty, GlobalAdmission::Opaque),
                })
            });
        Context::new().extended(bindings)
    }
}

/// D4's presentation of definition `name`, as the kernel checks it: a body
/// that calls itself by name — the elaborator's free `Global(name)` — becomes
/// `fix f'. body[Global(name) := f']`, with `f'` fresh, so `type_of_fix`
/// re-checks its termination instead of the self-call resolving to nothing;
/// any other body is presented as itself. The census and the term-mode gate
/// check definitions in this shape, so a self-recursive proof is no longer a
/// `self-reference` refusal.
#[must_use]
pub fn present_definition(name: &str, ty: &Type, body: &Term) -> Term {
    if !term_mentions_global(body, name) {
        return body.clone();
    }
    let binder = fresh_self_binder(name, body);
    let abstracted = abstract_global(body, name, &Term::Var(binder.clone()));
    Term::Fix(binder, ty.clone(), Box::new(abstracted))
}

/// Every name a `Var` or `Global` node in `term` spells, and every name the
/// `Eq` witnesses in `ty` and in the types `term` embeds spell.
///
/// Both node kinds, because the kernel checks admission on the lookup, not on
/// the node (29.9.26e D4); witness names, because the kernel types the terms
/// inside an `Eq` in proof mode (1.10.26c D7). A λ-bound name is collected
/// too; binding it as a global is harmless, since the binder shadows it.
#[must_use]
pub fn mentioned_names(term: &Term, ty: &Type) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    collect_names(term, &mut names);
    collect_witness_names(ty, &mut names);
    names
}

fn collect_names(term: &Term, names: &mut BTreeSet<String>) {
    if let Term::Var(name) | Term::Global(name) = term {
        names.insert(name.clone());
    }
    term.for_each_embedded_type(|ty| collect_witness_names(ty, names));
    term.for_each_subterm(|child| collect_names(child, names));
}

fn collect_witness_names(ty: &Type, names: &mut BTreeSet<String>) {
    if let Type::Eq(_, left, right) = ty {
        collect_names(left, names);
        collect_names(right, names);
    }
    for child in ty.children() {
        collect_witness_names(child, names);
    }
}

/// `<name>%self`, primed until no name in `body` spells it. `%` cannot appear
/// in a source identifier or an elaborator-generated binder, so no binder in
/// `body` can capture it; the check covers a body that already mentions it.
fn fresh_self_binder(name: &str, body: &Term) -> String {
    let taken = mentioned_names(body, &Type::Unit).into_iter().collect();
    fresh_name(&format!("{name}%self"), &taken)
}

/// `term` with every `Global(name)` replaced by `replacement` (D4). Types are
/// not walked: a `Global` only an embedded type spells is left, and with no
/// binding for `name` it is refused, never resolved.
fn abstract_global(term: &Term, name: &str, replacement: &Term) -> Term {
    let mut abstracted = term.clone();
    replace_global(&mut abstracted, name, replacement);
    abstracted
}

fn replace_global(term: &mut Term, name: &str, replacement: &Term) {
    if matches!(term, Term::Global(global) if global == name) {
        *term = replacement.clone();
        return;
    }
    for child in term.subterms_mut() {
        replace_global(child, name, replacement);
    }
}
