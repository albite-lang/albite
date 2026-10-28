//! Capture-avoiding substitution for motive instantiation (ADR 29.9.26d D3).
//!
//! [`Term::substitute`] never renames a binder. Instantiating a `natind`
//! motive substitutes an OPEN term (`succ(j)`, or the target) into a body that
//! may bind names of its own, so before substituting, every binder that would
//! capture a free variable of the replacement — and only such a binder, so an
//! uncaptured term keeps the spelling a user's annotation restates — is
//! renamed to a spelling the lexer cannot produce.
//!
//! The `Type` half lives here too: a type mentions terms only inside `Eq`
//! arguments, so `Type::substitute_term` substitutes there and nowhere else.

use std::collections::HashSet;

use crate::terms::{Term, Var};
use crate::types::Type;

impl Type {
    /// `self[var := replacement]`, substituting inside every `Eq` argument.
    ///
    /// Capture-avoiding: a binder inside an `Eq` argument that would capture a
    /// free variable of `replacement` is renamed first, and an inner `natind`
    /// whose binder is `var` shadows it (both via [`Term::substitute_avoiding`]).
    #[must_use]
    pub fn substitute_term(&self, var: &str, replacement: &Term) -> Type {
        match self {
            Type::Eq(ty, left, right) => Type::eq(
                ty.substitute_term(var, replacement),
                left.substitute_avoiding(var, replacement),
                right.substitute_avoiding(var, replacement),
            ),
            Type::Pi(binder, dom, cod) => {
                substitute_under_binder(Type::pi, (binder, dom, cod), var, replacement)
            }
            Type::Sigma(binder, dom, cod) => {
                substitute_under_binder(Type::sigma, (binder, dom, cod), var, replacement)
            }
            // A predicate's arguments are terms, as `Eq`'s are (ADR 6.10.26h D4)
            Type::PredApp(pred, args) => Type::pred_app(
                pred.clone(),
                args.iter()
                    .map(|arg| arg.substitute_avoiding(var, replacement))
                    .collect(),
            ),
            Type::PredAbs(params, body) => substitute_under_params(params, body, var, replacement),
            _ => self.map_children(|child| child.substitute_term(var, replacement)),
        }
    }

    /// The free term variables of every `Eq` argument in `self`. A motive is
    /// *dependent* exactly when its binder is in this set (ADR 29.9.26d D4).
    #[must_use]
    pub fn free_term_vars(&self) -> HashSet<Var> {
        let mut free = HashSet::new();
        collect_free_term_vars(self, &mut free);
        free
    }
}

/// `Pi(y, A, B)[x := r]` (ADR 30.9.26d D3), and `Sigma` the same way (ADR
/// 4.10.26h): the domain is outside `y`'s scope and always substituted;
/// `y == x` shadows the body; a `y` free in `r` that would capture a free `x`
/// is renamed first. `rebuild` is the former's constructor.
fn substitute_under_binder(
    rebuild: fn(String, Type, Type) -> Type,
    (binder, dom, cod): (&str, &Type, &Type),
    var: &str,
    replacement: &Term,
) -> Type {
    let dom = dom.substitute_term(var, replacement);
    if binder == var {
        return rebuild(binder.to_owned(), dom, cod.clone());
    }
    let replacement_free = replacement.free_vars();
    let cod_free = cod.free_term_vars();
    if replacement_free.contains(binder) && cod_free.contains(var) {
        let mut taken = replacement_free;
        taken.extend(cod_free);
        let fresh = fresh_name(binder, &taken);
        let renamed = cod.substitute_term(binder, &Term::Var(fresh.clone()));
        return rebuild(fresh, dom, renamed.substitute_term(var, replacement));
    }
    rebuild(
        binder.to_owned(),
        dom,
        cod.substitute_term(var, replacement),
    )
}

/// `PredAbs(xs, T)[x := r]` (ADR 6.10.26h D4): `Pi`'s rule over a list of
/// binders. The domains are outside every `xᵢ`'s scope; an `xᵢ == x` shadows
/// the body; an `xᵢ` free in `r` that would capture a free `x` is renamed.
fn substitute_under_params(
    params: &[(Var, Type)],
    body: &Type,
    var: &str,
    replacement: &Term,
) -> Type {
    let mut params: Vec<(Var, Type)> = params
        .iter()
        .map(|(x, dom)| (x.clone(), dom.substitute_term(var, replacement)))
        .collect();
    if params.iter().any(|(x, _)| x == var) {
        return Type::pred_abs(params, body.clone());
    }
    let replacement_free = replacement.free_vars();
    let mut body = body.clone();
    if body.free_term_vars().contains(var) {
        for (param, _) in &mut params {
            if replacement_free.contains(param.as_str()) {
                let mut taken = replacement_free.clone();
                taken.extend(body.free_term_vars());
                let fresh = fresh_name(param, &taken);
                body = body.substitute_term(param, &Term::Var(fresh.clone()));
                *param = fresh;
            }
        }
    }
    let body = body.substitute_term(var, replacement);
    Type::pred_abs(params, body)
}

/// `fv(Pi(y, A, B)) = fv(A) ∪ (fv(B) \ {y})`, the same for `Sigma` and, over
/// its list of binders, `PredAbs`; elsewhere, the `Eq` and `PredApp`
/// arguments.
fn collect_free_term_vars(ty: &Type, free: &mut HashSet<Var>) {
    if let Type::PredAbs(params, body) = ty {
        let mut body_free = body.free_term_vars();
        for (param, dom) in params {
            collect_free_term_vars(dom, free);
            body_free.remove(param);
        }
        free.extend(body_free);
        return;
    }
    if let Type::PredApp(_, args) = ty {
        for arg in args {
            free.extend(arg.free_vars());
        }
        return;
    }
    if let Type::Pi(binder, dom, cod) | Type::Sigma(binder, dom, cod) = ty {
        collect_free_term_vars(dom, free);
        let mut cod_free = cod.free_term_vars();
        cod_free.remove(binder);
        free.extend(cod_free);
        return;
    }
    if let Type::Eq(_, left, right) = ty {
        free.extend(left.free_vars());
        free.extend(right.free_vars());
    }
    for child in ty.children() {
        collect_free_term_vars(child, free);
    }
}

/// The suffix a renamed binder gains: a prime, which no identifier can spell.
const RENAME_MARK: char = '′';

/// `base` with primes appended until it is not in `taken`.
pub(crate) fn fresh_name(base: &str, taken: &HashSet<String>) -> String {
    let mut candidate = format!("{base}{RENAME_MARK}");
    while taken.contains(&candidate) {
        candidate.push(RENAME_MARK);
    }
    candidate
}

impl Term {
    /// Substitute `var := replacement`, renaming any binder of `self` that
    /// would capture a free variable of `replacement` (ADR 29.9.26d D3).
    #[must_use]
    pub fn substitute_avoiding(&self, var: &str, replacement: &Term) -> Term {
        let avoid = replacement.free_vars();
        if avoid.is_empty() {
            return self.substitute(var, replacement);
        }
        let mut renamed = self.clone();
        rename_capturing_binders(&mut renamed, var, &avoid);
        renamed.substitute(var, replacement)
    }
}

/// Rename, in place, every binder named in `avoid` whose scope holds a free
/// `var` — exactly the binders a substitution for `var` would reach through.
/// A `natind` motive binder is not handled here: `Term::substitute` renames it
/// itself when it enters the motive.
fn rename_capturing_binders(term: &mut Term, var: &str, avoid: &HashSet<String>) {
    match term {
        Term::Lambda(binder, _, body) | Term::Fix(binder, _, body) => {
            rebind(binder, body, var, avoid);
        }
        Term::Let(binder, _, def, body) => {
            rename_capturing_binders(def, var, avoid);
            rebind(binder, body, var, avoid);
        }
        Term::Case(scrutinee, left_binder, left, right_binder, right) => {
            rename_capturing_binders(scrutinee, var, avoid);
            rebind(left_binder, left, var, avoid);
            rebind(right_binder, right, var, avoid);
        }
        Term::AdtMatch(scrutinee, arms) => {
            rename_capturing_binders(scrutinee, var, avoid);
            for (_, binder, body) in arms.iter_mut() {
                rebind(binder, body, var, avoid);
            }
        }
        _ => {
            for child in term.subterms_mut() {
                rename_capturing_binders(child, var, avoid);
            }
        }
    }
}

/// One binder and its scope: rename the binder if it would capture, then
/// carry on into the scope. A binder equal to `var` shadows it, so nothing
/// below is substituted and nothing needs renaming.
fn rebind(binder: &mut String, body: &mut Term, var: &str, avoid: &HashSet<String>) {
    if binder == var {
        return;
    }
    if avoid.contains(binder.as_str()) && body.free_vars().contains(var) {
        let mut taken = avoid.clone();
        taken.extend(body.free_vars());
        let fresh = fresh_name(binder, &taken);
        *body = body.substitute(binder, &Term::Var(fresh.clone()));
        *binder = fresh;
    }
    rename_capturing_binders(body, var, avoid);
}

impl Type {
    /// Rewrite every `Pi(x, a, b)` to `Arrow(a′, b′)` (ADR 30.9.26d D6) and
    /// every `Sigma(x, a, b)` to `Product(a′, b′)` (ADR 4.10.26h D6),
    /// recursively; `Eq` is left untouched. Codegen erases an `Eq` value to
    /// `Unit` whatever its terms mention, so the erased former has the
    /// dependent one's layout.
    #[must_use]
    pub fn erase_dependency(&self) -> Type {
        match self {
            Type::Pi(_, dom, cod) => Type::arrow(dom.erase_dependency(), cod.erase_dependency()),
            Type::Sigma(_, dom, cod) => {
                Type::product(dom.erase_dependency(), cod.erase_dependency())
            }
            // A predicate binder erases to an ordinary type-generic one, so
            // codegen sees `∀P. …` with `P` used as a type (ADR 6.10.26h D7).
            Type::PredApp(pred, _) => Type::TyVar(pred.clone()),
            Type::ForallPred(pred, _, body) => Type::forall(pred.clone(), body.erase_dependency()),
            Type::PredAbs(_, body) => body.erase_dependency(),
            _ => self.map_children(Type::erase_dependency),
        }
    }

    /// Whether a `Pi`, a `Sigma` or a predicate form occurs anywhere in
    /// `self`, `Eq` arguments excluded — what [`Type::erase_dependency`]
    /// would rewrite.
    #[must_use]
    pub fn contains_dependent_former(&self) -> bool {
        matches!(
            self,
            Type::Pi(..)
                | Type::Sigma(..)
                | Type::PredApp(..)
                | Type::ForallPred(..)
                | Type::PredAbs(..)
        ) || self
            .children()
            .into_iter()
            .any(Type::contains_dependent_former)
    }
}

impl Term {
    /// [`Type::erase_dependency`] applied to every type embedded anywhere in
    /// the term: annotations, motives, type arguments (ADR 30.9.26d D6).
    #[must_use]
    pub fn erase_dependency(&self) -> Term {
        let mut erased = self.clone();
        erase_embedded_types(&mut erased);
        erased
    }
}

fn erase_embedded_types(term: &mut Term) {
    for ty in embedded_types_mut(term) {
        *ty = ty.erase_dependency();
    }
    for child in term.subterms_mut() {
        erase_embedded_types(child);
    }
}

/// Every `Type` stored directly on `term`, mutably. Exhaustive with no
/// wildcard, so a future variant carrying a type fails to compile here.
fn embedded_types_mut(term: &mut Term) -> Vec<&mut Type> {
    match term {
        Term::Lambda(_, ty, _)
        | Term::Fix(_, ty, _)
        | Term::Let(_, ty, _, _)
        | Term::Absurd(ty, _)
        | Term::Inl(ty, _)
        | Term::Inr(ty, _)
        | Term::Refl(ty, _)
        | Term::Fold(ty, _)
        | Term::Unfold(ty, _)
        | Term::NatRec(ty, _, _, _)
        | Term::NatInd(_, ty, _, _, _)
        | Term::TyApp(_, ty)
        | Term::Annot(_, ty)
        | Term::AdtConstruct(ty, _, _) => vec![ty],
        Term::Subst(base, motive, _, _) => vec![base, motive],
        Term::Var(_)
        | Term::Global(_)
        | Term::App(_, _)
        | Term::True
        | Term::False
        | Term::If(_, _, _)
        | Term::Unit
        | Term::Zero
        | Term::Succ(_)
        | Term::NatLit(_)
        | Term::NatAdd(_, _)
        | Term::NatSub(_, _)
        | Term::NatMul(_, _)
        | Term::NatDiv(_, _)
        | Term::NatMod(_, _)
        | Term::NatEq(_, _)
        | Term::NatLt(_, _)
        | Term::NatLe(_, _)
        | Term::NatGt(_, _)
        | Term::NatGe(_, _)
        | Term::IntLit(_)
        | Term::IntBin(_, _, _)
        | Term::IntNeg(_)
        | Term::NatToInt(_)
        | Term::IntToNat(_)
        | Term::Region(..)
        | Term::CallFrame(_)
        | Term::BoolAnd(_, _)
        | Term::BoolOr(_, _)
        | Term::BoolNot(_)
        | Term::StringLit(_)
        | Term::StrConcat(_, _)
        | Term::StrLen(_)
        | Term::StrEq(_, _)
        | Term::StrCharAt(_, _)
        | Term::StrSubstring(_, _, _)
        | Term::Pair(_, _)
        | Term::Fst(_)
        | Term::Snd(_)
        | Term::Case(_, _, _, _, _)
        | Term::TyAbs(_, _)
        | Term::ExternCall(_, _)
        | Term::RefNew(_)
        | Term::RefGet(_)
        | Term::RefSet(_, _)
        | Term::Sorry
        | Term::AdtMatch(_, _)
        | Term::Return(_)
        | Term::Spanned(_, _) => Vec::new(),
    }
}

/// Two `Pi` codomains with both binders renamed to one fresh name, ready for a
/// syntactic comparison (ADR 30.9.26d D4). The α environment of type equality
/// is not consulted for TERM variables, so the binder pair is unified here.
pub(crate) fn pi_codomains_unified(
    (left_binder, left_cod): (&str, &Type),
    (right_binder, right_cod): (&str, &Type),
) -> (Type, Type) {
    if left_binder == right_binder {
        return (left_cod.clone(), right_cod.clone());
    }
    let mut taken = left_cod.free_term_vars();
    taken.extend(right_cod.free_term_vars());
    taken.insert(left_binder.to_owned());
    taken.insert(right_binder.to_owned());
    let shared = Term::Var(fresh_name(left_binder, &taken));
    (
        left_cod.substitute_term(left_binder, &shared),
        right_cod.substitute_term(right_binder, &shared),
    )
}
