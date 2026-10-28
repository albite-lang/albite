//! String character operations — `char_at`, substring, and helpers.

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use albite_core::eval::IntTrapKind;
use albite_core::terms::Term;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

impl<'ctx> CodeGen<'ctx> {
    /// Compile `char_at(s, n)` — the byte at index `n`, or `0` past the end.
    pub(crate) fn compile_str_char_at(
        &mut self,
        s: &Term,
        idx: &Term,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let str_val = self.compile_term(s)?.into_struct_value();
        let idx_val = self.compile_term(idx)?.into_int_value();
        self.build_str_char_at(str_val, idx_val)
    }

    /// The lowering of `char_at` over compiled operands. Past the end is `0`,
    /// the value the evaluator and the kernel give, and nothing is read there:
    /// the load sits behind `build_byte_or_zero`'s unsigned guard (ADR
    /// 16.10.26i D1, D2).
    pub(crate) fn build_str_char_at(
        &mut self,
        str_val: StructValue<'ctx>,
        idx_val: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let (str_ptr, str_len) = self.extract_string_parts(str_val)?;
        let char_byte = self.build_byte_or_zero(str_ptr, str_len, idx_val)?;
        let char_nat = self
            .builder
            .build_int_z_extend(char_byte, self.context.i64_type(), "char_nat")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(char_nat.into())
    }

    /// Compile substring(s, start, len) - extract a substring.
    pub(crate) fn compile_str_substring(
        &mut self,
        s: &Term,
        start: &Term,
        len: &Term,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let str_val = self.compile_term(s)?.into_struct_value();
        let start_val = self.compile_term(start)?.into_int_value();
        let len_val = self.compile_term(len)?.into_int_value();
        self.build_str_substring(str_val, start_val, len_val)
    }

    /// The lowering of `substring` over compiled operands: clamp both ends,
    /// trap when either cut point is inside a character (ADR 16.10.26h D2,
    /// D4), and only then allocate and copy.
    pub(crate) fn build_str_substring(
        &mut self,
        str_val: StructValue<'ctx>,
        start_val: IntValue<'ctx>,
        len_val: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let i8_type = self.context.i8_type();

        // Extract original string pointer and length
        let (str_ptr, str_len) = self.extract_string_parts(str_val)?;

        // Clamp start to string length
        let start_clamped = self.build_min_nat(start_val, str_len, "start_clamped")?;

        // Calculate remaining length from start
        let remaining = self
            .builder
            .build_int_sub(str_len, start_clamped, "remaining")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        // Clamp requested length to remaining
        let actual_len = self.build_min_nat(len_val, remaining, "actual_len")?;
        let end = self
            .builder
            .build_int_add(start_clamped, actual_len, "cut_end")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        // A cut inside a character would make a `String` that is not UTF-8:
        // check both cut points before anything is allocated.
        let start_inside = self.build_cut_is_inside_character(str_ptr, str_len, start_clamped)?;
        let end_inside = self.build_cut_is_inside_character(str_ptr, str_len, end)?;
        let inside = self
            .builder
            .build_or(start_inside, end_inside, "cut_inside")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let on_boundary = self
            .builder
            .build_not(inside, "cut_on_boundary")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        self.emit_byte_trap_unless(on_boundary, IntTrapKind::SubstringInsideCharacter)?;

        // Allocate buffer for substring
        let buf = self.malloc_bytes(actual_len, "substr_buf")?;

        // Calculate source pointer (str_ptr + start)
        let src_ptr = unsafe {
            self.builder
                .build_gep(i8_type, str_ptr, &[start_clamped], "src_ptr")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?
        };

        // Copy the substring bytes
        self.build_memcpy(buf, src_ptr, actual_len)?;

        self.build_string_struct(buf, actual_len)
    }

    /// `i1`: the byte at `point` is a UTF-8 continuation byte (`10xxxxxx`).
    /// A point at the end reads nothing and is never inside (D2).
    fn build_cut_is_inside_character(
        &mut self,
        ptr: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        point: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let i8_type = self.context.i8_type();
        let byte = self.build_byte_or_zero(ptr, len, point)?;
        let top_bits = self
            .builder
            .build_and(byte, i8_type.const_int(0xC0, false), "cut_top_bits")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        self.builder
            .build_int_compare(
                IntPredicate::EQ,
                top_bits,
                i8_type.const_int(0x80, false),
                "cut_is_continuation",
            )
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))
    }

    /// The `i8` at `ptr[index]` when `index < len`, else `0`. The load sits
    /// only on the in-range side of an `icmp ult`, so an index at or past the
    /// end reads nothing (ADR 16.10.26h D4; 16.10.26i reuses it for `char_at`).
    pub(crate) fn build_byte_or_zero(
        &mut self,
        ptr: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        let llvm = |e: inkwell::builder::BuilderError| CodeGenError::LlvmError(e.to_string());
        let i8_type = self.context.i8_type();
        let function = self
            .compilation
            .current_fn
            .ok_or_else(|| CodeGenError::LlvmError("no current function".to_string()))?;
        let from_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| CodeGenError::LlvmError("no insert block".to_string()))?;
        let load_block = self.context.append_basic_block(function, "byte_load");
        let join_block = self.context.append_basic_block(function, "byte_join");

        let in_range = self
            .builder
            .build_int_compare(IntPredicate::ULT, index, len, "byte_in_range")
            .map_err(llvm)?;
        self.builder
            .build_conditional_branch(in_range, load_block, join_block)
            .map_err(llvm)?;

        self.builder.position_at_end(load_block);
        let byte_ptr = unsafe {
            self.builder
                .build_gep(i8_type, ptr, &[index], "byte_ptr")
                .map_err(llvm)?
        };
        let loaded = self
            .builder
            .build_load(i8_type, byte_ptr, "byte_loaded")
            .map_err(llvm)?
            .into_int_value();
        self.builder
            .build_unconditional_branch(join_block)
            .map_err(llvm)?;

        self.builder.position_at_end(join_block);
        let byte = self
            .builder
            .build_phi(i8_type, "byte_or_zero")
            .map_err(llvm)?;
        byte.add_incoming(&[(&loaded, load_block), (&i8_type.const_zero(), from_block)]);
        Ok(byte.as_basic_value().into_int_value())
    }
}

// Tests: char_ops_tests.rs — the substring cut check's and `char_at`'s IR shape
// (ADR 16.10.26h AC 4, 16.10.26i AC 3)
#[cfg(test)]
#[path = "char_ops_tests.rs"]
mod tests;
