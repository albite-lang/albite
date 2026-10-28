//! `ring`'s normal form (ADR 4.10.26g D2). Pure: functions over Core terms.
//!
//! A side of the goal is read as a polynomial over **atoms**: every subterm
//! that is not `+`, `*` or a numeral, compared after `strip_spans()`. Atoms
//! are ordered by first appearance, pre-order over the left side then the
//! right, so the order is fixed by the goal alone. A monomial is a multiset
//! of atom indices; the polynomial maps each monomial to its coefficient and
//! keeps the numeral summands as one constant.
//!
//! A `pow(x, e)` is an atom like any other application; one at a numeral `e`
//! is unfolded before the goal is read (`power.rs`, ADR 9.10.26j D4).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use albite_core::Term;

/// What `ring` sees at one node: a sum, a product, a numeral or an atom.
pub(crate) enum Shape<'a> {
    Add(&'a Term, &'a Term),
    Mul(&'a Term, &'a Term),
    Numeral(u64),
    Atom,
}

/// `term`'s shape, looking through spans.
pub(crate) fn shape(term: &Term) -> Shape<'_> {
    match term {
        Term::Spanned(inner, _) => shape(inner),
        Term::NatAdd(left, right) => Shape::Add(left, right),
        Term::NatMul(left, right) => Shape::Mul(left, right),
        _ => numeral_value(term).map_or(Shape::Atom, Shape::Numeral),
    }
}

/// The value of a numeral — `zero`, `succ` of a numeral, or a literal — or
/// `None`. The elaborator spells `2` as `succ (succ zero)` (P0.2).
pub(crate) fn numeral_value(term: &Term) -> Option<u64> {
    match term {
        Term::Spanned(inner, _) => numeral_value(inner),
        Term::Zero => Some(0),
        Term::NatLit(value) => Some(*value),
        Term::Succ(inner) => numeral_value(inner)?.checked_add(1),
        _ => None,
    }
}

/// The atoms of a goal, in their fixed order. An atom keeps its index for
/// the whole plan; one a rewrite creates (inside an atom) is appended.
#[derive(Debug, Default, Clone)]
pub(crate) struct AtomOrder {
    atoms: Vec<Term>,
}

impl AtomOrder {
    /// `atom`'s index, if it has one.
    pub(crate) fn index(&self, atom: &Term) -> Option<usize> {
        let atom = atom.strip_spans();
        self.atoms.iter().position(|known| *known == atom)
    }

    /// Append every atom of `term` not yet known, in pre-order. The walk
    /// stops at an atom: what is inside one is not `ring`'s.
    pub(crate) fn extend_from(&mut self, term: &Term) {
        match shape(term) {
            Shape::Add(left, right) | Shape::Mul(left, right) => {
                self.extend_from(left);
                self.extend_from(right);
            }
            Shape::Numeral(_) => {}
            Shape::Atom => {
                if self.index(term).is_none() {
                    self.atoms.push(term.strip_spans());
                }
            }
        }
    }

    /// The atom at `index`.
    pub(crate) fn atom(&self, index: usize) -> &Term {
        &self.atoms[index]
    }
}

/// A monomial: its atoms' indices, sorted, one per factor.
pub(crate) type Monomial = Vec<usize>;

/// D2's monomial order: by degree, then lexicographically by atom order.
pub(crate) fn monomial_order(left: &Monomial, right: &Monomial) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

/// A side as a polynomial: each monomial's coefficient, and the constant.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Polynomial {
    pub(crate) monomials: BTreeMap<Monomial, u128>,
    pub(crate) constant: u128,
}

impl Polynomial {
    fn atom(index: usize) -> Self {
        Polynomial {
            monomials: BTreeMap::from([(vec![index], 1)]),
            constant: 0,
        }
    }

    fn constant(value: u64) -> Self {
        Polynomial {
            monomials: BTreeMap::new(),
            constant: u128::from(value),
        }
    }

    fn add(mut self, other: Polynomial) -> Self {
        for (monomial, coefficient) in other.monomials {
            let entry = self.monomials.entry(monomial).or_insert(0);
            *entry = entry.saturating_add(coefficient);
        }
        self.constant = self.constant.saturating_add(other.constant);
        self
    }

    fn mul(&self, other: &Polynomial) -> Self {
        // Each side's constant joins its monomials as the empty monomial.
        let terms = |p: &Polynomial| -> Vec<(Monomial, u128)> {
            let mut terms: Vec<(Monomial, u128)> =
                p.monomials.iter().map(|(m, c)| (m.clone(), *c)).collect();
            if p.constant > 0 {
                terms.push((Vec::new(), p.constant));
            }
            terms
        };
        let mut product = Polynomial::default();
        for (left, left_coefficient) in terms(self) {
            for (right, right_coefficient) in terms(other) {
                let coefficient = left_coefficient.saturating_mul(right_coefficient);
                let mut monomial: Monomial = left.iter().chain(&right).copied().collect();
                monomial.sort_unstable();
                product = product.add(if monomial.is_empty() {
                    Polynomial {
                        monomials: BTreeMap::new(),
                        constant: coefficient,
                    }
                } else {
                    Polynomial {
                        monomials: BTreeMap::from([(monomial, coefficient)]),
                        constant: 0,
                    }
                });
            }
        }
        product
    }
}

/// `term` as a polynomial over `atoms`, or `None` when an atom of it is not
/// in `atoms` — which a caller that extended `atoms` from `term` never sees.
pub(crate) fn polynomial(term: &Term, atoms: &AtomOrder) -> Option<Polynomial> {
    Some(match shape(term) {
        Shape::Add(left, right) => polynomial(left, atoms)?.add(polynomial(right, atoms)?),
        Shape::Mul(left, right) => polynomial(left, atoms)?.mul(&polynomial(right, atoms)?),
        Shape::Numeral(value) => Polynomial::constant(value),
        Shape::Atom => Polynomial::atom(atoms.index(term)?),
    })
}

/// The largest coefficient [`render`] spells as copies; past it, a monomial
/// is spelled with its coefficient on the right, `m * c`, so a message about
/// `n * 1000` stays one line.
pub(crate) const RENDERED_COPIES_MAX: u128 = 16;

/// `p` in D2's spelling: monomials in [`monomial_order`], a coefficient as
/// that many copies (spelling (i)) up to [`RENDERED_COPIES_MAX`], factors in
/// atom order, the constant last; `0` when empty.
pub(crate) fn render(p: &Polynomial, atoms: &AtomOrder) -> String {
    let mut monomials: Vec<(&Monomial, &u128)> = p.monomials.iter().collect();
    monomials.sort_by(|(a, _), (b, _)| monomial_order(a, b));
    let mut summands = Vec::new();
    for (monomial, coefficient) in monomials {
        let factors: Vec<String> = monomial
            .iter()
            .map(|index| display_term(atoms.atom(*index)))
            .collect();
        let rendered = factors.join(" * ");
        if *coefficient > RENDERED_COPIES_MAX {
            summands.push(format!("{rendered} * {coefficient}"));
        } else {
            for _ in 0..*coefficient {
                summands.push(rendered.clone());
            }
        }
    }
    if p.constant > 0 || summands.is_empty() {
        summands.push(p.constant.to_string());
    }
    summands.join(" + ")
}

/// `term` as a message prints it, each numeral as its decimal value: the
/// elaborator's `succ (succ zero)` reads as `2`.
pub(crate) fn display_term(term: &Term) -> String {
    literal_numerals(term).to_string()
}

fn literal_numerals(term: &Term) -> Term {
    if let Some(value) = numeral_value(term) {
        return Term::NatLit(value);
    }
    let mut rebuilt = term.strip_spans();
    for child in rebuilt.subterms_mut() {
        *child = literal_numerals(child);
    }
    rebuilt
}

/// The key a summand sorts by (D2): a monomial by degree then atom order,
/// the constant last. `None` for a summand that is not yet a product of
/// atoms or a numeral — an earlier phase still has work there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SummandKey {
    Monomial(usize, Monomial),
    Constant,
}

/// `term`'s [`SummandKey`] over `atoms`.
pub(crate) fn summand_key(term: &Term, atoms: &AtomOrder) -> Option<SummandKey> {
    if numeral_value(term).is_some() {
        return Some(SummandKey::Constant);
    }
    let mut factors = Vec::new();
    collect_factors(term, atoms, &mut factors)?;
    factors.sort_unstable();
    Some(SummandKey::Monomial(factors.len(), factors))
}

fn collect_factors(term: &Term, atoms: &AtomOrder, factors: &mut Vec<usize>) -> Option<()> {
    match shape(term) {
        Shape::Mul(left, right) => {
            collect_factors(left, atoms, factors)?;
            collect_factors(right, atoms, factors)
        }
        Shape::Atom => {
            factors.push(atoms.index(term)?);
            Some(())
        }
        Shape::Add(..) | Shape::Numeral(_) => None,
    }
}
