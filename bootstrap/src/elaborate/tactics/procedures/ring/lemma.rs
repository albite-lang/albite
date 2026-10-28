//! The library lemmas `ring` cites (ADR 4.10.26g D3, 9.10.26j D4).

/// The library of the semiring laws.
pub(crate) const ARITH_LIBRARY: &str = "nat_arith";

/// The library of `pow` and its two recurrences (9.10.26j D4).
pub(crate) const POW_LIBRARY: &str = "nat_pow";

/// A lemma `ring` cites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lemma {
    AddZeroRight,
    AddZeroLeft,
    AddAssoc,
    AddComm,
    AddLeftComm,
    MulZeroRight,
    MulZeroLeft,
    MulOneRight,
    MulOneLeft,
    MulSuccRight,
    MulComm,
    MulLeftComm,
    MulAdd,
    AddMul,
    MulAssoc,
    PowZero,
    PowSucc,
}

impl Lemma {
    /// The lemma's name in its library.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Lemma::AddZeroRight => "add_zero_right",
            Lemma::AddZeroLeft => "add_zero_left",
            Lemma::AddAssoc => "add_assoc",
            Lemma::AddComm => "add_comm",
            Lemma::AddLeftComm => "add_left_comm",
            Lemma::MulZeroRight => "mul_zero_right",
            Lemma::MulZeroLeft => "mul_zero_left",
            Lemma::MulOneRight => "mul_one_right",
            Lemma::MulOneLeft => "mul_one_left",
            Lemma::MulSuccRight => "mul_succ_right",
            Lemma::MulComm => "mul_comm",
            Lemma::MulLeftComm => "mul_left_comm",
            Lemma::MulAdd => "mul_add",
            Lemma::AddMul => "add_mul",
            Lemma::MulAssoc => "mul_assoc",
            Lemma::PowZero => "pow_zero",
            Lemma::PowSucc => "pow_succ",
        }
    }

    /// The library the lemma is cited from: `pow`'s two from `nat_pow`,
    /// where `pow` is defined, the rest from `nat_arith`.
    pub(crate) fn library(self) -> &'static str {
        match self {
            Lemma::PowZero | Lemma::PowSucc => POW_LIBRARY,
            _ => ARITH_LIBRARY,
        }
    }
}
