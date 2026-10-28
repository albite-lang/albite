//! Top-level definition compilation.

mod main_wrapper;

pub(crate) use main_wrapper::RUN_ON_STACK_SYMBOL;

use super::backend::CodeGenError;
use super::CodeGen;
use albite_core::terms::Term;
use albite_core::types::Type;
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    /// Declare a top-level definition (add function signature to module).
    ///
    /// # Errors
    ///
    /// None today: declaration always succeeds. The `Result` keeps it
    /// uniform with [`Self::compile_def`], its fallible partner.
    pub fn declare_def(
        &mut self,
        name: &str,
        ty: &Type,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        // Every compile entry erases dependency first (ADR 30.9.26d D6); a
        // path that bypasses it fails loudly here rather than mis-lowering.
        debug_assert!(
            !ty.contains_dependent_former(),
            "a `Pi` or `Sigma` reached codegen: {name}"
        );
        self.types.set_current_def_name(name);
        self.defs.def_types.insert(name.to_string(), ty.clone());

        let env_ptr_type = self.context.ptr_type(AddressSpace::default());

        let fn_type = if let Type::Arrow(param_ty, ret_ty) = ty {
            let param = self.types.lower_type(param_ty);
            let ret = self.types.lower_type(ret_ty);
            ret.fn_type(&[env_ptr_type.into(), param.into()], false)
        } else {
            let ret = self.types.lower_type(ty);
            ret.fn_type(&[env_ptr_type.into()], false)
        };

        let function = self.module.add_function(name, fn_type, None);

        // Also declare the direct (uncurried) entry point for multi-arg functions
        self.declare_direct_entry(name, ty);

        // Declare decomposed $direct_mt entry if struct params block musttail (ADR 18.5.26a)
        if let Some(direct_fn) = self
            .module
            .get_function(&super::exec::direct_calls::direct_name(name))
        {
            if let Some(param_map) = self.declare_decomposed_entry(name, direct_fn.get_type()) {
                self.direct_calls.set_decompose_map(name, param_map);
            }
        }

        Ok(function)
    }

    /// Compile a top-level definition.
    ///
    /// # Errors
    ///
    /// As [`Self::compile_def_with_span`].
    pub fn compile_def(
        &mut self,
        name: &str,
        term: &Term,
        ty: &Type,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        self.compile_def_with_span(name, term, ty, None)
    }

    /// Compile a top-level definition with an optional source span for debug info.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::Unsupported`] if `name` was not first declared
    /// with [`Self::declare_def`], and propagates any [`CodeGenError`] from
    /// compiling the body or from LLVM function verification.
    pub fn compile_def_with_span(
        &mut self,
        name: &str,
        term: &Term,
        ty: &Type,
        span_start: Option<u32>,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        self.types.set_current_def_name(name);
        self.defs.def_types.insert(name.to_string(), ty.clone());

        // Set binding context so nested lambdas can inherit the source name
        self.naming.current_binding_name = Some(name.to_string());

        let function = self.module.get_function(name).ok_or_else(|| {
            CodeGenError::Unsupported(format!(
                "function '{name}' not declared (call declare_def first)"
            ))
        })?;
        self.compilation.current_fn = Some(function);

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        self.compilation.env.clear();

        // Attach debug info (DISubprogram + debug location) if enabled
        if let Some(span) = span_start {
            self.attach_debug_info_to_def(name, span, function);
        }

        // Emit allocation profiling hook at function entry (ADR 7.5.26b)
        if self.tracing.alloc_profile {
            self.emit_alloc_profile_set_fn(name)?;
        }

        // Escape analysis: identify non-escaping folds for stack allocation (ADR 8.5.26d)
        let escape_result = crate::escape_analysis::analyze_escapes(term);
        self.defs.non_escaping_folds = escape_result.non_escaping_folds;

        if let Type::Arrow(param_ty, ret_ty) = ty {
            self.compile_def_lambda(function, term, param_ty, ret_ty)?;
        } else {
            let expected_ty = self.types.lower_type(ty);
            let result = self.compile_term(term)?;
            let result = self.cast_to_type(result, expected_ty)?;
            self.emit_return_if_needed(&result)?;
        }

        self.verify_after_compile(name)?;

        // Compile the direct (uncurried) entry point if this function has arity > 1.
        // If a decomposition map exists, compile the decomposed $direct_mt + shim instead.
        if let Some(param_map) = self.direct_calls.decompose_map(name).cloned() {
            self.compile_decomposed_entry(name, term, ty, span_start, &param_map)?;
        } else {
            self.compile_direct_entry(name, term, ty, span_start)?;
        }

        Ok(function)
    }

    /// Compile the body of a lambda definition (Arrow-typed).
    fn compile_def_lambda(
        &mut self,
        function: FunctionValue<'ctx>,
        term: &Term,
        param_ty: &Type,
        ret_ty: &Type,
    ) -> Result<(), CodeGenError> {
        if let Term::Lambda(x, _, body) = term {
            let param_value = function.get_nth_param(1).ok_or_else(|| {
                CodeGenError::TypeError("expected parameter for function".to_string())
            })?;
            self.compilation
                .env
                .insert(x.clone(), (param_value, param_ty.clone()));

            // For curried functions, tell inner lambdas their expected return type.
            if let Type::Arrow(_, inner_ret) = ret_ty {
                self.compilation.expected_lambda_ret_type = Some(inner_ret.as_ref().clone());
            }

            let expected_ret_ty = self.types.lower_type(ret_ty);
            let result = self.compile_term(body)?;
            self.compilation.expected_lambda_ret_type = None;

            let result = self.cast_to_type(result, expected_ret_ty)?;
            self.emit_return_if_needed(&result)?;
            Ok(())
        } else {
            Err(CodeGenError::TypeError(
                "expected lambda for function type".to_string(),
            ))
        }
    }

    /// Emit a return instruction if the current block has no terminator.
    pub(crate) fn emit_return_if_needed(
        &self,
        result: &BasicValueEnum<'ctx>,
    ) -> Result<(), CodeGenError> {
        let current_bb = self.builder.get_insert_block().unwrap();
        if current_bb.get_terminator().is_none() {
            self.builder
                .build_return(Some(result))
                .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    /// Verify the LLVM module after compiling a top-level definition.
    pub(crate) fn verify_after_compile(&self, name: &str) -> Result<(), CodeGenError> {
        if self.monomorph.in_progress.is_empty() {
            if let Err(e) = self.module.verify() {
                eprintln!(
                    "LLVM verification failed after compiling '{}': {}",
                    name,
                    e.to_string()
                );
                return Err(CodeGenError::LlvmError(format!(
                    "Module verification failed: {}",
                    e.to_string()
                )));
            }
        }
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Allocation profiling helpers (ADR 7.5.26b)
    // ─────────────────────────────────────────────────────────────────────────

    /// Emit a call to `__albite_alloc_profile_set_fn("name")` at the current
    /// insertion point. Used at function entry when `--alloc-profile` is enabled.
    fn emit_alloc_profile_set_fn(&mut self, name: &str) -> Result<(), CodeGenError> {
        let set_fn = self
            .module
            .get_function("__albite_alloc_profile_set_fn")
            .ok_or_else(|| {
                CodeGenError::LlvmError("__albite_alloc_profile_set_fn not declared".to_string())
            })?;

        let name_ptr = self
            .builder
            .build_global_string_ptr(name, &format!("alloc_profile_{name}"))
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        self.builder
            .build_call(set_fn, &[name_ptr.as_pointer_value().into()], "")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        Ok(())
    }
}
