//! Dependency erasure at the elaboration→codegen boundary (ADR 30.9.26d D6).
//!
//! A `Pi` lives in elaborated Core, the IR cache and every importer's
//! signature collection, where calls substitute through it. Codegen and the
//! `compile/` passes that run before it read function types through `Arrow`
//! only, so the compile entry erases each definition once, here, and nothing
//! downstream needs a `Pi` arm.

use albite_core::terms::SpannedTerm;

use super::CoreDef;

impl CoreDef {
    /// This definition with every `Pi` — in its type and in every type its
    /// term embeds — rewritten to the `Arrow` it erases to.
    #[must_use]
    pub fn erase_dependency(&self) -> CoreDef {
        CoreDef {
            name: self.name.clone(),
            ty: self.ty.erase_dependency(),
            term: SpannedTerm {
                term: self.term.term.erase_dependency(),
                span: self.term.span,
            },
            span: self.span,
        }
    }

    /// This definition with every effect-free proof over a proposition
    /// erased to the unit value it lowers to (ADR 3.10.26e D4). The type is
    /// kept: a theorem still compiles to a function returning unit, because
    /// executable code cites theorems.
    #[must_use]
    pub fn erase_proofs(&self) -> CoreDef {
        CoreDef {
            name: self.name.clone(),
            ty: self.ty.clone(),
            term: SpannedTerm {
                term: self.term.term.erase_proofs(),
                span: self.term.span,
            },
            span: self.span,
        }
    }

    /// Everything the compile entries erase, in order: dependency, then
    /// proofs. `compile` and `info codegen symbols` both call this, so they
    /// see the same program (ADR 3.10.26e D4).
    #[must_use]
    pub fn erase_for_codegen(&self) -> CoreDef {
        self.erase_dependency().erase_proofs()
    }
}

#[cfg(test)]
mod tests {
    use albite_core::terms::{SpannedTerm, Term, TermSpan};
    use albite_core::Type;

    use crate::elaborate::CoreDef;
    use crate::span::Span;

    /// `λn:Nat. natind [i. Nat → Eq Nat i i] refl (λj. λih. ih) n`, under
    /// `ty`: a proof whose body erases.
    fn natind_lemma(ty: Type) -> CoreDef {
        let body = Type::eq(Type::Nat, Term::var("i"), Term::var("i"));
        let step = Term::lambda(
            "j",
            Type::Nat,
            Term::lambda("ih", body.clone(), Term::var("ih")),
        );
        let proof = Term::natind(
            "i",
            Type::pi("i", Type::Nat, body),
            Term::refl(Type::Nat, Term::Zero),
            step,
            Term::var("n"),
        );
        CoreDef {
            name: "lemma".to_string(),
            ty,
            term: SpannedTerm::new(Term::lambda("n", Type::Nat, proof), TermSpan::new(4, 12)),
            span: Span::new(4, 12),
        }
    }

    // 3.10.26e AC 4: the proof goes, and the name, the type and both spans
    // stay — a theorem keeps its symbol.
    #[test]
    fn erase_proofs_keeps_name_type_and_span() {
        let ty = Type::arrow(
            Type::Nat,
            Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
        );
        let def = natind_lemma(ty.clone());
        let erased = def.erase_proofs();
        assert_eq!(erased.name, "lemma");
        assert_eq!(erased.ty, ty);
        assert_eq!(erased.span, def.span);
        assert_eq!(erased.term.span, Some(TermSpan::new(4, 12)));
        assert_eq!(erased.term.term, Term::lambda("n", Type::Nat, Term::Unit));
    }

    // 3.10.26e AC 4: the shared helper both entries call erases a `Pi` in the
    // type AND the proof in the term.
    #[test]
    fn erase_for_codegen_erases_dependency_then_proofs() {
        let body = Type::eq(Type::Nat, Term::var("n"), Term::var("n"));
        let def = natind_lemma(Type::pi("n", Type::Nat, body.clone()));
        let erased = def.erase_for_codegen();
        assert_eq!(erased.ty, Type::arrow(Type::Nat, body));
        assert_eq!(erased.term.term, Term::lambda("n", Type::Nat, Term::Unit));
    }

    // 30.9.26d AC8: a definition's type and every type its term embeds lose
    // their `Pi`; the name, the spans and every non-`Pi` type are kept.
    #[test]
    fn erase_dependency_rewrites_the_type_and_the_term() {
        let body = Type::eq(Type::Nat, Term::var("n"), Term::var("n"));
        let pi = Type::pi("n", Type::Nat, body.clone());
        let def = CoreDef {
            name: "lemma".to_string(),
            ty: pi.clone(),
            term: SpannedTerm::new(
                Term::let_in("f", pi.clone(), Term::global("other"), Term::var("f")),
                TermSpan::new(3, 9),
            ),
            span: Span::new(3, 9),
        };
        let erased = def.erase_dependency();
        let arrow = Type::arrow(Type::Nat, body);
        assert_eq!(erased.name, "lemma");
        assert_eq!(erased.ty, arrow);
        assert_eq!(erased.span, def.span);
        assert_eq!(erased.term.span, Some(TermSpan::new(3, 9)));
        let Term::Let(_, let_ty, _, _) = &erased.term.term else {
            panic!("the term is not a let");
        };
        assert_eq!(*let_ty, arrow);
    }
}
