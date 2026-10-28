//! Natural number operations compilation.
//!
//! Handles compilation of Nat arithmetic and comparisons:
//! - Arithmetic: `NatAdd`, `NatSub`, `NatMul`, `NatDiv`, `NatMod`
//! - Comparisons: `NatEq`, `NatLt`, `NatLe`, `NatGt`, `NatGe`

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

/// What [`CodeGen::nonzero_divisor`] made of a `Nat` divisor.
enum NatDivisor<'ctx> {
    /// A constant zero: the quotient and the remainder are both this `0`.
    Zero(IntValue<'ctx>),
    /// A constant non-zero divisor, safe to divide by as it is.
    NonZero(IntValue<'ctx>),
    /// A run-time divisor: `is_zero` is `b == 0` and `divisor` is
    /// `select(is_zero, 1, b)`.
    Guarded {
        is_zero: IntValue<'ctx>,
        divisor: IntValue<'ctx>,
    },
}

impl<'ctx> CodeGen<'ctx> {
    // ═══════════════════════════════════════════════════════════════════
    // Arithmetic Operations
    // ═══════════════════════════════════════════════════════════════════

    /// Compile `a + b` for natural numbers.
    pub(crate) fn compile_nat_add(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_add(a, b, "nat_add")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Compile saturating `a - b` for natural numbers.
    ///
    /// Returns `max(a - b, 0)` to handle underflow.
    pub(crate) fn compile_nat_sub(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let diff = self
            .builder
            .build_int_sub(a, b, "nat_sub_raw")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let underflow = self
            .builder
            .build_int_compare(IntPredicate::UGT, b, a, "underflow")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let zero = self.context.i64_type().const_zero();
        let result = self
            .builder
            .build_select(underflow, zero, diff, "nat_sub")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result)
    }

    /// Compile `a * b` for natural numbers.
    pub(crate) fn compile_nat_mul(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_mul(a, b, "nat_mul")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Compile `a / b` for natural numbers (unsigned division).
    ///
    /// `n / 0` is `0`, as the evaluator and the kernel say (ADR 9.10.26g D1):
    /// the divisor is made non-zero before `udiv`, then a zero divisor
    /// selects `0`, because `a / 1` is `a`.
    pub(crate) fn compile_nat_div(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let (divisor, is_zero) = match self.nonzero_divisor(b)? {
            NatDivisor::Zero(zero) => return Ok(zero.into()),
            NatDivisor::NonZero(divisor) => (divisor, None),
            NatDivisor::Guarded { is_zero, divisor } => (divisor, Some(is_zero)),
        };
        let quotient = self
            .builder
            .build_int_unsigned_div(a, divisor, "nat_div")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let Some(is_zero) = is_zero else {
            return Ok(quotient.into());
        };
        let zero = self.context.i64_type().const_zero();
        self.builder
            .build_select(is_zero, zero, quotient, "nat_div_guarded")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))
    }

    /// Compile `a % b` for natural numbers (unsigned remainder).
    ///
    /// `n % 0` is `0` (ADR 9.10.26g D1). No outer `select` is needed: the
    /// guard divides by `1` instead, and `a % 1` is already `0`.
    pub(crate) fn compile_nat_mod(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let divisor = match self.nonzero_divisor(b)? {
            NatDivisor::Zero(zero) => return Ok(zero.into()),
            NatDivisor::NonZero(divisor) | NatDivisor::Guarded { divisor, .. } => divisor,
        };
        let result = self
            .builder
            .build_int_unsigned_rem(a, divisor, "nat_mod")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Make a `Nat` divisor safe for `udiv`/`urem` (ADR 9.10.26g D2).
    ///
    /// A constant divisor is decided here, at build time, because user modules
    /// get no middle-end pass to fold the guard away: non-zero stays bare (the
    /// native core's `write_decimal` divides by `10` and pays nothing), and
    /// zero is the answer `0` itself — which also stops the builder folding
    /// `udiv 7, 0` to poison. A run-time divisor gets `select(b == 0, 1, b)`,
    /// branchless so the CFG the musttail shapes see is unchanged. `freeze`
    /// would not do: a zero divisor is immediate UB, not poison.
    fn nonzero_divisor(&self, b: IntValue<'ctx>) -> Result<NatDivisor<'ctx>, CodeGenError> {
        match b.get_zero_extended_constant() {
            Some(0) => return Ok(NatDivisor::Zero(b.get_type().const_zero())),
            Some(_) => return Ok(NatDivisor::NonZero(b)),
            None => {}
        }
        let is_zero = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                b,
                b.get_type().const_zero(),
                "nat_divisor_is_zero",
            )
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let divisor = self
            .builder
            .build_select(is_zero, b.get_type().const_int(1, false), b, "nat_divisor")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?
            .into_int_value();
        Ok(NatDivisor::Guarded { is_zero, divisor })
    }

    // ═══════════════════════════════════════════════════════════════════
    // Comparison Operations
    // ═══════════════════════════════════════════════════════════════════

    /// Compile `a == b` for natural numbers.
    pub(crate) fn compile_nat_eq(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_compare(IntPredicate::EQ, a, b, "nat_eq")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Compile `a < b` for natural numbers.
    pub(crate) fn compile_nat_lt(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_compare(IntPredicate::ULT, a, b, "nat_lt")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Compile `a <= b` for natural numbers.
    pub(crate) fn compile_nat_le(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_compare(IntPredicate::ULE, a, b, "nat_le")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Compile `a > b` for natural numbers.
    pub(crate) fn compile_nat_gt(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_compare(IntPredicate::UGT, a, b, "nat_gt")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }

    /// Compile `a >= b` for natural numbers.
    pub(crate) fn compile_nat_ge(
        &self,
        a: IntValue<'ctx>,
        b: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let result = self
            .builder
            .build_int_compare(IntPredicate::UGE, a, b, "nat_ge")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(result.into())
    }
}

// Tests: nat_ops_tests.rs — the IR shape of the guarded division (ADR 9.10.26g).
#[cfg(test)]
#[path = "nat_ops_tests.rs"]
mod tests;
