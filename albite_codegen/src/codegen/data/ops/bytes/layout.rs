//! The core's one-block buffer (ADR 6.10.26m D2, 9.10.26n D1).
//!
//! In the native core's own run `tg_bytes_new` makes **one** allocation:
//! the data region first, rounded up to the trailer's alignment, then the
//! trailer `{len, live}`. The handle is the trailer's address; `data` is not
//! stored but derived (`header.rs`), since data stays at offset 0, so a
//! converted string's pointer is an allocation start that
//! `tg_string_concat_owned` may `realloc`.
//!
//! The size arithmetic is `albite_core::native_core::core_buffer_layout`'s,
//! emitted over a runtime `len`; a constant `len` folds to that function's
//! answer, which the IR-shape tests pin. Outside the core's run a buffer
//! keeps its separate header, so a stale handle in a user program always
//! reaches a live header and traps (D2, "outside the core's run").
//!
//! `tg_bytes_new` zero-fills the data region; `tg_bytes_new_uninit`, reserved
//! to the core under its write-cover rule, does not (ADR 9.10.26n D2). The
//! grow intrinsic (`grow.rs`, ADR 6.10.26n D2) reuses both halves: the
//! layout, for the old and new block sizes it passes to `__albite_realloc`,
//! and the trailer write.

use super::{llvm_err, FIELD_LEN, FIELD_LIVE};
use crate::codegen::backend::CodeGenError;
use crate::codegen::{AllocClass, CodeGen};
use albite_core::native_core::{BUFFER_HEADER_ALIGN, CORE_TRAILER_BYTES};
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

impl<'ctx> CodeGen<'ctx> {
    /// One block of `core_buffer_layout(len)` bytes with the trailer at its
    /// offset; the data region is zero-filled only when `filled`. The caller
    /// has already refused a `len` above the ceiling, so `len + 8 + 16`
    /// cannot wrap.
    pub(super) fn compile_core_bytes_new(
        &mut self,
        len: IntValue<'ctx>,
        filled: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let (block_size, header_offset) = self.build_core_buffer_layout(len, "bytes")?;
        let block = self.build_malloc_call(block_size, AllocClass::Str, "bytes_block")?;
        if filled {
            // The data region only; the trailer is written by the stores below.
            self.build_memset_zero(block, header_offset)?;
        }
        self.write_core_trailer(block, header_offset, len)
    }

    /// `core_buffer_layout(len)` over a runtime `len`: `(block_bytes,
    /// header_offset)`, named `<prefix>_block_size` / `<prefix>_header_offset`.
    /// The caller has bounded `len` by the ceiling, so neither add wraps.
    pub(super) fn build_core_buffer_layout(
        &self,
        len: IntValue<'ctx>,
        prefix: &str,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), CodeGenError> {
        let i64_type = self.context.i64_type();
        let padded = self
            .builder
            .build_int_nuw_add(
                len,
                i64_type.const_int(BUFFER_HEADER_ALIGN, false),
                &format!("{prefix}_padded"),
            )
            .map_err(|e| llvm_err(&e))?;
        let header_offset = self
            .builder
            .build_and(
                padded,
                i64_type.const_int(!(BUFFER_HEADER_ALIGN - 1), false),
                &format!("{prefix}_header_offset"),
            )
            .map_err(|e| llvm_err(&e))?;
        let block_size = self
            .builder
            .build_int_nuw_add(
                header_offset,
                i64_type.const_int(CORE_TRAILER_BYTES, false),
                &format!("{prefix}_block_size"),
            )
            .map_err(|e| llvm_err(&e))?;
        Ok((block_size, header_offset))
    }

    /// Write the trailer `{len, live: 1}` at `block + header_offset` and
    /// return its address as the handle. `data` is not stored (D1).
    pub(super) fn write_core_trailer(
        &self,
        block: PointerValue<'ctx>,
        header_offset: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let i64_type = self.context.i64_type();
        // SAFETY: an `i8` GEP to `header_offset < block_size`, inside the block.
        let header = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    block,
                    &[header_offset],
                    "bytes_trailer",
                )
                .map_err(|e| llvm_err(&e))?
        };
        self.store_header_field(header, FIELD_LEN, len.into())?;
        self.store_header_field(header, FIELD_LIVE, i64_type.const_int(1, false).into())?;
        let handle = self
            .builder
            .build_ptr_to_int(header, i64_type, "bytes_handle")
            .map_err(|e| llvm_err(&e))?;
        Ok(handle.into())
    }
}
