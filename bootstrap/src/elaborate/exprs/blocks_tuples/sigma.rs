//! A `let` tuple pattern over a dependent pair (ADR 6.10.26c).
//!
//! `let (k, e) = s; body`, with `s : Σx:A. B`, is checked with `k : A` and
//! `e : B[x := k]` in scope and emitted as the β-redex
//! `(λk:A. λe:B[x := k]. body) (fst s) (snd s)` (D2). The kernel's λ rule and
//! Π-application perform `k := fst(s)`; its `let` is opaque (no ζ), which is
//! why the lowering is not `let k = fst(s); let e = snd(s)`. Only the two
//! `let` entry points reach this file: `match`, and a `Sigma` nested inside a
//! `Product` pattern, keep today's E0014 (D4).

use crate::ast::Pattern;
use crate::span::{Span, Spanned};
use albite_core::{Term, Type};

use crate::elaborate::error::{ElabError, ElabErrorKind};
use crate::elaborate::{ElabResult, Elaborator};

/// The note that tells D3's refusal apart from every other E0011.
const ESCAPING_WITNESS_NOTE: &str =
    "the type of this block depends on the unpacked witness; state the expected type";

/// A `let` whose tuple pattern meets a `Sigma`: the pattern's parts and the
/// already-inferred scrutinee.
pub(in crate::elaborate::exprs) struct SigmaLet<'p> {
    patterns: &'p [Pattern],
    value_term: Term,
    value_ty: Type,
    span: Span,
}

impl<'p> SigmaLet<'p> {
    pub(in crate::elaborate::exprs) fn new(
        patterns: &'p [Pattern],
        value_term: Term,
        value_ty: Type,
        span: Span,
    ) -> Self {
        Self {
            patterns,
            value_term,
            value_ty,
            span,
        }
    }
}

/// `[A, B[x := k]]` for a named first part, `[A, B[x := fst(s)]]` for `_`
/// (D4); `None` when `ty` is no dependent pair.
pub(in crate::elaborate::exprs) fn sigma_parts(
    ty: &Type,
    first: &Pattern,
    s: &str,
) -> Option<[Type; 2]> {
    let Type::Sigma(binder, witness_ty, evidence_ty) = ty else {
        return None;
    };
    let witness = if let Pattern::Var(ident) = first {
        Term::var(&ident.name)
    } else {
        Term::fst(Term::var(s))
    };
    Some([
        (**witness_ty).clone(),
        evidence_ty.substitute_term(binder, &witness),
    ])
}

/// The local a scrutinee already is, so no `__tup` is needed (D2).
fn scrutinee_local(term: &Term) -> Option<&str> {
    match term {
        Term::Spanned(inner, _) => scrutinee_local(inner),
        Term::Var(name) => Some(name),
        _ => None,
    }
}

/// The name a pattern part binds, or `None` for `_`. A nested tuple binds a
/// fresh `__e`, which the inner pattern then takes apart (D2's nesting).
enum PartName {
    Named(String),
    Unbound,
}

impl PartName {
    fn name(&self) -> Option<&str> {
        match self {
            PartName::Named(name) => Some(name),
            PartName::Unbound => None,
        }
    }
}

impl<'a> Elaborator<'a> {
    /// `let <tuple pattern> = <dependent pair>; body` (ADR 6.10.26c D1–D3).
    ///
    /// `body` elaborates the continuation with the pattern's names in scope;
    /// in check mode it returns `expected`, as the `Product` path does.
    pub(in crate::elaborate::exprs) fn elab_let_sigma(
        &mut self,
        pair: SigmaLet<'_>,
        expected: Option<&Type>,
        body: impl FnOnce(&mut Self) -> ElabResult<(Term, Type)>,
    ) -> ElabResult<(Term, Type)> {
        // A part named like the scrutinee would make `fst(s)`/`snd(s)` refer
        // to the part, so that case binds a fresh `__tup` too.
        let reusable = scrutinee_local(&pair.value_term)
            .filter(|s| !pair.patterns.iter().any(|p| binds_name(p, s)))
            .map(str::to_owned);
        let Some(s) = reusable else {
            return self.elab_let_sigma_fresh(pair, expected, body);
        };
        self.destructure_sigma(pair.patterns, &pair.value_ty, &s, pair.span, body)
    }

    /// D2's non-variable scrutinee: `let __tup = value; redex`, the redex
    /// annotated with the expected type in check mode, and refused in infer
    /// mode when its type would name `__tup` (D3).
    fn elab_let_sigma_fresh(
        &mut self,
        pair: SigmaLet<'_>,
        expected: Option<&Type>,
        body: impl FnOnce(&mut Self) -> ElabResult<(Term, Type)>,
    ) -> ElabResult<(Term, Type)> {
        let SigmaLet {
            patterns,
            value_term,
            value_ty,
            span,
        } = pair;
        let tuple_var = self.fresh_var("__tup");
        let saved_depth = self.depth;
        self.env
            .bind_local(tuple_var.clone(), value_ty.clone(), self.depth);
        self.depth += 1;
        let destructured = self.destructure_sigma(patterns, &value_ty, &tuple_var, span, body);
        self.depth = saved_depth;
        let (redex, ty) = destructured?;
        let redex = match expected {
            Some(expected) => Term::annot(redex, expected.clone()),
            None if ty.free_term_vars().contains(&tuple_var) => {
                return Err(ElabError::cannot_infer(span).with_note(ESCAPING_WITNESS_NOTE));
            }
            None => redex,
        };
        Ok((Term::let_in(&tuple_var, value_ty, value_term, redex), ty))
    }

    /// Bind the two parts of `pair_ty` as locals, elaborate what they scope
    /// over, and emit `(λk:A. λe:B[x := k]. inner) (fst s) (snd s)`, a λ only
    /// for a part that binds a name (D2).
    fn destructure_sigma(
        &mut self,
        patterns: &[Pattern],
        pair_ty: &Type,
        s: &str,
        span: Span,
        body: impl FnOnce(&mut Self) -> ElabResult<(Term, Type)>,
    ) -> ElabResult<(Term, Type)> {
        let [first, second] = patterns else {
            return Err(ElabError::new(
                span,
                ElabErrorKind::ExpectedType {
                    expected: format!(
                        "a pattern of 2 parts, not {} (a dependent pair has two parts)",
                        patterns.len()
                    ),
                    found: pair_ty.clone(),
                },
            )
            .with_help(
                "take the nested parts apart with a nested pattern: `let (k, (p, q)) = h`",
            ));
        };
        let witness = self.witness_name(first, pair_ty)?;
        let evidence = self.evidence_name(second)?;
        let Some([witness_ty, evidence_ty]) = sigma_parts(pair_ty, first, s) else {
            return Err(not_a_dependent_pair(span, pair_ty));
        };

        let saved_depth = self.depth;
        let bound = [(&witness, &witness_ty), (&evidence, &evidence_ty)];
        for (name, ty) in bound.iter().filter_map(|(n, t)| Some((n.name()?, *t))) {
            self.env.bind_local(name.to_owned(), ty.clone(), self.depth);
            self.depth += 1;
        }
        let inner = match (second, evidence.name()) {
            (Pattern::Tuple(sub_patterns, _), Some(e)) => {
                self.destructure_nested(sub_patterns, &evidence_ty, e, span, body)
            }
            _ => body(self),
        };
        self.depth = saved_depth;
        let (mut term, mut ty) = inner?;

        if let Some(e) = evidence.name() {
            term = Term::lambda(e, evidence_ty, term);
        }
        if let Some(k) = witness.name() {
            term = Term::app(Term::lambda(k, witness_ty, term), Term::fst(Term::var(s)));
            ty = ty.substitute_term(k, &Term::fst(Term::var(s)));
        }
        if let Some(e) = evidence.name() {
            term = Term::app(term, Term::snd(Term::var(s)));
            ty = ty.substitute_term(e, &Term::snd(Term::var(s)));
        }
        Ok((term, ty))
    }

    /// The second part's nested pattern, over the λ-bound `e`: an inner
    /// `Sigma` is D2 again with `s := e`, an inner `Product` is today's
    /// `build_tuple_lets` on `e`. Neither makes a fresh `let` (D2's nesting).
    fn destructure_nested(
        &mut self,
        sub_patterns: &[Pattern],
        ty: &Type,
        e: &str,
        span: Span,
        body: impl FnOnce(&mut Self) -> ElabResult<(Term, Type)>,
    ) -> ElabResult<(Term, Type)> {
        if matches!(ty, Type::Sigma(..)) {
            return self.destructure_sigma(sub_patterns, ty, e, span, body);
        }
        let element_types = self.extract_tuple_types(ty, sub_patterns.len(), span)?;
        let bindings = self.collect_tuple_bindings(sub_patterns, &element_types, e, span)?;
        let saved_depth = self.depth;
        for (name, ty) in &bindings {
            self.env.bind_local(name.clone(), ty.clone(), self.depth);
            self.depth += 1;
        }
        let inner = body(self);
        self.depth = saved_depth;
        let (term, ty) = inner?;
        let term = self.build_tuple_lets(sub_patterns, &element_types, e, term, span)?;
        Ok((term, ty))
    }

    /// The first part: a name or `_` (D1). A name that the pair's own body
    /// mentions as a different variable would be captured by `B[x := k]`, so
    /// it is refused rather than silently rebound.
    fn witness_name(&self, first: &Pattern, pair_ty: &Type) -> ElabResult<PartName> {
        match first {
            Pattern::Wildcard(_) => Ok(PartName::Unbound),
            Pattern::Var(ident) if captures_in_evidence(pair_ty, &ident.name) => {
                Err(ElabError::new(
                    first.span(),
                    ElabErrorKind::UnsupportedPattern(format!(
                        "the witness name `{}` is already used in the pair's type",
                        ident.name
                    )),
                )
                .with_help("choose a name the pair's type does not mention"))
            }
            Pattern::Var(ident) => Ok(PartName::Named(ident.name.clone())),
            _ => Err(ElabError::new(
                first.span(),
                ElabErrorKind::UnsupportedPattern(
                    "the witness of a dependent pair is bound by a name or `_`".to_string(),
                ),
            )),
        }
    }

    /// The second part: a name, `_`, or a nested tuple bound to a fresh `__e`.
    fn evidence_name(&mut self, second: &Pattern) -> ElabResult<PartName> {
        match second {
            Pattern::Var(ident) => Ok(PartName::Named(ident.name.clone())),
            Pattern::Wildcard(_) => Ok(PartName::Unbound),
            Pattern::Tuple(..) => Ok(PartName::Named(self.fresh_var("e"))),
            _ => Err(ElabError::new(
                second.span(),
                ElabErrorKind::UnsupportedPattern(
                    "only variable, wildcard, and tuple patterns are supported in let bindings"
                        .to_string(),
                ),
            )),
        }
    }
}

/// Whether a pattern part binds `name` directly (a nested part binds a fresh
/// `__e`, which no user name can be).
fn binds_name(pattern: &Pattern, name: &str) -> bool {
    matches!(pattern, Pattern::Var(ident) if ident.name == name)
}

/// Whether `name` is free in `Σx:A. B`'s body as a variable other than `x`.
fn captures_in_evidence(pair_ty: &Type, name: &str) -> bool {
    let Type::Sigma(binder, _, evidence_ty) = pair_ty else {
        return false;
    };
    binder != name && evidence_ty.free_term_vars().contains(name)
}

/// Defensive: both callers test for a `Sigma` before reaching here.
fn not_a_dependent_pair(span: Span, found: &Type) -> ElabError {
    ElabError::new(
        span,
        ElabErrorKind::ExpectedType {
            expected: "a dependent pair".to_string(),
            found: found.clone(),
        },
    )
}
