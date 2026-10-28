//! The core's at-most-once buffer rule (ADR 6.10.26m D3), read syntactically.
//!
//! In the core's own run a buffer's header is a trailer inside its data block,
//! so once 6.10.26n's grow `realloc`s that block, a stale handle points into
//! freed memory. The rule that keeps that safe: every buffer variable is
//! referenced **at most once** in its scope.
//!
//! `Bytes` is a single-field constructor, and elaboration erases it: a buffer
//! reaches Core as a `Nat` binding (6.10.26m P0.3), so the rule cannot key on
//! a binding's type. It keys on what makes a binding a buffer in the source:
//!
//! - a parameter the source declares `Bytes`;
//! - the scrutinee binding an `MkBytes` pattern lowers to ([`MKBYTES_SCRUTINEE`]);
//! - a `let` whose definition is a buffer variable — the `MkBytes` handle,
//!   `let h = __ctor_MkBytes`, is one;
//! - a `let` whose definition is a call to a core function the source
//!   declares as returning `Bytes`.
//!
//! Uses are counted by [`Term::var_use_count`], which **sums** match arms, so
//! a buffer used once in each of two arms is refused. That is conservative:
//! it refuses a safe program and never accepts an unsafe one.

use std::collections::{BTreeMap, BTreeSet};

use albite_core::terms::Term;

use crate::ast::{Item, Pattern, TypeExpr};

/// The `let` an `MkBytes(h)` pattern lowers its scrutinee to.
pub const MKBYTES_SCRUTINEE: &str = "__ctor_MkBytes";

/// The buffer type's source name.
const BUFFER_TYPE: &str = "Bytes";

/// What the source declares about buffers, by bare function name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BufferSignatures {
    /// Functions whose declared return type is `Bytes`.
    pub returns_buffer: BTreeSet<String>,
    /// Each function's parameters declared `Bytes`, by name.
    pub buffer_params: BTreeMap<String, Vec<String>>,
}

/// Whether `ty` names the buffer type, qualified or not.
fn is_buffer_type(ty: &TypeExpr) -> bool {
    matches!(ty, TypeExpr::Path(path)
        if path.segments.last().is_some_and(|segment| segment.name == BUFFER_TYPE))
}

/// Read every function signature in `texts` for its `Bytes` parameters and
/// return type.
#[must_use]
pub fn buffer_signatures(texts: &[&str]) -> BufferSignatures {
    let mut signatures = BufferSignatures::default();
    for text in texts {
        let (parsed, _) = crate::parser::parse(text);
        for item in &parsed.items {
            let Item::Function(function) = item else {
                continue;
            };
            let name = function.name.name.clone();
            if function.return_type.as_ref().is_some_and(is_buffer_type) {
                signatures.returns_buffer.insert(name.clone());
            }
            let params: Vec<String> = function
                .params
                .iter()
                .filter(|param| is_buffer_type(&param.ty))
                .filter_map(|param| match &param.pattern {
                    Pattern::Var(ident) => Some(ident.name.clone()),
                    _ => None,
                })
                .collect();
            if !params.is_empty() {
                signatures.buffer_params.insert(name, params);
            }
        }
    }
    signatures
}

/// The last `::` segment of a definition or global name.
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

/// The global an application spine is headed by, if any.
fn call_head(term: &Term) -> Option<&str> {
    match unwrapped(term) {
        Term::App(function, _) => call_head(function),
        Term::Global(name) => Some(name),
        _ => None,
    }
}

/// Whether a `let x = definition` binds a buffer, given those in scope.
fn binds_buffer(
    name: &str,
    definition: &Term,
    in_scope: &[String],
    signatures: &BufferSignatures,
) -> bool {
    if name == MKBYTES_SCRUTINEE {
        return true;
    }
    if let Term::Var(source) = unwrapped(definition) {
        return in_scope.contains(source);
    }
    call_head(definition).is_some_and(|head| signatures.returns_buffer.contains(bare_name(head)))
}

/// Every buffer variable `definition`'s body references more than once in
/// its scope, in binding order.
#[must_use]
pub fn reused_buffers(definition: &str, term: &Term, signatures: &BufferSignatures) -> Vec<String> {
    let params = signatures
        .buffer_params
        .get(bare_name(definition))
        .cloned()
        .unwrap_or_default();
    let mut walk_state = BufferWalk::default();
    walk(term, &params, &mut walk_state, signatures);
    walk_state.reused
}

/// What a walk carries: the buffers in scope, and those found reused.
#[derive(Default)]
struct BufferWalk {
    in_scope: Vec<String>,
    reused: Vec<String>,
}

/// Walk `term` with the buffers in scope. A top-level parameter is a buffer
/// when `params` names it; any other binder shadows a buffer of its name.
fn walk(term: &Term, params: &[String], state: &mut BufferWalk, signatures: &BufferSignatures) {
    match term {
        Term::Lambda(name, _, body) | Term::Fix(name, _, body) => {
            let is_buffer = params.contains(name);
            scoped(name, is_buffer, body, state, |inner| {
                walk(body, params, inner, signatures);
            });
        }
        Term::Let(name, _, definition, body) => {
            walk(definition, &[], state, signatures);
            let is_buffer = binds_buffer(name, definition, &state.in_scope, signatures);
            scoped(name, is_buffer, body, state, |inner| {
                walk(body, &[], inner, signatures);
            });
        }
        Term::Spanned(inner, _) => walk(inner, params, state, signatures),
        other => other.for_each_subterm(|child| walk(child, &[], state, signatures)),
    }
}

/// Bind `name` over `body`: record it when it is a buffer used twice, then
/// walk the body with it in scope (a buffer) or shadowed (anything else).
fn scoped(
    name: &str,
    is_buffer: bool,
    body: &Term,
    state: &mut BufferWalk,
    walk_body: impl FnOnce(&mut BufferWalk),
) {
    if is_buffer && body.var_use_count(name) > 1 {
        state.reused.push(name.to_string());
    }
    let saved = state.in_scope.clone();
    state.in_scope.retain(|buffer| buffer != name);
    if is_buffer {
        state.in_scope.push(name.to_string());
    }
    walk_body(state);
    state.in_scope = saved;
}
