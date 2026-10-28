//! The core's adoption rule (ADR 6.10.26n D3), read syntactically.
//!
//! `tg_bytes_from_string_grown` `realloc`s its string argument's block, so it
//! is safe only on a string that is dead and is the allocator's. The code
//! generator calls an owned export only on the liveness gate's dead heap
//! temporary (18.5.26f), so the rule pins adoption to exactly that operand,
//! in two hops:
//!
//! - the extern may be called only inside the wrapper [`ADOPT_WRAPPER`]. An
//!   `extern` declaration elaborates to a stub definition of its own name
//!   whose body is the `extern_call`, and a caller names that stub as a
//!   global, so both spellings are the extern hop, and the stub itself is
//!   the one other definition the hop is allowed in;
//! - the wrapper may be called only inside a definition an export row
//!   shadowing `tg_string_concat_owned` names.
//!
//! At each hop the adopted argument must be the definition's **first
//! parameter itself** — not an expression derived from it, which could be an
//! interior pointer — and that parameter may not be read after the adopting
//! call in evaluation order, because its pointer may have moved or been
//! freed. Earlier reads, and reads in a branch the adoption never reaches,
//! are allowed: `if`/`case` arms are walked from the state before the
//! branch, and a read is "after" only along the path that adopted.
//! Elsewhere the walk is sequential, which can refuse a safe program but
//! never accepts an unsafe one.

use albite_core::native_core::NATIVE_CORE_EXPORTS;
use albite_core::terms::Term;

/// The adopting intrinsic.
pub const ADOPT_EXTERN: &str = "tg_bytes_from_string_grown";

/// The core's one wrapper over it (`bytes.tg`).
pub const ADOPT_WRAPPER: &str = "bytes_from_string_grown";

/// The Rust export whose shadowing rows may adopt.
const OWNED_CONCAT: &str = "tg_string_concat_owned";

/// Which hop an adopting call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptHop {
    /// A call to [`ADOPT_EXTERN`]: its `extern_call`, or its stub by name.
    Extern,
    /// A call to [`ADOPT_WRAPPER`].
    Wrapper,
}

/// What an adopting call adopts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptedArgument {
    /// The definition's first parameter, as a bare variable.
    FirstParameter,
    /// An expression that mentions the first parameter, but is not it.
    DerivedFromFirst,
    /// Anything else: another parameter, a literal, a local.
    Other,
}

/// One adopting call in a definition's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adoption {
    /// Which hop.
    pub hop: AdoptHop,
    /// What it adopts.
    pub argument: AdoptedArgument,
    /// Whether the first parameter is read after it, on a path through it.
    pub used_after: bool,
}

/// The definitions an owned export row names: the only ones that may call
/// [`ADOPT_WRAPPER`].
#[must_use]
pub fn owned_definitions() -> Vec<&'static str> {
    NATIVE_CORE_EXPORTS
        .iter()
        .filter(|row| row.shadows == OWNED_CONCAT)
        .map(|row| row.definition)
        .collect()
}

/// The last `::` segment of a name.
fn bare_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// `term` without its source-span and annotation wrappers.
fn unwrapped(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) | Term::Annot(inner, _) => unwrapped(inner),
        other => other,
    }
}

/// The first parameter of a definition's term — its outermost lambda's
/// binder — and that lambda's body.
fn first_parameter(term: &Term) -> Option<(&str, &Term)> {
    match unwrapped(term) {
        Term::Lambda(name, _, body) => Some((name, body)),
        Term::TyAbs(_, body) => first_parameter(body),
        _ => None,
    }
}

/// Every adopting call in `term`, a definition's whole term, in walk order.
#[must_use]
pub fn adoptions(term: &Term) -> Vec<Adoption> {
    let (first, body) = match first_parameter(term) {
        Some((name, body)) => (Some(name.to_string()), body),
        None => (None, term),
    };
    let mut walk = AdoptWalk {
        first,
        pending: Vec::new(),
        found: Vec::new(),
    };
    walk.visit(body);
    walk.found
}

/// The walk's state: the first parameter while it is in scope, the
/// adoptions on the current path, and every adoption found.
struct AdoptWalk {
    first: Option<String>,
    pending: Vec<usize>,
    found: Vec<Adoption>,
}

impl AdoptWalk {
    fn visit(&mut self, term: &Term) {
        match term {
            Term::Var(name) if self.first.as_deref() == Some(name) => {
                for &index in &self.pending {
                    self.found[index].used_after = true;
                }
            }
            Term::ExternCall(symbol, args) => {
                for arg in args {
                    self.visit(arg);
                }
                if bare_name(symbol.strip_prefix("__c_").unwrap_or(symbol)) == ADOPT_EXTERN {
                    self.adopt(AdoptHop::Extern, args.first());
                }
            }
            Term::App(..) => self.visit_application(term),
            Term::If(condition, then_branch, else_branch) => {
                self.visit(condition);
                self.branches(&[(None, then_branch), (None, else_branch)]);
            }
            Term::Case(scrutinee, left_name, left, right_name, right) => {
                self.visit(scrutinee);
                self.branches(&[(Some(left_name), left), (Some(right_name), right)]);
            }
            Term::Let(name, _, definition, body) => {
                self.visit(definition);
                self.scoped(name, body);
            }
            Term::Lambda(name, _, body) | Term::Fix(name, _, body) => self.scoped(name, body),
            other => other.for_each_subterm(|child| self.visit(child)),
        }
    }

    /// An application spine: a call to the wrapper adopts, after its
    /// arguments are evaluated; any other spine is walked in order.
    fn visit_application(&mut self, term: &Term) {
        let mut args = Vec::new();
        let mut head = term;
        while let Term::App(function, arg) = unwrapped(head) {
            args.push(arg.as_ref());
            head = function;
        }
        args.reverse();
        let hop = match unwrapped(head) {
            Term::Global(name) if bare_name(name) == ADOPT_WRAPPER => Some(AdoptHop::Wrapper),
            Term::Global(name) if bare_name(name) == ADOPT_EXTERN => Some(AdoptHop::Extern),
            _ => None,
        };
        self.visit(head);
        for arg in &args {
            self.visit(arg);
        }
        if let Some(hop) = hop {
            self.adopt(hop, args.first().copied());
        }
    }

    fn adopt(&mut self, hop: AdoptHop, adopted: Option<&Term>) {
        let argument = match (adopted, self.first.as_deref()) {
            (Some(arg), Some(first)) if matches!(unwrapped(arg), Term::Var(v) if v == first) => {
                AdoptedArgument::FirstParameter
            }
            (Some(arg), Some(first)) if arg.var_use_count(first) > 0 => {
                AdoptedArgument::DerivedFromFirst
            }
            _ => AdoptedArgument::Other,
        };
        self.pending.push(self.found.len());
        self.found.push(Adoption {
            hop,
            argument,
            used_after: false,
        });
    }

    /// Alternative arms: each starts from the path before the branch, and
    /// the path after it is every arm's, joined.
    fn branches(&mut self, arms: &[(Option<&String>, &Term)]) {
        let before = self.pending.clone();
        let mut joined = before.clone();
        for (binder, arm) in arms {
            self.pending.clone_from(&before);
            match binder {
                Some(name) => self.scoped(name, arm),
                None => self.visit(arm),
            }
            joined.extend(self.pending.iter().filter(|index| !before.contains(index)));
        }
        self.pending = joined;
    }

    /// Walk `body` under a binder `name`, which shadows the first parameter
    /// when it reuses its name.
    fn scoped(&mut self, name: &str, body: &Term) {
        let shadows = self.first.as_deref() == Some(name);
        let saved = if shadows { self.first.take() } else { None };
        self.visit(body);
        if shadows {
            self.first = saved;
        }
    }
}
