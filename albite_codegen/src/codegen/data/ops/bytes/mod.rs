//! Inline lowering of the checked byte buffer (ADR 3.10.26n D4).
//!
//! The nine `tg_bytes_*` intrinsics are recognised in
//! `compile_extern_call_term` **before** anything is declared, so no
//! `tg_bytes_*` symbol ever reaches the linker and no Rust is called for a
//! byte. The evaluator runs the same nine from a handle table
//! (`albite_core::eval::byte_intrinsics`), which owns the symbol list, the
//! length ceiling and the UTF-8 lead table this module emits.
//!
//! ## Representation
//!
//! A handle is the address of a header `{ len: i64, live: i64, data: ptr }`.
//! `data` is its **own** allocation, so `tg_bytes_into_string` returns an
//! allocation start — the precondition `tg_string_concat_owned` places on its
//! left operand. An interior pointer into one block would break it. In the
//! native core's own run the header is instead a two-field trailer in the
//! data's block (`layout.rs`, ADR 6.10.26m D2, 9.10.26n D1), which keeps data
//! at the block's start; `header.rs` reads the fields in both runs.
//!
//! ## Checks
//!
//! Every bound is an unsigned compare (`icmp ult`/`ule`), so an index of 2⁶³
//! reads as large, never negative, and the blit bound is `at ≤ len` then
//! `len(s) ≤ len − at`, which cannot wrap. Each check branches to a block that
//! calls `tg_int_trap(code)` and is `unreachable` — the operand-free trap path
//! `Int` uses (D6). Liveness is tested first, then the index or bound, then
//! the value: the evaluator's order, so both sides print the same line. The
//! two conversions to `String` live in `convert.rs` (ADR 4.10.26n D5), the
//! adoption of a dead string in `grow.rs` (ADR 6.10.26n D2).

mod convert;
mod grow;
mod header;
mod layout;
mod utf8;

use crate::codegen::backend::CodeGenError;
use crate::codegen::{AllocClass, CodeGen};
use albite_core::eval::byte_intrinsics::{ByteIntrinsic, BYTES_MAX_LEN};
use albite_core::eval::IntTrapKind;
use header::{FIELD_DATA, FIELD_LEN, FIELD_LIVE, HEADER_BYTES};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

fn llvm_err(error: &impl ToString) -> CodeGenError {
    CodeGenError::LlvmError(error.to_string())
}

impl<'ctx> CodeGen<'ctx> {
    /// Lower `symbol` inline if it is a byte intrinsic; `None` otherwise, so
    /// the caller goes on to declare and call an ordinary extern.
    pub(crate) fn try_compile_byte_intrinsic(
        &mut self,
        symbol: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let Some(op) = ByteIntrinsic::from_symbol(symbol) else {
            return Ok(None);
        };
        self.refuse_reserved_outside_core(op)?;
        let int = |value: &BasicValueEnum<'ctx>| value.into_int_value();
        let value = match (op, args) {
            (ByteIntrinsic::New, [len]) => self.compile_bytes_new(int(len), true)?,
            (ByteIntrinsic::NewUninit, [len]) => self.compile_bytes_new(int(len), false)?,
            (ByteIntrinsic::Len, [h]) => {
                let header = self.live_bytes_header(int(h))?;
                self.load_header_field(header, FIELD_LEN)?.into()
            }
            (ByteIntrinsic::Get, [h, i]) => self.compile_bytes_get(int(h), int(i))?,
            (ByteIntrinsic::Set, [h, i, v]) => self.compile_bytes_set(int(h), int(i), int(v))?,
            (ByteIntrinsic::BlitString, [h, at, s]) => {
                self.compile_bytes_blit_string(int(h), int(at), *s)?
            }
            (ByteIntrinsic::IntoString, [h]) => self.compile_bytes_into_string(int(h), true)?,
            (ByteIntrinsic::IntoStringUnchecked, [h]) => {
                self.compile_bytes_into_string(int(h), false)?
            }
            (ByteIntrinsic::FromStringGrown, [s, extra]) => {
                self.compile_bytes_from_string_grown(*s, int(extra))?
            }
            _ => {
                return Err(CodeGenError::LlvmError(format!(
                    "{symbol}: {} argument(s) is not the intrinsic's arity",
                    args.len()
                )))
            }
        };
        Ok(Some(value))
    }

    /// Refuse above the ceiling, then allocate the header and a separate,
    /// zero-filled data block; the handle is the header's address. `filled`
    /// is false only for `tg_bytes_new_uninit`, which is reserved to the
    /// core's run (ADR 9.10.26n D2), so a user program's buffer is always
    /// filled.
    fn compile_bytes_new(
        &mut self,
        len: IntValue<'ctx>,
        filled: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let i64_type = self.context.i64_type();
        let ceiling = i64_type.const_int(BYTES_MAX_LEN, false);
        let fits = self
            .builder
            .build_int_compare(IntPredicate::ULE, len, ceiling, "bytes_len_fits")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(fits, IntTrapKind::ByteBufferTooLarge)?;
        if self.native_core.is_core_run {
            return self.compile_core_bytes_new(len, filled);
        }

        let header_size = i64_type.const_int(HEADER_BYTES, false);
        let header = self.build_parent_alloc_call(header_size, AllocClass::Str, "bytes_header")?;
        // One spare byte, so an empty buffer never asks the allocator for 0.
        let data_size = self
            .builder
            .build_int_nuw_add(len, i64_type.const_int(1, false), "bytes_data_size")
            .map_err(|e| llvm_err(&e))?;
        let data = self.build_parent_alloc_call(data_size, AllocClass::Str, "bytes_data")?;
        self.build_memset_zero(data, len)?;

        self.store_header_field(header, FIELD_LEN, len.into())?;
        self.store_header_field(header, FIELD_LIVE, i64_type.const_int(1, false).into())?;
        self.store_header_field(header, FIELD_DATA, data.into())?;
        let handle = self
            .builder
            .build_ptr_to_int(header, i64_type, "bytes_handle")
            .map_err(|e| llvm_err(&e))?;
        Ok(handle.into())
    }

    /// The byte at `index`, after the liveness and `icmp ult` checks.
    fn compile_bytes_get(
        &mut self,
        handle: IntValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let header = self.live_bytes_header(handle)?;
        let slot = self.checked_byte_slot(header, index)?;
        let byte = self
            .builder
            .build_load(self.context.i8_type(), slot, "bytes_byte")
            .map_err(|e| llvm_err(&e))?
            .into_int_value();
        let widened = self
            .builder
            .build_int_z_extend(byte, self.context.i64_type(), "bytes_byte_nat")
            .map_err(|e| llvm_err(&e))?;
        Ok(widened.into())
    }

    /// Store `value` at `index`; refuses an index out of range, then a value
    /// above 255. Returns the handle.
    fn compile_bytes_set(
        &mut self,
        handle: IntValue<'ctx>,
        index: IntValue<'ctx>,
        value: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let header = self.live_bytes_header(handle)?;
        let slot = self.checked_byte_slot(header, index)?;
        let max_byte = self.context.i64_type().const_int(255, false);
        let is_byte = self
            .builder
            .build_int_compare(IntPredicate::ULE, value, max_byte, "bytes_value_fits")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(is_byte, IntTrapKind::ByteValueOutOfRange)?;
        let byte = self
            .builder
            .build_int_truncate(value, self.context.i8_type(), "bytes_value")
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_store(slot, byte)
            .map_err(|e| llvm_err(&e))?;
        Ok(handle.into())
    }

    /// Copy all of `text` to `at`, behind the two-part bound, with
    /// `llvm.memcpy`. Returns the handle.
    fn compile_bytes_blit_string(
        &mut self,
        handle: IntValue<'ctx>,
        at: IntValue<'ctx>,
        text: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let header = self.live_bytes_header(handle)?;
        let len = self.load_header_field(header, FIELD_LEN)?;
        let at_fits = self
            .builder
            .build_int_compare(IntPredicate::ULE, at, len, "bytes_at_fits")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(at_fits, IntTrapKind::ByteIndexOutOfRange)?;
        // `at ≤ len` holds here, so this cannot wrap.
        let room = self
            .builder
            .build_int_sub(len, at, "bytes_room")
            .map_err(|e| llvm_err(&e))?;
        let text = text.into_struct_value();
        let text_ptr = self
            .builder
            .build_extract_value(text, 0, "blit_src")
            .map_err(|e| llvm_err(&e))?
            .into_pointer_value();
        let text_len = self
            .builder
            .build_extract_value(text, 1, "blit_len")
            .map_err(|e| llvm_err(&e))?
            .into_int_value();
        let text_fits = self
            .builder
            .build_int_compare(IntPredicate::ULE, text_len, room, "bytes_text_fits")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(text_fits, IntTrapKind::ByteIndexOutOfRange)?;
        let destination = self.data_slot(header, at, "blit_dst")?;
        self.build_memcpy(destination, text_ptr, text_len)?;
        Ok(handle.into())
    }

    /// The header behind `handle`, after the liveness check.
    fn live_bytes_header(
        &mut self,
        handle: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let header = self
            .builder
            .build_int_to_ptr(handle, ptr_type, "bytes_header_ptr")
            .map_err(|e| llvm_err(&e))?;
        let live = self.load_header_field(header, FIELD_LIVE)?;
        let is_live = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                live,
                self.context.i64_type().const_zero(),
                "bytes_is_live",
            )
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(is_live, IntTrapKind::ByteBufferConsumed)?;
        Ok(header)
    }

    /// `&data[index]`, after `icmp ult index, len`.
    fn checked_byte_slot(
        &mut self,
        header: PointerValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let len = self.load_header_field(header, FIELD_LEN)?;
        let in_range = self
            .builder
            .build_int_compare(IntPredicate::ULT, index, len, "bytes_index_in_range")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(in_range, IntTrapKind::ByteIndexOutOfRange)?;
        self.data_slot(header, index, "bytes_slot")
    }

    /// `llvm.memset(dest, 0, len)`.
    fn build_memset_zero(
        &self,
        dest: PointerValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), CodeGenError> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let memset = inkwell::intrinsics::Intrinsic::find("llvm.memset")
            .and_then(|intrinsic| {
                intrinsic.get_declaration(&self.module, &[ptr_type.into(), i64_type.into()])
            })
            .ok_or_else(|| CodeGenError::LlvmError("cannot declare llvm.memset".into()))?;
        let zero = self.context.i8_type().const_zero();
        let not_volatile = self.context.bool_type().const_zero();
        self.builder
            .build_call(
                memset,
                &[dest.into(), zero.into(), len.into(), not_volatile.into()],
                "",
            )
            .map_err(|e| llvm_err(&e))?;
        Ok(())
    }

    /// `br %ok, %cont, %trap`; the trap block calls `tg_int_trap(code)` and is
    /// `unreachable`. The builder is left at `cont`.
    pub(crate) fn emit_byte_trap_unless(
        &mut self,
        ok: IntValue<'ctx>,
        trap: IntTrapKind,
    ) -> Result<(), CodeGenError> {
        let function = self
            .compilation
            .current_fn
            .ok_or_else(|| CodeGenError::LlvmError("no current function".to_string()))?;
        let trap_block = self.context.append_basic_block(function, "bytes_trap");
        let cont_block = self.context.append_basic_block(function, "bytes_cont");
        self.builder
            .build_conditional_branch(ok, cont_block, trap_block)
            .map_err(|e| llvm_err(&e))?;

        self.builder.position_at_end(trap_block);
        let code = self
            .context
            .i32_type()
            .const_int(u64::from(trap.code()), false);
        self.builder
            .build_call(self.int_trap_function(), &[code.into()], "")
            .map_err(|e| llvm_err(&e))?;
        self.builder.build_unreachable().map_err(|e| llvm_err(&e))?;

        self.builder.position_at_end(cont_block);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod grow_tests;
