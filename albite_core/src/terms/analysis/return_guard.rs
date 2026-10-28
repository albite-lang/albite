//! Return detection for the evaluator's call frames (ADR 13.10.26n D3).
//!
//! `has_unguarded_return` answers whether a function body holds a `return`
//! that would exit *that* function: one outside every nested `Lambda` (a
//! closure's `return` exits the closure) and every nested `TyAbs` (a generic
//! local, which framing at its own `TyApp` catches). β-reduction frames only
//! a body that answers yes, so a body with no `return` and no `?` pays this
//! walk and nothing more.

use crate::terms::Term;

impl Term {
    /// Whether `self` holds a `Term::Return` outside every nested binder that
    /// is a function boundary of its own (`Lambda`, `TyAbs`, `CallFrame`).
    #[must_use]
    pub fn has_unguarded_return(&self) -> bool {
        match self {
            Term::Return(_) => true,
            Term::Lambda(..) | Term::TyAbs(..) | Term::CallFrame(_) => false,
            _ => {
                let mut found = false;
                self.for_each_subterm(|sub| {
                    if !found {
                        found = sub.has_unguarded_return();
                    }
                });
                found
            }
        }
    }

    /// `self` with every `Term::CallFrame` replaced by its body.
    ///
    /// A driver that stops on a stuck term hands it back with whatever
    /// frames it was inside; the frame is the evaluator's alone, so no caller
    /// that writes an evaluated term back into Core or a type may see one
    /// (ADR 13.10.26n D3). A value never holds a frame — a frame is not a
    /// value and a lambda body is never stepped — so only a stuck result
    /// pays this walk.
    #[must_use]
    pub fn strip_call_frames(mut self) -> Term {
        if let Term::CallFrame(body) = self {
            return body.strip_call_frames();
        }
        for sub in self.subterms_mut() {
            let taken = std::mem::replace(sub, Term::Unit);
            *sub = taken.strip_call_frames();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::terms::Term;
    use crate::types::Type;

    // 13.10.26n AC2: the walk finds a non-tail return under a spine, and
    // stops at a nested lambda, which is a function boundary of its own.
    #[test]
    fn a_return_under_let_and_if_is_unguarded() {
        let body = Term::let_in(
            "x",
            Type::Nat,
            Term::if_then_else(
                Term::var("c"),
                Term::early_return(Term::nat_lit(99)),
                Term::nat_lit(1),
            ),
            Term::nat_add(Term::var("x"), Term::nat_lit(1)),
        );
        assert!(body.has_unguarded_return());
        assert!(Term::early_return(Term::Unit).has_unguarded_return());
    }

    #[test]
    fn a_return_inside_a_nested_lambda_is_guarded() {
        let curried = Term::lambda("y", Type::Nat, Term::early_return(Term::var("y")));
        assert!(!curried.has_unguarded_return());
        let let_bound_closure = Term::let_in(
            "g",
            Type::arrow(Type::Nat, Type::Nat),
            curried,
            Term::app(Term::var("g"), Term::nat_lit(1)),
        );
        assert!(!let_bound_closure.has_unguarded_return());
        let generic = Term::ty_abs("T", Term::early_return(Term::Unit));
        assert!(!generic.has_unguarded_return());
        assert!(!Term::call_frame(Term::early_return(Term::Unit)).has_unguarded_return());
    }

    // 13.10.26n D3: a frame is threaded through every term walk as a unary
    // node, and stripping it leaves exactly its body.
    #[test]
    fn a_call_frame_is_threaded_through_the_term_walks() {
        let body = Term::nat_add(Term::var("x"), Term::nat_lit(1));
        let framed = Term::call_frame(body.clone());
        assert_eq!(framed.strip_spans(), framed);
        assert!(framed.free_vars().contains("x"));
        assert!(framed.free_type_vars().is_empty());
        assert_eq!(
            framed.substitute("x", &Term::nat_lit(2)),
            Term::call_frame(Term::nat_add(Term::nat_lit(2), Term::nat_lit(1)))
        );
        assert!(framed.to_string().starts_with("(frame "), "{framed}");
        assert_eq!(framed.clone().strip_call_frames(), body);
        let nested = Term::pair(framed.clone(), Term::call_frame(framed));
        assert_eq!(nested.strip_call_frames(), Term::pair(body.clone(), body));
        assert!(crate::typecheck::TypeError::CallFrameInProof
            .to_string()
            .contains("call frame"));
    }

    #[test]
    fn a_return_free_body_has_none() {
        let body = Term::nat_add(Term::var("x"), Term::nat_lit(1));
        assert!(!body.has_unguarded_return());
        assert!(!Term::Unit.has_unguarded_return());
    }
}
