//! `omega`'s arithmetic over `ring`'s normal forms (ADR 6.10.26f D3, D7).
//! Pure.
//!
//! A difference of two normal forms has integer coefficients: [`Signed`]
//! holds one, decides whether it is a witness, and renders it for D7's
//! second message. A witness, once non-negative, is spelled back as a Core
//! term by [`polynomial_term`], in D2's order, so `ring` has the least to do.

use std::collections::BTreeMap;

use albite_core::Term;

use crate::elaborate::tactics::procedures::ring::normal_form::{
    display_term, monomial_order, AtomOrder, Monomial, Polynomial, RENDERED_COPIES_MAX,
};

/// A polynomial with integer coefficients: a difference of two normal forms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Signed {
    pub(crate) monomials: BTreeMap<Monomial, i128>,
    pub(crate) constant: i128,
}

fn signed(coefficient: u128) -> i128 {
    i128::try_from(coefficient).unwrap_or(i128::MAX)
}

impl Signed {
    /// `left − right`.
    pub(crate) fn difference(left: &Polynomial, right: &Polynomial) -> Self {
        let mut out = Signed {
            monomials: BTreeMap::new(),
            constant: signed(left.constant) - signed(right.constant),
        };
        for (monomial, coefficient) in &left.monomials {
            *out.monomials.entry(monomial.clone()).or_insert(0) += signed(*coefficient);
        }
        for (monomial, coefficient) in &right.monomials {
            *out.monomials.entry(monomial.clone()).or_insert(0) -= signed(*coefficient);
        }
        out.monomials.retain(|_, coefficient| *coefficient != 0);
        out
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.monomials.is_empty() && self.constant == 0
    }

    pub(crate) fn negated(&self) -> Self {
        Signed {
            monomials: self
                .monomials
                .iter()
                .map(|(m, c)| (m.clone(), -c))
                .collect(),
            constant: -self.constant,
        }
    }

    /// The summands in D2's order — monomials, then the constant — each
    /// with its coefficient.
    fn summands(&self) -> Vec<(Option<&Monomial>, i128)> {
        let mut monomials: Vec<(&Monomial, &i128)> = self.monomials.iter().collect();
        monomials.sort_by(|(a, _), (b, _)| monomial_order(a, b));
        let mut summands: Vec<(Option<&Monomial>, i128)> =
            monomials.into_iter().map(|(m, c)| (Some(m), *c)).collect();
        if self.constant != 0 {
            summands.push((None, self.constant));
        }
        summands
    }

    /// The first negative summand, rendered with its sign, or `None`.
    pub(crate) fn first_negative(&self, atoms: &AtomOrder) -> Option<String> {
        self.summands()
            .into_iter()
            .find(|(_, coefficient)| *coefficient < 0)
            .map(|(monomial, coefficient)| format!("-{}", summand(monomial, -coefficient, atoms)))
    }

    /// `n * n - n`, `-1`, `0`: the summands joined by their signs.
    pub(crate) fn render(&self, atoms: &AtomOrder) -> String {
        let mut out = String::new();
        for (monomial, coefficient) in self.summands() {
            let body = summand(monomial, coefficient.abs(), atoms);
            let sign = match (out.is_empty(), coefficient < 0) {
                (true, true) => "-",
                (true, false) => "",
                (false, true) => " - ",
                (false, false) => " + ",
            };
            out.push_str(sign);
            out.push_str(&body);
        }
        if out.is_empty() {
            out.push('0');
        }
        out
    }
}

/// One summand's magnitude: `n * m`, `n * m * 3`, or a numeral.
fn summand(monomial: Option<&Monomial>, magnitude: i128, atoms: &AtomOrder) -> String {
    let Some(monomial) = monomial else {
        return magnitude.to_string();
    };
    let factors: Vec<String> = monomial
        .iter()
        .map(|index| display_term(atoms.atom(*index)))
        .collect();
    let factors = factors.join(" * ");
    if magnitude == 1 {
        factors
    } else {
        format!("{factors} * {magnitude}")
    }
}

/// `value` as a numeral, spelled as the elaborator spells one.
pub(crate) fn numeral(value: u128) -> Term {
    (0..value).fold(Term::Zero, |acc, _| Term::succ(acc))
}

/// A non-negative polynomial as a term: monomials in D2's order, right
/// nested, a coefficient as copies up to `ring`'s render bound, the constant
/// last; `0` when empty.
pub(crate) fn polynomial_term(p: &Polynomial, atoms: &AtomOrder) -> Term {
    let mut monomials: Vec<(&Monomial, &u128)> = p.monomials.iter().collect();
    monomials.sort_by(|(a, _), (b, _)| monomial_order(a, b));
    let mut summands = Vec::new();
    for (monomial, coefficient) in monomials {
        let product = monomial
            .iter()
            .rev()
            .map(|index| atoms.atom(*index).clone())
            .reduce(|right, left| Term::nat_mul(left, right))
            .unwrap_or_else(|| numeral(1));
        if *coefficient > RENDERED_COPIES_MAX {
            summands.push(Term::nat_mul(product, numeral(*coefficient)));
        } else {
            summands.extend((0..*coefficient).map(|_| product.clone()));
        }
    }
    if p.constant > 0 {
        summands.push(numeral(p.constant));
    }
    summands
        .into_iter()
        .rev()
        .reduce(|right, left| Term::nat_add(left, right))
        .unwrap_or(Term::Zero)
}
