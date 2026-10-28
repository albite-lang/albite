//! The core-reserved runtime intrinsics, lowered inline (ADR 7.10.26q D1).
//!
//! `CORE_RUNTIME_INTRINSICS` (`albite_core::native_core`) is the fixed list
//! the core's rules admit beside the byte intrinsics. Each trap row lowers to
//! one libc call, because no Tungsten `extern` can state libc's ABI: a
//! `String` is a `{ptr, len}` pair and a descriptor a C `int`, and this
//! lowering is what knows both.
//!
//! - `tg_core_write_stderr(s)` → `write(2, s.ptr, s.len)`; the `ssize_t` is the
//!   `Nat` result and the trap ignores it. A short or interrupted write is not
//!   retried: every trap line is far under `PIPE_BUF`, and the process aborts
//!   on the next instruction either way.
//! - `tg_core_abort()` → `abort()`, then `unreachable`. The builder is left at
//!   the end of a fresh, unreachable block, as `emit_int_trap_branch` leaves it
//!   at its continuation, so the extern stub's own `ret` lands there and not
//!   after a terminator. `abort` rather than `llvm.trap`: `SIGABRT`, exit 134,
//!   is what Rust's `tg_int_trap` raises and what the goldens see.
//! - `tg_core_publish_args(argc, argv)` → the argument store's publish
//!   protocol (ADR 7.10.26r D2), over `albite_core`'s two statics declared
//!   as external globals. `argv` arrives as a `Nat` and is only stored.
//!
//! All are refused outside the core's own run, and none runs on the
//! evaluator, which prints its trap line from `IntTrapKind` itself.

use albite_core::native_core::RuntimeIntrinsic;
use inkwell::types::{BasicTypeEnum, FunctionType};
use inkwell::values::{
    BasicValueEnum, FunctionValue, InstructionValue, IntValue, PointerValue, StructValue,
};
use inkwell::{AddressSpace, AtomicOrdering};

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;

/// The file descriptor `tg_core_write_stderr` writes to.
const STDERR_FD: u64 = 2;

/// The argument store `albite_core` defines (ADR 7.10.26r D1).
pub(crate) const ARGS_COUNT_SYMBOL: &str = "tg_core_args_count";
/// The published `argv`, null until the first publish.
pub(crate) const ARGS_VECTOR_SYMBOL: &str = "tg_core_args_vector";

fn llvm_err(error: &impl ToString) -> CodeGenError {
    CodeGenError::LlvmError(error.to_string())
}

/// Make a load or store atomic with `ordering`, at the store's `align 8`.
fn atomic_access(
    access: Option<InstructionValue<'_>>,
    ordering: AtomicOrdering,
) -> Result<(), CodeGenError> {
    let access =
        access.ok_or_else(|| CodeGenError::LlvmError("no access instruction".to_string()))?;
    access.set_alignment(8).map_err(|e| llvm_err(&e))?;
    access
        .set_atomic_ordering(ordering)
        .map_err(|e| llvm_err(&e))
}

/// The refusal a runtime intrinsic gets outside the core's run.
pub(crate) fn runtime_refusal(intrinsic: RuntimeIntrinsic) -> String {
    format!(
        "{} is reserved to the native core (ADR 7.10.26q D1): only the core's \
         own run may write to stderr, abort or publish the program's arguments",
        intrinsic.symbol()
    )
}

impl<'ctx> CodeGen<'ctx> {
    /// Lower `symbol` inline if it is a byte intrinsic or a runtime
    /// intrinsic; `None` otherwise, so the caller declares an ordinary extern.
    pub(crate) fn try_compile_inline_intrinsic(
        &mut self,
        symbol: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        if let Some(value) = self.try_compile_byte_intrinsic(symbol, args)? {
            return Ok(Some(value));
        }
        self.try_compile_runtime_intrinsic(symbol, args)
    }

    /// Lower `symbol` if it is a runtime intrinsic, refusing it outside the
    /// core's run.
    #[allow(clippy::similar_names)] // Reason: argc/argv are C's own names for the pair
    pub(crate) fn try_compile_runtime_intrinsic(
        &mut self,
        symbol: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodeGenError> {
        let Some(intrinsic) = RuntimeIntrinsic::from_symbol(symbol) else {
            return Ok(None);
        };
        if !self.native_core.is_core_run {
            return Err(CodeGenError::LlvmError(runtime_refusal(intrinsic)));
        }
        let lowered = match (intrinsic, args) {
            (RuntimeIntrinsic::WriteStderr, [BasicValueEnum::StructValue(text)]) => {
                self.compile_core_write_stderr(*text)?
            }
            (RuntimeIntrinsic::Abort, []) => self.compile_core_abort()?,
            (
                RuntimeIntrinsic::PublishArgs,
                [BasicValueEnum::IntValue(argc), BasicValueEnum::IntValue(argv)],
            ) => self.compile_core_publish_args(*argc, *argv)?,
            _ => {
                return Err(CodeGenError::LlvmError(format!(
                    "{symbol}: {} argument(s) is not the intrinsic's shape",
                    args.len()
                )))
            }
        };
        Ok(Some(lowered))
    }

    /// `write(2, ptr, len)`, its result the `Nat`.
    fn compile_core_write_stderr(
        &mut self,
        text: StructValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let (ptr, len) = self.extract_string_parts(text)?;
        let i64_type = self.context.i64_type();
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let write_type =
            i64_type.fn_type(&[i32_type.into(), ptr_type.into(), i64_type.into()], false);
        let write = self.libc_function("write", write_type);
        let fd = i32_type.const_int(STDERR_FD, false);
        self.builder
            .build_call(write, &[fd.into(), ptr.into(), len.into()], "core_written")
            .map_err(|e| llvm_err(&e))?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodeGenError::LlvmError("write returned void".to_string()))
    }

    /// `abort()`, `unreachable`, and a fresh block for whatever follows.
    fn compile_core_abort(&mut self) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let function = self
            .compilation
            .current_fn
            .ok_or_else(|| CodeGenError::LlvmError("no current function".to_string()))?;
        let abort_type = self.context.void_type().fn_type(&[], false);
        let abort = self.libc_function("abort", abort_type);
        abort.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.noreturn_attribute(),
        );
        self.builder
            .build_call(abort, &[], "")
            .map_err(|e| llvm_err(&e))?;
        self.builder.build_unreachable().map_err(|e| llvm_err(&e))?;
        let after = self.context.append_basic_block(function, "after_abort");
        self.builder.position_at_end(after);
        Ok(self.compile_unit())
    }

    /// The publish protocol over the argument store (ADR 7.10.26r D1, D2):
    /// load the vector (acquire); if it is null, store the count (release)
    /// and exchange the vector in from null (`acq_rel`). Every access is
    /// `align 8`, which LLVM requires of an atomic load or store.
    #[allow(clippy::similar_names)] // Reason: argc/argv are C's own names for the pair
    fn compile_core_publish_args(
        &mut self,
        argc: IntValue<'ctx>,
        argv: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let function = self
            .compilation
            .current_fn
            .ok_or_else(|| CodeGenError::LlvmError("no current function".to_string()))?;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let count = self.external_global(ARGS_COUNT_SYMBOL, self.context.i64_type().into());
        let vector = self.external_global(ARGS_VECTOR_SYMBOL, ptr_type.into());
        let published = self
            .builder
            .build_load(ptr_type, vector, "args_published")
            .map_err(|e| llvm_err(&e))?;
        atomic_access(
            published.into_pointer_value().as_instruction(),
            AtomicOrdering::Acquire,
        )?;
        let unpublished = self
            .builder
            .build_is_null(published.into_pointer_value(), "args_unpublished")
            .map_err(|e| llvm_err(&e))?;
        let publish = self.context.append_basic_block(function, "publish_args");
        let done = self.context.append_basic_block(function, "args_ready");
        self.builder
            .build_conditional_branch(unpublished, publish, done)
            .map_err(|e| llvm_err(&e))?;
        self.builder.position_at_end(publish);
        let store = self
            .builder
            .build_store(count, argc)
            .map_err(|e| llvm_err(&e))?;
        atomic_access(Some(store), AtomicOrdering::Release)?;
        let argv_ptr = self
            .builder
            .build_int_to_ptr(argv, ptr_type, "argv_ptr")
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_cmpxchg(
                vector,
                ptr_type.const_null(),
                argv_ptr,
                AtomicOrdering::AcquireRelease,
                AtomicOrdering::Acquire,
            )
            .map_err(|e| llvm_err(&e))?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|e| llvm_err(&e))?;
        self.builder.position_at_end(done);
        Ok(self.compile_unit())
    }

    /// The external global `name`, declared once with no initializer, so
    /// `prefix_native_core_symbols` leaves its C name alone.
    fn external_global(&self, name: &str, value_type: BasicTypeEnum<'ctx>) -> PointerValue<'ctx> {
        self.module
            .get_global(name)
            .unwrap_or_else(|| self.module.add_global(value_type, None, name))
            .as_pointer_value()
    }

    /// The libc function `name`, declared once with `signature`.
    fn libc_function(&self, name: &str, signature: FunctionType<'ctx>) -> FunctionValue<'ctx> {
        self.module
            .get_function(name)
            .unwrap_or_else(|| self.module.add_function(name, signature, None))
    }
}
