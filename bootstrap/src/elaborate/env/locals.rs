//! Local scope and type variable management for Env.
//!
//! Manages the local variable binding stack and type variable scope.

use std::collections::HashMap;

use albite_core::Type;

use super::definitions::LocalBinding;
use super::Env;

impl Env {
    // ─────────────────────────────────────────────────────────────────────────
    // Local scopes
    // ─────────────────────────────────────────────────────────────────────────

    /// Enter a new local scope.
    pub fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
    }

    /// Exit the current local scope.
    pub fn pop_scope(&mut self) {
        self.locals.pop();
    }

    /// Bind a local variable in the current scope.
    pub fn bind_local(&mut self, name: String, ty: Type, level: usize) {
        self.bind_local_with(name, ty, level, false);
    }

    /// Bind a `let mut` local, or rebind one by assignment (ADR 13.10.26r D3).
    pub fn bind_mutable_local(&mut self, name: String, ty: Type, level: usize) {
        self.bind_local_with(name, ty, level, true);
    }

    fn bind_local_with(&mut self, name: String, ty: Type, level: usize, mutable: bool) {
        let depths = self.mutable_depths;
        if let Some(scope) = self.locals.last_mut() {
            let binding = LocalBinding {
                name: name.clone(),
                ty,
                level,
                mutable,
                depths,
            };
            scope.insert(name, binding);
        }
    }

    /// Look up a local variable by name.
    pub(crate) fn lookup_local(&self, name: &str) -> Option<&LocalBinding> {
        // Search from innermost to outermost scope
        for scope in self.locals.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(binding);
            }
        }
        None
    }

    /// Every local binding in every open scope, outermost first. A tactic
    /// reads the theorem's parameters here (ADR 3.10.26e D6).
    pub(crate) fn local_bindings(&self) -> impl Iterator<Item = &LocalBinding> {
        self.locals.iter().flat_map(HashMap::values)
    }

    /// Get the current de Bruijn depth (number of local bindings).
    #[allow(dead_code)]
    pub fn depth(&self) -> usize {
        self.locals.iter().map(|s| s.len()).sum()
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Type variables
    // ─────────────────────────────────────────────────────────────────────────

    /// Add a type variable to scope.
    pub fn push_type_var(&mut self, name: String) {
        self.type_vars.push((name, None));
    }

    /// Add a type parameter with its kind: `Some(domains)` for a predicate
    /// variable, `None` for a plain one (ADR 6.10.26h D2). Popped by
    /// [`Env::pop_type_var`] like any other.
    pub fn push_kinded_type_var(&mut self, name: String, kind: Option<Vec<Type>>) {
        self.type_vars.push((name, kind));
    }

    /// Remove a type variable from scope.
    pub fn pop_type_var(&mut self) {
        self.type_vars.pop();
    }

    /// Check if a type variable is in scope.
    pub fn has_type_var(&self, name: &str) -> bool {
        self.type_vars.iter().any(|(var, _)| var == name)
    }

    /// The domains of the innermost type variable `name` when it is a
    /// predicate variable; `None` when it is a plain one or not in scope.
    pub fn type_var_kind(&self, name: &str) -> Option<&[Type]> {
        self.type_vars
            .iter()
            .rev()
            .find(|(var, _)| var == name)
            .and_then(|(_, kind)| kind.as_deref())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Self-references (ADR 6.10.26b D1)
    // ─────────────────────────────────────────────────────────────────────────

    /// Put a type under collection in scope under its own name, with the
    /// parameters a uniform recursive use must pass.
    pub fn push_self_reference(&mut self, name: String, params: Vec<String>) {
        self.self_references.push((name, params));
    }

    /// Remove the innermost type under collection from scope.
    pub fn pop_self_reference(&mut self) {
        self.self_references.pop();
    }

    /// The parameters of the innermost type under collection named `name`,
    /// or `None` when `name` is not a self-reference here.
    pub fn self_reference_params(&self, name: &str) -> Option<&[String]> {
        self.self_references
            .iter()
            .rev()
            .find(|(own, _)| own == name)
            .map(|(_, params)| params.as_slice())
    }
}
