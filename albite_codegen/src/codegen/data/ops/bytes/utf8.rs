//! The UTF-8 check `tg_bytes_into_string` runs natively (ADR 3.10.26n D3).
//!
//! The loop is `utf8_valid_by_lead_table` in `albite_core`, emitted: per
//! scalar, the lead byte's packed entry from a 256-entry constant table, a
//! refusal for an invalid lead, a refusal when too few bytes remain, the first
//! continuation against its narrowed range and the rest against `80..=BF`.
//! That function is tested against `std::str::from_utf8`, which the evaluator
//! uses, so the two sides agree on which buffers convert.

use super::llvm_err;
use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use albite_core::eval::byte_intrinsics::{utf8_lead_entry, UTF8_LEAD_INVALID};
use albite_core::eval::IntTrapKind;
use inkwell::module::Linkage;
use inkwell::values::{GlobalValue, IntValue, PointerValue};
use inkwell::IntPredicate;

/// The table's symbol: private to each module, so it never reaches the linker.
pub(super) const UTF8_LEAD_TABLE: &str = "__albite_bytes_utf8_lead";

impl<'ctx> CodeGen<'ctx> {
    /// Walk `data[0..len]`, trapping with `ByteBufferNotUtf8` on the first
    /// invalid scalar. The builder is left after the loop.
    pub(super) fn emit_utf8_validation(
        &mut self,
        data: PointerValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), CodeGenError> {
        let function = self
            .compilation
            .current_fn
            .ok_or_else(|| CodeGenError::LlvmError("no current function".to_string()))?;
        let i64_type = self.context.i64_type();
        let preheader = self
            .builder
            .get_insert_block()
            .ok_or_else(|| CodeGenError::LlvmError("no insert block".to_string()))?;
        let head = self.context.append_basic_block(function, "utf8_head");
        let body = self.context.append_basic_block(function, "utf8_body");
        let step = self.context.append_basic_block(function, "utf8_step");
        let done = self.context.append_basic_block(function, "utf8_done");
        self.builder
            .build_unconditional_branch(head)
            .map_err(|e| llvm_err(&e))?;

        self.builder.position_at_end(head);
        let index_phi = self
            .builder
            .build_phi(i64_type, "utf8_index")
            .map_err(|e| llvm_err(&e))?;
        let index = index_phi.as_basic_value().into_int_value();
        let finished = self
            .builder
            .build_int_compare(IntPredicate::UGE, index, len, "utf8_finished")
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_conditional_branch(finished, done, body)
            .map_err(|e| llvm_err(&e))?;

        self.builder.position_at_end(body);
        let continuations = self.emit_utf8_scalar_check(data, len, index, step)?;

        self.builder.position_at_end(step);
        let width = self
            .builder
            .build_int_add(continuations, i64_type.const_int(1, false), "utf8_width")
            .map_err(|e| llvm_err(&e))?;
        let next = self
            .builder
            .build_int_add(index, width, "utf8_next")
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_unconditional_branch(head)
            .map_err(|e| llvm_err(&e))?;
        index_phi.add_incoming(&[(&i64_type.const_zero(), preheader), (&next, step)]);

        self.builder.position_at_end(done);
        Ok(())
    }

    /// Check the scalar at `index`, branching to `step` when it is valid.
    /// Returns its continuation count (an `i64`), which dominates `step`.
    fn emit_utf8_scalar_check(
        &mut self,
        data: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        index: IntValue<'ctx>,
        step: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();
        let lead = self.load_data_byte_i32(data, index, 0)?;
        let entry = self.load_utf8_lead_entry(lead)?;
        let lead_valid = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                entry,
                i32_type.const_int(u64::from(UTF8_LEAD_INVALID), false),
                "utf8_lead_valid",
            )
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(lead_valid, IntTrapKind::ByteBufferNotUtf8)?;

        let field = |shift: u64, name: &str| -> Result<IntValue<'ctx>, CodeGenError> {
            let shifted = self
                .builder
                .build_right_shift(entry, i32_type.const_int(shift, false), false, name)
                .map_err(|e| llvm_err(&e))?;
            self.builder
                .build_and(shifted, i32_type.const_int(0xFF, false), name)
                .map_err(|e| llvm_err(&e))
        };
        let continuation_count_word = field(16, "utf8_count")?;
        let first_lo = field(8, "utf8_first_lo")?;
        let first_hi = field(0, "utf8_first_hi")?;
        let continuations = self
            .builder
            .build_int_z_extend(continuation_count_word, i64_type, "utf8_continuations")
            .map_err(|e| llvm_err(&e))?;

        // `index < len` holds in the body, so `len − index ≥ 1` cannot wrap.
        let remaining = self
            .builder
            .build_int_sub(len, index, "utf8_remaining")
            .map_err(|e| llvm_err(&e))?;
        let room = self
            .builder
            .build_int_compare(IntPredicate::ULT, continuations, remaining, "utf8_room")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(room, IntTrapKind::ByteBufferNotUtf8)?;

        for offset in 1..=3_u64 {
            self.branch_to_step_when_count_is(continuations, offset - 1, step)?;
            let byte = self.load_data_byte_i32(data, index, offset)?;
            let (lo, hi) = if offset == 1 {
                (first_lo, first_hi)
            } else {
                (
                    i32_type.const_int(0x80, false),
                    i32_type.const_int(0xBF, false),
                )
            };
            let in_range = self.byte_in_range(byte, lo, hi)?;
            self.emit_byte_trap_unless(in_range, IntTrapKind::ByteBufferNotUtf8)?;
        }
        self.builder
            .build_unconditional_branch(step)
            .map_err(|e| llvm_err(&e))?;
        Ok(continuations)
    }

    /// `br (continuations == count), %step, %more`; the builder is left at `more`.
    fn branch_to_step_when_count_is(
        &self,
        continuations: IntValue<'ctx>,
        count: u64,
        step: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<(), CodeGenError> {
        let function = self
            .compilation
            .current_fn
            .ok_or_else(|| CodeGenError::LlvmError("no current function".to_string()))?;
        let more = self.context.append_basic_block(function, "utf8_more");
        let complete = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                continuations,
                self.context.i64_type().const_int(count, false),
                "utf8_complete",
            )
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_conditional_branch(complete, step, more)
            .map_err(|e| llvm_err(&e))?;
        self.builder.position_at_end(more);
        Ok(())
    }

    /// `lo ≤ byte ≤ hi`, unsigned.
    fn byte_in_range(
        &self,
        byte: IntValue<'ctx>,
        lo: IntValue<'ctx>,
        hi: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let above = self
            .builder
            .build_int_compare(IntPredicate::UGE, byte, lo, "utf8_above_lo")
            .map_err(|e| llvm_err(&e))?;
        let below = self
            .builder
            .build_int_compare(IntPredicate::ULE, byte, hi, "utf8_below_hi")
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_and(above, below, "utf8_in_range")
            .map_err(|e| llvm_err(&e))
    }

    /// `zext data[index + offset]` to `i32`.
    fn load_data_byte_i32(
        &self,
        data: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        offset: u64,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let position = self
            .builder
            .build_int_add(
                index,
                self.context.i64_type().const_int(offset, false),
                "utf8_position",
            )
            .map_err(|e| llvm_err(&e))?;
        // SAFETY: an `i8` GEP; the `utf8_room` check bounds `position < len`.
        let slot = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), data, &[position], "utf8_slot")
                .map_err(|e| llvm_err(&e))?
        };
        let byte = self
            .builder
            .build_load(self.context.i8_type(), slot, "utf8_byte")
            .map_err(|e| llvm_err(&e))?
            .into_int_value();
        self.builder
            .build_int_z_extend(byte, self.context.i32_type(), "utf8_byte32")
            .map_err(|e| llvm_err(&e))
    }

    /// `UTF8_LEAD_TABLE[lead]`.
    fn load_utf8_lead_entry(&self, lead: IntValue<'ctx>) -> Result<IntValue<'ctx>, CodeGenError> {
        let table = self.utf8_lead_table();
        let i32_type = self.context.i32_type();
        let index = self
            .builder
            .build_int_z_extend(lead, self.context.i64_type(), "utf8_lead_index")
            .map_err(|e| llvm_err(&e))?;
        // SAFETY: `lead` is a zero-extended byte, so `0..=255` of 256 entries.
        let slot = unsafe {
            self.builder
                .build_gep(
                    i32_type.array_type(256),
                    table.as_pointer_value(),
                    &[self.context.i64_type().const_zero(), index],
                    "utf8_entry_slot",
                )
                .map_err(|e| llvm_err(&e))?
        };
        Ok(self
            .builder
            .build_load(i32_type, slot, "utf8_entry")
            .map_err(|e| llvm_err(&e))?
            .into_int_value())
    }

    /// The private constant table, emitted once per module.
    fn utf8_lead_table(&self) -> GlobalValue<'ctx> {
        self.module.get_global(UTF8_LEAD_TABLE).unwrap_or_else(|| {
            let i32_type = self.context.i32_type();
            let entries: Vec<_> = (0..=u8::MAX)
                .map(|lead| i32_type.const_int(u64::from(utf8_lead_entry(lead)), false))
                .collect();
            let table = self
                .module
                .add_global(i32_type.array_type(256), None, UTF8_LEAD_TABLE);
            table.set_initializer(&i32_type.const_array(&entries));
            table.set_constant(true);
            table.set_linkage(Linkage::Private);
            table
        })
    }
}
