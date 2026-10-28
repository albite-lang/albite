//! A buffer's header fields, in both runs (ADR 3.10.26n D4, 9.10.26n D1).
//!
//! Outside the core's run a handle is the address of a separate header
//! `{ len: i64, live: i64, data: ptr }`. In the core's own run it is the
//! address of a trailer `{ len: i64, live: i64 }` inside the data's block
//! (`layout.rs`), and `data` is not stored: data starts the block, so it is
//! `trailer − round_up_8(len + 1)`, derived from the trailer's own `len`.
//! Every field access goes through [`CodeGen::bytes_header_type`], which picks
//! the run's struct, so a `FIELD_DATA` access left behind in the core's run is
//! a `build_struct_gep` error, not a load 8 bytes past a 16-byte trailer.

use super::llvm_err;
use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::AddressSpace;

/// Header field indices.
pub(super) const FIELD_LEN: u32 = 0;
pub(super) const FIELD_LIVE: u32 = 1;
pub(super) const FIELD_DATA: u32 = 2;

/// A user program's header size in bytes: three 8-byte fields.
pub(super) const HEADER_BYTES: u64 = 24;

impl<'ctx> CodeGen<'ctx> {
    /// `{ i64 len, i64 live, ptr data }`, or in the core's run the trailer
    /// `{ i64 len, i64 live }` (ADR 9.10.26n D1).
    pub(super) fn bytes_header_type(&self) -> StructType<'ctx> {
        let i64_type = self.context.i64_type();
        if self.native_core.is_core_run {
            return self
                .context
                .struct_type(&[i64_type.into(), i64_type.into()], false);
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        self.context
            .struct_type(&[i64_type.into(), i64_type.into(), ptr_type.into()], false)
    }

    /// The buffer's data pointer: the stored `data` field, or in the core's
    /// run the block's start, `trailer − header_offset(len)` (D1).
    pub(super) fn bytes_data_pointer(
        &self,
        header: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        if !self.native_core.is_core_run {
            return Ok(self
                .load_header_field_value(header, FIELD_DATA)?
                .into_pointer_value());
        }
        let len = self.load_header_field(header, FIELD_LEN)?;
        let (_, header_offset) = self.build_core_buffer_layout(len, "bytes_data")?;
        let back = self
            .builder
            .build_int_neg(header_offset, "bytes_data_back")
            .map_err(|e| llvm_err(&e))?;
        // SAFETY: an `i8` GEP back to the block's start, which the trailer
        // sits `header_offset` bytes past.
        unsafe {
            self.builder
                .build_gep(self.context.i8_type(), header, &[back], "bytes_data")
                .map_err(|e| llvm_err(&e))
        }
    }

    /// `data + offset`, unchecked — every caller has bounded `offset`.
    pub(super) fn data_slot(
        &self,
        header: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let data = self.bytes_data_pointer(header)?;
        // SAFETY: an `i8` GEP; the caller's unsigned bound keeps it in the block.
        unsafe {
            self.builder
                .build_gep(self.context.i8_type(), data, &[offset], name)
                .map_err(|e| llvm_err(&e))
        }
    }

    pub(super) fn header_field_ptr(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        self.builder
            .build_struct_gep(self.bytes_header_type(), header, field, "bytes_field")
            .map_err(|e| llvm_err(&e))
    }

    pub(super) fn load_header_field_value(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let field_type = self
            .bytes_header_type()
            .get_field_type_at_index(field)
            .ok_or_else(|| CodeGenError::LlvmError("bytes header field".into()))?;
        let slot = self.header_field_ptr(header, field)?;
        self.builder
            .build_load(field_type, slot, "bytes_load")
            .map_err(|e| llvm_err(&e))
    }

    pub(super) fn load_header_field(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<IntValue<'ctx>, CodeGenError> {
        Ok(self
            .load_header_field_value(header, field)?
            .into_int_value())
    }

    pub(super) fn store_header_field(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), CodeGenError> {
        let slot = self.header_field_ptr(header, field)?;
        self.builder
            .build_store(slot, value)
            .map_err(|e| llvm_err(&e))?;
        Ok(())
    }
}
