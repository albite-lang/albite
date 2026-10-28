//! Packing a small tagged-union blob into the integer it travels as on a
//! `$direct_mt` entry, and back (ADR 2.10.26h D1, D2).
//!
//! Both directions go through one entry-block scratch slot of the word type,
//! named `sum_word` and aligned 8 — never `indirect_buf` or `sret`, so the IR
//! audits that count buffers never count a word. Packing stores `0` first: a
//! blob with a 1-to-3-byte payload has undefined tail padding, and zeroing
//! makes the word a pure function of the value. The memory round-trip is
//! independent of byte order (both ends run on one target), and SROA folds it
//! at `-O2`. These two helpers serve every site — the shim, the callee entry,
//! the self-edge and each return path — so they cannot drift.

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use inkwell::types::{BasicTypeEnum, IntType};
use inkwell::values::{BasicValue, BasicValueEnum, InstructionValue, IntValue, PointerValue};

/// The scratch slot's name — distinct from every buffer the IR audits count.
const SUM_WORD_SLOT: &str = "sum_word";

/// The scratch slot's alignment: a word is at most an `i64`.
const SUM_WORD_ALIGN: u32 = 8;

impl<'ctx> CodeGen<'ctx> {
    /// Pack a blob into its word: zero the slot, store the blob, load the word.
    pub(crate) fn pack_word(
        &self,
        blob: BasicValueEnum<'ctx>,
        word_ty: IntType<'ctx>,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let slot = self.sum_word_slot(word_ty)?;
        let zero = self.builder.build_store(slot, word_ty.const_zero());
        align_store(zero)?;
        let store = self.builder.build_store(slot, blob);
        align_store(store)?;
        let word = self
            .builder
            .build_load(word_ty, slot, "sum_word.pack")
            .map_err(llvm_err)?;
        set_align(word.as_instruction_value())?;
        Ok(word.into_int_value())
    }

    /// Unpack a word into the blob the body expects: store the word, load the
    /// blob.
    pub(crate) fn unpack_word(
        &self,
        word: IntValue<'ctx>,
        blob_ty: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let slot = self.sum_word_slot(word.get_type())?;
        let store = self.builder.build_store(slot, word);
        align_store(store)?;
        let blob = self
            .builder
            .build_load(blob_ty, slot, "sum_word.unpack")
            .map_err(llvm_err)?;
        set_align(blob.as_instruction_value())?;
        Ok(blob)
    }

    /// Return a word body's result from the body end (ADR 2.10.26h D4). A
    /// block a tail self-call already terminated needs nothing; the musttail
    /// epilogue's dead-block placeholder is already a word and is returned as
    /// it is, never packed as a blob; anything else is cast to the blob and
    /// packed.
    pub(crate) fn emit_word_return(
        &mut self,
        result: BasicValueEnum<'ctx>,
        blob_ty: BasicTypeEnum<'ctx>,
        word_ty: IntType<'ctx>,
    ) -> Result<(), CodeGenError> {
        let terminated = self
            .builder
            .get_insert_block()
            .and_then(inkwell::basic_block::BasicBlock::get_terminator)
            .is_some();
        if terminated {
            return Ok(());
        }
        let word = if result.get_type() == BasicTypeEnum::from(word_ty) {
            result.into_int_value()
        } else {
            let blob = self.cast_to_type(result, blob_ty)?;
            self.pack_word(blob, word_ty)?
        };
        self.builder.build_return(Some(&word)).map_err(llvm_err)?;
        Ok(())
    }

    /// The value an early `return` hands back from `function` (ADR 2.10.26h
    /// D4): inside a word-returning `$direct_mt`, a blob is packed through the
    /// zeroing slot, because `cast_to_type` would carry undefined padding bits
    /// across; every other function keeps `cast_to_type`.
    pub(crate) fn early_return_value(
        &mut self,
        function: inkwell::values::FunctionValue<'ctx>,
        ret_ty: BasicTypeEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let is_mt = function
            .get_name()
            .to_string_lossy()
            .ends_with(super::DIRECT_MT_SUFFIX);
        if let (true, BasicTypeEnum::IntType(word_ty)) = (is_mt, ret_ty) {
            if self.small_sum_word(value.get_type()) == Some(word_ty) {
                return Ok(self.pack_word(value, word_ty)?.into());
            }
        }
        self.cast_to_type(value, ret_ty)
    }

    /// A fresh `sum_word` slot in the current function's entry block, so a pack
    /// inside a loop body or a dead arm never grows the frame per iteration.
    fn sum_word_slot(&self, word_ty: IntType<'ctx>) -> Result<PointerValue<'ctx>, CodeGenError> {
        let function = self
            .builder
            .get_insert_block()
            .and_then(inkwell::basic_block::BasicBlock::get_parent)
            .ok_or_else(|| CodeGenError::TypeError("sum_word slot outside a function".into()))?;
        let entry = function
            .get_first_basic_block()
            .ok_or_else(|| CodeGenError::TypeError("sum_word slot with no entry block".into()))?;
        let entry_builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => entry_builder.position_before(&first),
            None => entry_builder.position_at_end(entry),
        }
        let slot = entry_builder
            .build_alloca(word_ty, SUM_WORD_SLOT)
            .map_err(llvm_err)?;
        set_align(slot.as_instruction())?;
        Ok(slot)
    }
}

fn llvm_err(e: impl std::fmt::Display) -> CodeGenError {
    CodeGenError::LlvmError(e.to_string())
}

fn align_store(
    store: Result<InstructionValue<'_>, inkwell::builder::BuilderError>,
) -> Result<(), CodeGenError> {
    set_align(Some(store.map_err(llvm_err)?))
}

fn set_align(inst: Option<InstructionValue<'_>>) -> Result<(), CodeGenError> {
    if let Some(inst) = inst {
        inst.set_alignment(SUM_WORD_ALIGN).map_err(llvm_err)?;
    }
    Ok(())
}
