//! A byte buffer's two conversions to `String` (ADR 3.10.26n D3, 4.10.26n D5).
//!
//! `tg_bytes_into_string` walks the bytes for UTF-8 and traps on an invalid
//! sequence. `tg_bytes_into_string_unchecked` is the same lowering minus the
//! walk: a compiled `String` may hold any bytes (`substring` copies at byte
//! offsets), and the native core's `tg_string_concat` must return them
//! unchanged, as Rust's does. It is reserved to the native core's own run, so
//! a user program cannot convert unchecked and diverge from the evaluator,
//! which always checks.

use super::{FIELD_LEN, FIELD_LIVE};
use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use albite_core::eval::byte_intrinsics::ByteIntrinsic;
use inkwell::values::{BasicValueEnum, IntValue};

/// Whether a run may lower `op`: a reserved intrinsic only in the core's run.
pub(super) fn intrinsic_allowed(op: ByteIntrinsic, is_core_run: bool) -> bool {
    !op.is_reserved() || is_core_run
}

/// The refusal a reserved intrinsic gets outside the core's run.
pub(super) fn reserved_refusal(op: ByteIntrinsic) -> String {
    let why = match op {
        ByteIntrinsic::FromStringGrown => {
            "(ADR 6.10.26n D2): adopting a string literal would realloc read-only data"
        }
        ByteIntrinsic::NewUninit => {
            "(ADR 9.10.26n D2): an unfilled buffer is admitted only under the core's \
             write-cover rule"
        }
        _ => {
            "(ADR 4.10.26n D5): a program may convert bytes only through the checked \
             tg_bytes_into_string"
        }
    };
    format!("{} is reserved to the native core {why}", op.symbol())
}

impl<'ctx> CodeGen<'ctx> {
    /// Refuse a reserved intrinsic in any run but the native core's.
    pub(super) fn refuse_reserved_outside_core(
        &self,
        op: ByteIntrinsic,
    ) -> Result<(), CodeGenError> {
        if intrinsic_allowed(op, self.native_core.is_core_run) {
            Ok(())
        } else {
            Err(CodeGenError::LlvmError(reserved_refusal(op)))
        }
    }

    /// Validate UTF-8 when `checked`, clear `live`, and return `{ data, len }`.
    /// The liveness check runs either way, so a consumed handle still traps.
    pub(super) fn compile_bytes_into_string(
        &mut self,
        handle: IntValue<'ctx>,
        checked: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let header = self.live_bytes_header(handle)?;
        let len = self.load_header_field(header, FIELD_LEN)?;
        // The stored field, or derived from the trailer in the core's run
        // (ADR 9.10.26n D1).
        let data = self.bytes_data_pointer(header)?;
        if checked {
            self.emit_utf8_validation(data, len)?;
        }
        let dead = self.context.i64_type().const_zero();
        self.store_header_field(header, FIELD_LIVE, dead.into())?;
        self.build_string_struct(data, len)
    }
}
