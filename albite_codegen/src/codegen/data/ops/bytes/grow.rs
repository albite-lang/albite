//! `tg_bytes_from_string_grown(s, extra)`: adopt a dead string's block as a
//! live buffer `extra` unwritten bytes longer (ADR 6.10.26n D2, 9.10.26n D2).
//!
//! Reserved to the native core, and by the core's rules (D3) called only on
//! the owned export's left operand — a `++` result from the core's own run,
//! so its block is `core_buffer_layout(len(s))`'s. That is what lets `old`
//! be the block's real size rather than `len(s)`: under
//! `ALBITE_ARENA=bump`, `grow_last` grows in place only when `ptr + old` is
//! the arena cursor, so `len(s)` would make every owned join copy.
//!
//! The lowering, in order:
//! 1. the bound, `len(s) ≤ MAX` then `extra ≤ MAX − len(s)` — neither the sum
//!    nor the difference can wrap — trapping `ByteBufferTooLarge`;
//! 2. `__albite_realloc_amortised(ptr, block(len(s)), block(len(s) +
//!    extra))` (ADR 16.10.26j D5): `__albite_realloc` under a region or the
//!    bump arena, and otherwise a `realloc(3)` that keeps room for the next
//!    grows, so the core's blocks move no more often than Rust's;
//! 3. the trailer `{len(s) + extra, live: 1}` (ADR 9.10.26n D1).
//!
//! The `extra` new bytes are left unwritten (ADR 9.10.26n D2): the old
//! trailer's stale bytes may sit among them, and the core's write-cover rule
//! proves the one caller blits over exactly that tail before converting.
//!
//! `__albite_realloc_amortised` is declared on first use, so a module that
//! never grows does not mention it.

use super::llvm_err;
use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use albite_core::eval::byte_intrinsics::BYTES_MAX_LEN;
use albite_core::eval::IntTrapKind;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue};
use inkwell::{AddressSpace, IntPredicate};

/// The runtime's owned grow: `ptr (ptr, i64 old, i64 new)`, the signature
/// of `__albite_realloc` (ADR 14.9.26b), amortised (ADR 16.10.26j D5).
pub(crate) const GROW_SYMBOL: &str = "__albite_realloc_amortised";

impl<'ctx> CodeGen<'ctx> {
    /// Lower the adoption; returns the new buffer's handle.
    pub(super) fn compile_bytes_from_string_grown(
        &mut self,
        text: BasicValueEnum<'ctx>,
        extra: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let i64_type = self.context.i64_type();
        let text = text.into_struct_value();
        let text_ptr = self
            .builder
            .build_extract_value(text, 0, "grow_src")
            .map_err(|e| llvm_err(&e))?
            .into_pointer_value();
        let text_len = self
            .builder
            .build_extract_value(text, 1, "grow_len")
            .map_err(|e| llvm_err(&e))?
            .into_int_value();

        let ceiling = i64_type.const_int(BYTES_MAX_LEN, false);
        let len_fits = self
            .builder
            .build_int_compare(IntPredicate::ULE, text_len, ceiling, "grow_len_fits")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(len_fits, IntTrapKind::ByteBufferTooLarge)?;
        // `len ≤ MAX` holds here, so this cannot wrap.
        let room = self
            .builder
            .build_int_sub(ceiling, text_len, "grow_room")
            .map_err(|e| llvm_err(&e))?;
        let extra_fits = self
            .builder
            .build_int_compare(IntPredicate::ULE, extra, room, "grow_extra_fits")
            .map_err(|e| llvm_err(&e))?;
        self.emit_byte_trap_unless(extra_fits, IntTrapKind::ByteBufferTooLarge)?;

        let new_len = self
            .builder
            .build_int_nuw_add(text_len, extra, "grow_new_len")
            .map_err(|e| llvm_err(&e))?;
        let (old_block, _) = self.build_core_buffer_layout(text_len, "grow_old")?;
        let (new_block, header_offset) = self.build_core_buffer_layout(new_len, "grow_new")?;
        let call = self
            .builder
            .build_call(
                self.grow_function(),
                &[text_ptr.into(), old_block.into(), new_block.into()],
                "grow_block",
            )
            .map_err(|e| llvm_err(&e))?;
        let block = call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodeGenError::LlvmError(format!("{GROW_SYMBOL} returns void")))?
            .into_pointer_value();

        // The tail `[len(s), new_len)` is left unwritten (ADR 9.10.26n D2):
        // the core's write-cover rule proves the owned export blits exactly
        // it before converting, and the padding past it is never read.
        self.write_core_trailer(block, header_offset, new_len)
    }

    /// `__albite_realloc_amortised`, declared on first use.
    fn grow_function(&self) -> FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function(GROW_SYMBOL) {
            return function;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let signature =
            ptr_type.fn_type(&[ptr_type.into(), i64_type.into(), i64_type.into()], false);
        self.module
            .add_function(GROW_SYMBOL, signature, Some(Linkage::External))
    }
}
