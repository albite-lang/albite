//! Main entry-point wrapper compilation and output formatting.
//!
//! Contains `compile_main_wrapper` (emits `main` + `__albite_inner_main`)
//! and the `print_value` family of output helpers.

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use albite_core::types::Type;
use inkwell::values::{BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;

/// The runtime entry `main` hands `__albite_inner_main` to, which runs it
/// on a thread whose stack the runtime sizes (ADR 19.10.26c D2). Declared by
/// `declare_runtime_functions`.
pub(crate) const RUN_ON_STACK_SYMBOL: &str = "__albite_run_on_stack";

impl<'ctx> CodeGen<'ctx> {
    /// Compile a main function wrapper.
    ///
    /// Emits two functions:
    /// - `__albite_inner_main`: contains signal handler setup, CLI args init,
    ///   the call to `albite_main`, and result printing.
    /// - `main`: stores argc/argv in globals, then hands
    ///   `__albite_inner_main` to `__albite_run_on_stack`, so the body runs
    ///   on a stack the runtime sizes rather than the shell's (ADR 19.10.26c).
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::LlvmError`] if a runtime symbol the wrapper
    /// calls (`__albite_arena_init`, `__albite_run_on_stack`, `printf`) is
    /// not declared, or LLVM
    /// rejects an emitted instruction.
    pub fn compile_main_wrapper(
        &mut self,
        main_ty: &Type,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        let inner_main = self.emit_inner_main(main_ty, i32_type, ptr_type)?;
        self.emit_outer_main(inner_main, i32_type, ptr_type)
    }

    /// Emit `__albite_inner_main() -> i32`: signal handlers, args init,
    /// call to `albite_main`, and result printing.
    fn emit_inner_main(
        &mut self,
        main_ty: &Type,
        i32_type: inkwell::types::IntType<'ctx>,
        ptr_type: inkwell::types::PointerType<'ctx>,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        let inner_type = i32_type.fn_type(&[], false);
        let inner_main = self
            .module
            .add_function("__albite_inner_main", inner_type, None);

        let inner_entry = self.context.append_basic_block(inner_main, "entry");
        self.builder.position_at_end(inner_entry);

        self.emit_signal_handlers()?;
        self.emit_arena_init()?;
        self.emit_cli_args_init(i32_type, ptr_type)?;

        if let Some(albite_main) = self.module.get_function("albite_main") {
            let null_env = ptr_type.const_null();
            let result = self
                .builder
                .build_call(albite_main, &[null_env.into()], "main_result")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?
                .try_as_basic_value()
                .left();

            self.print_value(result, main_ty)?;
        }

        self.emit_alloc_profile_teardown()?;

        let zero = i32_type.const_int(0, false);
        self.builder
            .build_return(Some(&zero))
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        Ok(inner_main)
    }

    /// Install stack overflow signal handler (unless debug-info is active).
    fn emit_signal_handlers(&mut self) -> Result<(), CodeGenError> {
        if self.tracing.debug_info.is_none() {
            let handler_ty = self.context.void_type().fn_type(&[], false);
            let handler_fn =
                self.module
                    .add_function("__albite_install_signal_handlers", handler_ty, None);
            self.builder
                .build_call(handler_fn, &[], "")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    /// Read `ALBITE_ARENA` into the runtime's process-global mode
    /// (ADR 14.9.26b) — `__albite_arena_init()`, before the first
    /// allocation. Emitted unconditionally, on the `emit_cli_args_init`
    /// precedent, and deliberately *not* inside `emit_signal_handlers`, which
    /// is skipped under `--debug-info` and would leave a debug build silently
    /// in mode `off`. The declaration comes from `declare_runtime_functions`.
    ///
    /// A program marked as using regions calls `__albite_arena_init_regions()`
    /// instead (ADR 7.10.26s D4), declared here on first use, so a region-free
    /// program's prologue and declarations stay byte-identical.
    fn emit_arena_init(&mut self) -> Result<(), CodeGenError> {
        let init_fn = if self.regions.regions_used {
            self.arena_init_regions_function()
        } else {
            self.module
                .get_function(crate::codegen::data::ARENA_INIT_SYMBOL)
                .ok_or_else(|| {
                    CodeGenError::LlvmError("__albite_arena_init not declared".to_string())
                })?
        };
        self.builder
            .build_call(init_fn, &[], "")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(())
    }

    /// `void __albite_arena_init_regions()`, declared on first use.
    fn arena_init_regions_function(&self) -> FunctionValue<'ctx> {
        let symbol = crate::codegen::data::ARENA_INIT_REGIONS_SYMBOL;
        self.module.get_function(symbol).unwrap_or_else(|| {
            let signature = self.context.void_type().fn_type(&[], false);
            self.module.add_function(symbol, signature, None)
        })
    }

    /// Initialize CLI args from globals.
    #[allow(clippy::similar_names)] // Reason: LLVM value pairs `argc_*`/`argv_*`
    fn emit_cli_args_init(
        &mut self,
        i32_type: inkwell::types::IntType<'ctx>,
        ptr_type: inkwell::types::PointerType<'ctx>,
    ) -> Result<(), CodeGenError> {
        // Always init CLI args unconditionally. In per-module codegen,
        // tg_argc/tg_argv live in separate codegen units and are not visible
        // in the main wrapper's module, so a visibility check would skip
        // initialisation and leave the argument store empty (ADR 10.5.26c).
        // The switch picks Rust's `tg_init_args_c` or the core's
        // `tg_native_init_args_c`, which shares its type (ADR 7.10.26r D3).
        let init_args_c = self.init_args_function();

        let argc_global = self.module.add_global(i32_type, None, "__albite_argc");
        let argv_global = self.module.add_global(ptr_type, None, "__albite_argv");
        argc_global.set_initializer(&i32_type.const_int(0, false));
        argv_global.set_initializer(&ptr_type.const_null());

        let argc_val = self
            .builder
            .build_load(i32_type, argc_global.as_pointer_value(), "argc")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let argv_val = self
            .builder
            .build_load(ptr_type, argv_global.as_pointer_value(), "argv")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        self.builder
            .build_call(init_args_c, &[argc_val.into(), argv_val.into()], "")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        Ok(())
    }

    /// Emit allocation profile teardown: set filter and print report (ADR 7.5.26b).
    fn emit_alloc_profile_teardown(&mut self) -> Result<(), CodeGenError> {
        if !self.tracing.alloc_profile {
            return Ok(());
        }

        if let Some(ref filter) = self.tracing.alloc_profile_filter {
            if !filter.is_empty() {
                if let Some(set_filter_fn) = self
                    .module
                    .get_function("__albite_alloc_profile_set_filter")
                {
                    let filter_ptr = self
                        .builder
                        .build_global_string_ptr(filter, "alloc_profile_filter")
                        .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
                    self.builder
                        .build_call(set_filter_fn, &[filter_ptr.as_pointer_value().into()], "")
                        .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
                }
            }
        }

        if let Some(report_fn) = self.module.get_function("__albite_alloc_profile_report") {
            self.builder
                .build_call(report_fn, &[], "")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        }

        Ok(())
    }

    /// Emit `main(argc, argv) -> i32`: stores args in globals, then returns
    /// `__albite_run_on_stack(__albite_inner_main)`.
    #[allow(clippy::similar_names)] // Reason: LLVM value pairs `argc*`/`argv*`
    fn emit_outer_main(
        &mut self,
        inner_main: FunctionValue<'ctx>,
        i32_type: inkwell::types::IntType<'ctx>,
        ptr_type: inkwell::types::PointerType<'ctx>,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        let main_type = i32_type.fn_type(&[i32_type.into(), ptr_type.into()], false);
        let c_main = self.module.add_function("main", main_type, None);

        let entry = self.context.append_basic_block(c_main, "entry");
        self.builder.position_at_end(entry);

        let argc = c_main.get_nth_param(0).unwrap();
        let argv = c_main.get_nth_param(1).unwrap();

        // Store argc/argv into globals so __albite_inner_main can read them.
        // Always store unconditionally — in per-module codegen, tg_argc/tg_argv
        // live in separate codegen units and are not visible here (ADR 10.5.26c).
        let argc_global = self
            .module
            .get_global("__albite_argc")
            .expect("__albite_argc global should exist");
        let argv_global = self
            .module
            .get_global("__albite_argv")
            .expect("__albite_argv global should exist");

        self.builder
            .build_store(argc_global.as_pointer_value(), argc)
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        self.builder
            .build_store(argv_global.as_pointer_value(), argv)
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        // Run __albite_inner_main on a thread the runtime sizes: on Linux
        // the main thread gets the shell's RLIMIT_STACK whatever the link
        // says, so the program's depth would depend on its caller
        // (ADR 19.10.26c D1). The body installs the overflow handler itself,
        // so its alternate stack and bounds belong to the spawned thread.
        let run_on_stack = self
            .module
            .get_function(RUN_ON_STACK_SYMBOL)
            .ok_or_else(|| {
                CodeGenError::LlvmError(format!("{RUN_ON_STACK_SYMBOL} not declared"))
            })?;
        let body = inner_main.as_global_value().as_pointer_value();
        let result = self
            .builder
            .build_call(run_on_stack, &[body.into()], "exit_code")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?
            .try_as_basic_value()
            .left()
            .unwrap();

        self.builder
            .build_return(Some(&result))
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        Ok(c_main)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Output formatting helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Print a value based on its type.
    pub(super) fn print_value(
        &mut self,
        value: Option<BasicValueEnum<'ctx>>,
        ty: &Type,
    ) -> Result<(), CodeGenError> {
        let printf = self.get_printf()?;

        match ty {
            // `%lld` is already the signed conversion, so an `Int` result
            // prints signed through the same arm (ADR 14.9.26c).
            Type::Nat | Type::Int => self.print_nat(printf, value),
            Type::Bool => self.print_bool(printf, value),
            Type::String => self.print_string(printf, value),
            Type::Unit => self.print_format(printf, "()\n", "unit_fmt"),
            _ => self.print_format(printf, "<value>\n", "value_fmt"),
        }
    }

    fn get_printf(&self) -> Result<inkwell::values::FunctionValue<'ctx>, CodeGenError> {
        self.module
            .get_function("printf")
            .ok_or_else(|| CodeGenError::LlvmError("printf not declared".to_string()))
    }

    fn print_nat(
        &mut self,
        printf: inkwell::values::FunctionValue<'ctx>,
        value: Option<BasicValueEnum<'ctx>>,
    ) -> Result<(), CodeGenError> {
        let fmt = self
            .builder
            .build_global_string_ptr("%lld\n", "nat_fmt")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        if let Some(v) = value {
            self.builder
                .build_call(printf, &[fmt.as_pointer_value().into(), v.into()], "")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    fn print_bool(
        &mut self,
        printf: inkwell::values::FunctionValue<'ctx>,
        value: Option<BasicValueEnum<'ctx>>,
    ) -> Result<(), CodeGenError> {
        let true_str = self
            .builder
            .build_global_string_ptr("true\n", "true_str")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let false_str = self
            .builder
            .build_global_string_ptr("false\n", "false_str")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        if let Some(BasicValueEnum::IntValue(b)) = value {
            let str_ptr = self
                .builder
                .build_select(
                    b,
                    true_str.as_pointer_value(),
                    false_str.as_pointer_value(),
                    "bool_str",
                )
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
            self.builder
                .build_call(printf, &[str_ptr.into()], "")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    fn print_string(
        &mut self,
        printf: inkwell::values::FunctionValue<'ctx>,
        value: Option<BasicValueEnum<'ctx>>,
    ) -> Result<(), CodeGenError> {
        let fmt = self
            .builder
            .build_global_string_ptr("%s\n", "str_fmt")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        if let Some(BasicValueEnum::StructValue(s)) = value {
            let ptr = self
                .builder
                .build_extract_value(s, 0, "str_ptr")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
            self.builder
                .build_call(printf, &[fmt.as_pointer_value().into(), ptr.into()], "")
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    fn print_format(
        &mut self,
        printf: inkwell::values::FunctionValue<'ctx>,
        format: &str,
        name: &str,
    ) -> Result<(), CodeGenError> {
        let fmt = self
            .builder
            .build_global_string_ptr(format, name)
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        self.builder
            .build_call(printf, &[fmt.as_pointer_value().into()], "")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
