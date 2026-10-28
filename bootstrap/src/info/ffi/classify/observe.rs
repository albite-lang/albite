//! What one body is seen to do, read from its syntax (ADR 5.10.26f D2).
//!
//! Calls, method calls, paths used as values and macros are recorded as
//! [`Observation`]s; nothing is resolved here. A macro's arguments are parsed
//! as expressions and read through; one whose arguments do not parse is
//! recorded unreadable, which can make its body unknown but never pure.

use syn::visit::Visit;

use super::extract::Observation;

/// Collects a body's observations. Macro arguments are parsed as expressions
/// and visited; a macro whose arguments do not parse is recorded unreadable.
#[derive(Default)]
pub(crate) struct BodyObserver {
    pub(crate) observations: Vec<Observation>,
}

impl<'ast> Visit<'ast> for BodyObserver {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        // `<T as Trait>::f(x)` is recorded by its trait path, `Trait::f`: the
        // self type is dropped, and the trait's own method is what resolves.
        match &*call.func {
            syn::Expr::Path(path) => {
                self.observations
                    .push(Observation::Call(path_segments(&path.path)));
            }
            other => self.visit_expr(other),
        }
        for arg in &call.args {
            self.visit_expr(arg);
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.observations
            .push(Observation::Method(call.method.to_string()));
        self.visit_expr(&call.receiver);
        for arg in &call.args {
            self.visit_expr(arg);
        }
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        self.observations
            .push(Observation::PathRef(path_segments(&path.path)));
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let name = macro_name(mac);
        let arguments = macro_arguments(mac);
        self.observations.push(Observation::Macro {
            name,
            readable: arguments.is_some(),
        });
        for argument in arguments.into_iter().flatten() {
            self.visit_expr(&argument);
        }
    }
}

/// A macro's arguments as expressions: a comma list, or `[x; n]`'s repeat.
/// `None` when they do not parse — the body is then partly unreadable.
fn macro_arguments(mac: &syn::Macro) -> Option<Vec<syn::Expr>> {
    let tokens = &mac.tokens;
    match syn::parse2::<syn::Expr>(quote_bracketed(tokens)).ok()? {
        syn::Expr::Array(array) => Some(array.elems.into_iter().collect()),
        syn::Expr::Repeat(repeat) => Some(vec![*repeat.expr, *repeat.len]),
        other => Some(vec![other]),
    }
}

fn quote_bracketed(tokens: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let group = proc_macro2::Group::new(proc_macro2::Delimiter::Bracket, tokens.clone());
    proc_macro2::TokenTree::Group(group).into()
}

pub(crate) fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

pub(crate) fn macro_name(mac: &syn::Macro) -> String {
    mac.path
        .segments
        .last()
        .map_or_else(String::new, |segment| segment.ident.to_string())
}

/// The names a `thread_local! { static NAME: … }` block declares.
pub(crate) fn thread_local_names(mac: &syn::Macro) -> Vec<String> {
    let tokens: Vec<proc_macro2::TokenTree> = mac.tokens.clone().into_iter().collect();
    tokens
        .windows(2)
        .filter_map(|pair| match pair {
            [proc_macro2::TokenTree::Ident(keyword), proc_macro2::TokenTree::Ident(name)]
                if keyword == "static" =>
            {
                Some(name.to_string())
            }
            _ => None,
        })
        .collect()
}
