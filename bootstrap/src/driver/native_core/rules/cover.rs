//! The core's write-cover rule (ADR 9.10.26n D3), read syntactically.
//!
//! In the core's own run two allocations leave their new bytes unwritten
//! (D2): `bytes_new_uninit(L)` opens `[0, L)`, and `bytes_from_string_grown(s,
//! E)` opens `[string_len(s), string_len(s) + E)`. The rule admits exactly one
//! continuation after either opening call, bound by a `let`:
//!
//! ```text
//! let b₁ = bytes_blit_string(b₀, o₁, s₁); … let bₖ = bytes_blit_string(bₖ₋₁, oₖ, sₖ);
//! bytes_into_string_unchecked(bₖ)
//! ```
//!
//! with k ≥ 1, each `sᵢ` a variable, and the copies tiling the unwritten range
//! exactly: `o₁ ≡ start`, `oᵢ₊₁ ≡ oᵢ + string_len(sᵢ)`, and `oₖ +
//! string_len(sₖ) ≡ start + size`. `≡` compares **linear sums** — a constant
//! plus a multiset of `string_len(v)` atoms keyed by the *binder* `v`
//! resolves to, not its spelling — so a shadowed name, or an alias whose
//! string was rebound, is a different atom and the tiling fails. Anything
//! that does not normalise is refused. No other use of any `bᵢ` fits the
//! shape: no read, no branch, no other call before the conversion.
//!
//! Who may name the unfilled allocation is checked here too, on **every
//! occurrence** rather than every call, so a wrapper passed as a value is
//! caught: `tg_bytes_new_uninit` only in its wrapper and its own stub, the
//! wrapper only in a definition an export row names.

use std::collections::BTreeMap;

use albite_core::native_core::NATIVE_CORE_EXPORTS;
use albite_core::terms::Term;

/// The unfilled-allocation intrinsic.
pub const UNINIT_EXTERN: &str = "tg_bytes_new_uninit";

/// Its one wrapper (`bytes.tg`).
pub const UNINIT_WRAPPER: &str = "bytes_new_uninit";

/// The grow wrapper, the other opening call (6.10.26n D2).
const GROWN_WRAPPER: &str = "bytes_from_string_grown";

/// The one copy a chain link may make.
const BLIT_WRAPPER: &str = "bytes_blit_string";

/// The one conversion a chain may end in.
const CONVERT_WRAPPER: &str = "bytes_into_string_unchecked";

/// The last `::` segment of a name, `__c_` prefix stripped.
fn bare_name(name: &str) -> &str {
    let bare = name.rsplit("::").next().unwrap_or(name);
    bare.strip_prefix("__c_").unwrap_or(bare)
}

/// `term` without its source-span and annotation wrappers.
fn unwrapped(term: &Term) -> &Term {
    match term {
        Term::Spanned(inner, _) | Term::Annot(inner, _) => unwrapped(inner),
        other => other,
    }
}

/// An application spine's global head and its arguments, in order.
fn call_spine(term: &Term) -> Option<(&str, Vec<&Term>)> {
    let mut args = Vec::new();
    let mut head = unwrapped(term);
    while let Term::App(function, arg) = head {
        args.push(arg.as_ref());
        head = unwrapped(function);
    }
    args.reverse();
    match head {
        Term::Global(name) => Some((bare_name(name), args)),
        _ => None,
    }
}

/// A length's normal form: a constant plus `string_len` atoms, each keyed by
/// the binder its string resolves to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LinearSum {
    constant: u128,
    atoms: BTreeMap<usize, u32>,
}

impl LinearSum {
    fn constant(value: u128) -> Self {
        LinearSum {
            constant: value,
            ..LinearSum::default()
        }
    }

    fn atom(binder: usize) -> Self {
        LinearSum {
            constant: 0,
            atoms: BTreeMap::from([(binder, 1)]),
        }
    }

    fn plus(mut self, other: &LinearSum) -> Self {
        self.constant += other.constant;
        for (binder, count) in &other.atoms {
            *self.atoms.entry(*binder).or_default() += count;
        }
        self
    }
}

/// `3 + len(#0) + 2·len(#1)`: atoms by binder number, in binding order.
impl std::fmt::Display for LinearSum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.constant)?;
        for (binder, count) in &self.atoms {
            let factor = if *count == 1 {
                String::new()
            } else {
                format!("{count}·")
            };
            write!(f, " + {factor}len(#{binder})")?;
        }
        Ok(())
    }
}

/// One binder in scope: its name, a unique id, and — for a `let` whose
/// definition normalises — that normal form, so an alias resolves to it.
struct Binder {
    name: String,
    id: usize,
    value: Option<LinearSum>,
}

/// The walk: the binders in scope, the next id, and the reasons found.
struct CoverWalk<'a> {
    definition: &'a str,
    scope: Vec<Binder>,
    next_id: usize,
    reasons: Vec<String>,
}

/// Every way `definition`'s `term` breaks the write-cover rule, one reason
/// each; empty when it keeps it.
#[must_use]
pub fn uncovered_buffers(definition: &str, term: &Term) -> Vec<String> {
    let mut walk = CoverWalk {
        definition: bare_name(definition),
        scope: Vec::new(),
        next_id: 0,
        reasons: Vec::new(),
    };
    walk.visit(term);
    walk.reasons
}

/// Whether `definition` may name `name` at all (D3, "Who may name it").
fn may_name(definition: &str, name: &str) -> bool {
    match name {
        UNINIT_EXTERN => definition == UNINIT_WRAPPER || definition == UNINIT_EXTERN,
        UNINIT_WRAPPER => NATIVE_CORE_EXPORTS
            .iter()
            .any(|row| row.definition == definition),
        _ => true,
    }
}

/// Whether a call to `head` opens an unwritten range: the two wrappers, and
/// only outside the wrappers themselves, which are the hop.
fn is_opening(definition: &str, head: &str) -> bool {
    (head == UNINIT_WRAPPER || head == GROWN_WRAPPER)
        && definition != UNINIT_WRAPPER
        && definition != GROWN_WRAPPER
}

impl CoverWalk<'_> {
    fn refuse(&mut self, reason: impl Into<String>) {
        self.reasons.push(reason.into());
    }

    fn bind(&mut self, name: &str, value: Option<LinearSum>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.scope.push(Binder {
            name: name.to_string(),
            id,
            value,
        });
        id
    }

    fn resolve(&self, name: &str) -> Option<&Binder> {
        self.scope.iter().rev().find(|binder| binder.name == name)
    }

    /// The binder a bare variable resolves to.
    fn variable(&self, term: &Term) -> Option<usize> {
        match unwrapped(term) {
            Term::Var(name) => self.resolve(name).map(|binder| binder.id),
            _ => None,
        }
    }

    /// `term`'s linear sum, or `None` when it does not normalise.
    fn normalise(&self, term: &Term) -> Option<LinearSum> {
        match unwrapped(term) {
            Term::Zero => Some(LinearSum::constant(0)),
            Term::NatLit(value) => Some(LinearSum::constant(u128::from(*value))),
            Term::Succ(inner) => Some(self.normalise(inner)?.plus(&LinearSum::constant(1))),
            Term::NatAdd(left, right) => Some(self.normalise(left)?.plus(&self.normalise(right)?)),
            Term::StrLen(text) => self.variable(text).map(LinearSum::atom),
            Term::Var(name) => self.resolve(name)?.value.clone(),
            _ => None,
        }
    }

    fn visit(&mut self, term: &Term) {
        match term {
            Term::Global(name) | Term::ExternCall(name, _)
                if !may_name(self.definition, bare_name(name)) =>
            {
                let name = bare_name(name).to_string();
                self.refuse(format!("names `{name}` outside the definitions that may"));
                term.for_each_subterm(|child| self.visit(child));
            }
            Term::Global(name) if is_opening(self.definition, bare_name(name)) => {
                let name = bare_name(name).to_string();
                self.refuse(format!(
                    "names `{name}` as a value, not a `let`'s opening call"
                ));
            }
            Term::Let(name, _, definition, body) => self.visit_let(name, definition, body),
            Term::Lambda(name, _, body) | Term::Fix(name, _, body) => self.scoped(name, None, body),
            Term::Case(scrutinee, left_name, left, right_name, right) => {
                self.visit(scrutinee);
                self.scoped(left_name, None, left);
                self.scoped(right_name, None, right);
            }
            Term::AdtMatch(scrutinee, arms) => {
                self.visit(scrutinee);
                for (_, name, arm) in arms {
                    self.scoped(name, None, arm);
                }
            }
            Term::App(..) => self.visit_application(term),
            other => other.for_each_subterm(|child| self.visit(child)),
        }
    }

    /// The opening call this spine makes, if it is one, after refusing it
    /// in a definition that may not name it.
    fn opening_call<'t>(&mut self, term: &'t Term) -> Option<(&'t str, Vec<&'t Term>)> {
        let (head, args) = call_spine(term)?;
        if !is_opening(self.definition, head) {
            return None;
        }
        if !may_name(self.definition, head) {
            self.refuse(format!("names `{head}` outside the definitions that may"));
        }
        Some((head, args))
    }

    /// A `let`: an opening call starts a chain; anything else is walked and
    /// bound, with its normal form when it has one, so an alias resolves.
    fn visit_let(&mut self, name: &str, definition: &Term, body: &Term) {
        if let Some((head, args)) = self.opening_call(definition) {
            for arg in &args {
                self.visit(arg);
            }
            self.check_chain(name, head, &args, body);
            return;
        }
        self.visit(definition);
        let value = self.normalise(definition);
        self.scoped(name, value, body);
    }

    /// An application: an opening call here is not a `let`'s definition, so
    /// it is refused; any other spine is walked.
    fn visit_application(&mut self, term: &Term) {
        let Some((head, args)) = self.opening_call(term) else {
            term.for_each_subterm(|child| self.visit(child));
            return;
        };
        self.refuse(format!("calls `{head}` other than as a `let`'s definition"));
        for arg in &args {
            self.visit(arg);
        }
    }

    /// Walk `body` with `name` bound, then unbind it.
    fn scoped(&mut self, name: &str, value: Option<LinearSum>, body: &Term) {
        self.bind(name, value);
        self.visit(body);
        self.scope.pop();
    }

    /// The opening call `head(args)` is bound to `buffer` over `body`: the
    /// body must be exactly the tiling chain and the conversion.
    fn check_chain(&mut self, buffer: &str, head: &str, args: &[&Term], body: &Term) {
        let depth = self.scope.len();
        match self.unwritten_range(head, args) {
            Some((start, size)) => {
                let opened = self.bind(buffer, None);
                if let Err(reason) = self.follow_chain(opened, start, &size, body) {
                    self.refuse(reason);
                }
            }
            None => self.refuse(format!("`{head}`'s size does not normalise")),
        }
        self.scope.truncate(depth);
    }

    /// `(start, start + size)` of the range an opening call leaves unwritten.
    fn unwritten_range(&self, head: &str, args: &[&Term]) -> Option<(LinearSum, LinearSum)> {
        match (head, args) {
            (UNINIT_WRAPPER, [size]) => {
                let start = LinearSum::constant(0);
                let end = start.clone().plus(&self.normalise(size)?);
                Some((start, end))
            }
            (GROWN_WRAPPER, [text, extra]) => {
                let start = LinearSum::atom(self.variable(text)?);
                let end = start.clone().plus(&self.normalise(extra)?);
                Some((start, end))
            }
            _ => None,
        }
    }

    /// Follow the links from `previous` at `cursor` to the conversion, which
    /// must land at `end`. Each link binds its buffer, so later links resolve
    /// it; the caller unbinds them all.
    fn follow_chain(
        &mut self,
        mut previous: usize,
        mut cursor: LinearSum,
        end: &LinearSum,
        mut body: &Term,
    ) -> Result<(), String> {
        let mut links = 0;
        while let Term::Let(name, _, definition, rest) = unwrapped(body) {
            cursor = self.link(previous, &cursor, definition)?;
            previous = self.bind(name, None);
            links += 1;
            body = rest;
        }
        let converts_last_link = matches!(call_spine(body),
            Some((CONVERT_WRAPPER, args))
                if args.len() == 1 && self.variable(args[0]) == Some(previous));
        if links == 0 || !converts_last_link {
            return Err(format!(
                "uses the buffer other than by {links} copies and one unchecked conversion"
            ));
        }
        if &cursor == end {
            Ok(())
        } else {
            Err(format!(
                "the copies end at {cursor}, not the range's end {end}"
            ))
        }
    }

    /// One link `bytes_blit_string(previous, o, s)` at `cursor`: the next cursor.
    fn link(
        &self,
        previous: usize,
        cursor: &LinearSum,
        definition: &Term,
    ) -> Result<LinearSum, String> {
        let Some((BLIT_WRAPPER, args)) = call_spine(definition) else {
            return Err("a link is not a `bytes_blit_string` call".to_string());
        };
        let [buffer, offset, text] = args[..] else {
            return Err("a copy does not take three arguments".to_string());
        };
        if self.variable(buffer) != Some(previous) {
            return Err("a copy's buffer is not the previous link's".to_string());
        }
        let Some(text) = self.variable(text) else {
            return Err("a copy's string is not a variable".to_string());
        };
        match self.normalise(offset) {
            Some(offset) if &offset == cursor => Ok(offset.plus(&LinearSum::atom(text))),
            Some(offset) => Err(format!(
                "a copy at {offset} leaves a gap or overlap at {cursor}"
            )),
            None => Err("a copy's offset does not normalise".to_string()),
        }
    }
}
