//! The admitted bodies a context carries, and the map conversion reads them
//! through (ADR 2.10.26l D3).
//!
//! A body enters a context only as an [`AdmittedBody`], which only the
//! kernel's own admission (`typecheck/delta.rs`) can make, and only through
//! [`Context::extended`], which is crate-private. Outside
//! `albite_core` there is therefore no way to hand the kernel a body it has
//! not typed itself. Every binding shadows by recency, so a later `Opaque` (or
//! term) binding of the same name hides an admitted body, and a context with
//! no body gives conversion an empty map — today's behaviour exactly.

use std::collections::HashMap;

use crate::conversion::Unfoldings;
use crate::typecheck::AdmittedBody;
use crate::Term;

use super::{Binding, Context};

impl Context {
    /// The context extended with every binding in `bindings`, in order — one
    /// clone of the binding list, where a `with_*` chain clones it per
    /// binding (D3.4: an admitted closure can be long).
    #[must_use]
    pub(crate) fn extended(&self, bindings: impl IntoIterator<Item = Binding>) -> Self {
        let mut ctx = self.clone();
        ctx.bindings.extend(bindings);
        ctx
    }

    /// What conversion may unfold under this context: each name whose most
    /// recent term-level binding carries an admitted body.
    pub(crate) fn unfoldings(&self) -> ContextUnfoldings<'_> {
        let mut seen: HashMap<&str, Option<&AdmittedBody>> = HashMap::new();
        for binding in self.bindings.iter().rev() {
            let (name, body) = match binding {
                Binding::Body(name, _, body) => (name.as_str(), Some(body)),
                Binding::Term(name, _) | Binding::Global(name, _, _) => (name.as_str(), None),
                Binding::TypeVar(_) | Binding::PredVar(..) => continue,
            };
            seen.entry(name).or_insert(body);
        }
        ContextUnfoldings {
            bodies: seen
                .into_iter()
                .filter_map(|(name, body)| Some((name, body?)))
                .collect(),
        }
    }
}

/// A context's admitted bodies, keyed by global name, as conversion reads
/// them: the body, and a self-recursive one's certified decreasing position.
pub(crate) struct ContextUnfoldings<'a> {
    bodies: HashMap<&'a str, &'a AdmittedBody>,
}

impl ContextUnfoldings<'_> {
    /// Whether no body may be unfolded.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }
}

impl Unfoldings for ContextUnfoldings<'_> {
    fn body(&self, name: &str) -> Option<&Term> {
        self.bodies.get(name).map(|body| body.term())
    }

    fn decreasing(&self, name: &str) -> Option<usize> {
        self.bodies.get(name).and_then(|body| body.decreasing())
    }
}
