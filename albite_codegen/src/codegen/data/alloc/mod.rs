//! The shared heap-allocation helpers (`build_malloc_call`,
//! `build_parent_alloc_call`) and the `AllocClass` tags they feed to the
//! runtime's allocation symbols: `__albite_alloc(size, class)` (ADR
//! 14.9.26b), where the class reaches the allocation profiler (ADR 2.7.26a)
//! when one is active, and the region-bypassing `__albite_alloc_parent`
//! (ADR 7.10.26s D3). Split from `data/mod.rs` unchanged (7.10.26s §2.2).

use crate::codegen::{CodeGen, CodeGenError};

/// Allocation class tags every generated allocation carries (ADR 2.7.26a).
///
/// Discriminants MUST match the `CLASS_*` constants in
/// `albite_runtime/src/alloc_profile/mod.rs` — they cross the FFI
/// boundary as the second argument of `__albite_alloc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AllocClass {
    /// Recursive-ADT node (`fold_to_heap` / `mu_alloc`).
    Mu = 0,
    /// Closure environment (`env_alloc`).
    Env = 1,
    /// Mutable reference cell (`ref.new`).
    Ref = 2,
    /// String buffer (`malloc_bytes` in string ops).
    Str = 3,
    /// Recursive-ADT node whose value is an empty constructor — stamped by
    /// profiled builds only (ADR 29.9.26h), via `mu_types::fold_class`.
    MuNullary = 4,
}

/// The runtime's allocation symbol: `ptr __albite_alloc(i64 size, i32 class)`
/// (ADR 14.9.26b). Codegen never chooses the arena mode — the prologue's
/// [`ARENA_INIT_SYMBOL`] reads `ALBITE_ARENA` at start-up.
pub(crate) const ALLOC_SYMBOL: &str = "__albite_alloc";

/// The region-bypassing allocation symbol (ADR 7.10.26s D3):
/// `ptr __albite_alloc_parent(i64 size, i32 class)`. It ignores the
/// runtime's region stack, so a handle's memory never lives in a region (G4).
pub(crate) const ALLOC_PARENT_SYMBOL: &str = "__albite_alloc_parent";

/// The prologue call that reads the mode: `void __albite_arena_init()`.
pub(crate) const ARENA_INIT_SYMBOL: &str = "__albite_arena_init";

/// The prologue call of a program that uses regions (ADR 7.10.26s D4):
/// `void __albite_arena_init_regions()`, which selects the scoped mode where
/// `__albite_arena_init` would select `off`. Declared on first use, so a
/// region-free program's IR never names it.
pub(crate) const ARENA_INIT_REGIONS_SYMBOL: &str = "__albite_arena_init_regions";

/// The runtime-exported mode flag generated code branches on (ADR 18.9.26c):
/// `external hidden global i32`, `0` = off, written once by the prologue.
pub(crate) const ARENA_MODE_GLOBAL: &str = "__albite_arena_mode";

/// The platform allocator the off arm calls directly (ADR 18.9.26c).
pub(crate) const MALLOC_SYMBOL: &str = "malloc";

/// Block-name stem of the off arm. `check-arena-mode` accepts a `@malloc`
/// call only inside a block whose label starts with it (LLVM appends a
/// counter to repeated names), so the two must change together.
pub(crate) const MALLOC_BLOCK: &str = "alloc.malloc";

/// Relative weight of the off arm in the mode branch's `branch_weights`
/// (against `1` for the arena arm): `off` is the default mode.
const OFF_ARM_WEIGHT: u64 = 2000;

fn llvm_err(builder_error: &impl ToString) -> CodeGenError {
    CodeGenError::LlvmError(builder_error.to_string())
}

fn into_pointer(
    call: inkwell::values::CallSiteValue<'_>,
) -> Result<inkwell::values::PointerValue<'_>, CodeGenError> {
    call.try_as_basic_value()
        .left()
        .ok_or_else(|| CodeGenError::LlvmError("malloc returned void".to_string()))
        .map(inkwell::values::BasicValueEnum::into_pointer_value)
}

impl<'ctx> CodeGen<'ctx> {
    /// Build a heap allocation, tagged with its allocation class.
    ///
    /// Non-profiled (ADR 18.9.26c): branch on `@__albite_arena_mode` —
    /// mode off calls `@malloc` directly, anything else calls
    /// `@__albite_alloc(size, class)` — so the default path pays one
    /// predicted branch instead of a call level. Leaves the builder at the end
    /// of the `alloc.join` block, which every caller continues in.
    ///
    /// Profiled (`--alloc-profile`): always calls the symbol, because the
    /// profiler records inside it and a direct `@malloc` would bypass it.
    pub(crate) fn build_malloc_call(
        &self,
        size: inkwell::values::IntValue<'ctx>,
        class: AllocClass,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodeGenError> {
        self.build_alloc_through(ALLOC_SYMBOL, size, class, name)
    }

    /// Build a heap allocation that never lands in a region (ADR 7.10.26s D3):
    /// the same mode branch as [`Self::build_malloc_call`], whose off arm keeps
    /// its direct `@malloc` in an `alloc.malloc` block, but whose arena arm
    /// calls `@__albite_alloc_parent`. Used for the user-run `bytes_new`
    /// header and data — a handle's memory (G4).
    pub(crate) fn build_parent_alloc_call(
        &self,
        size: inkwell::values::IntValue<'ctx>,
        class: AllocClass,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodeGenError> {
        self.build_alloc_through(ALLOC_PARENT_SYMBOL, size, class, name)
    }

    /// The shape both entries share: profiled calls `symbol` unconditionally,
    /// otherwise the mode branch with `symbol` on the arena arm.
    fn build_alloc_through(
        &self,
        symbol: &str,
        size: inkwell::values::IntValue<'ctx>,
        class: AllocClass,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodeGenError> {
        if self.tracing.alloc_profile {
            return self.build_symbol_alloc(symbol, size, class, name);
        }
        self.build_mode_branched_alloc(symbol, size, class, name)
    }

    /// `call ptr @<symbol>(i64 size, i32 class)` at the builder.
    fn build_symbol_alloc(
        &self,
        symbol: &str,
        size: inkwell::values::IntValue<'ctx>,
        class: AllocClass,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodeGenError> {
        let alloc_fn = self.allocation_function(symbol);
        let class_val = self.context.i32_type().const_int(class as u64, false);
        let call = self
            .builder
            .build_call(alloc_fn, &[size.into(), class_val.into()], name)
            .map_err(|e| llvm_err(&e))?;
        into_pointer(call)
    }

    /// `ptr <symbol>(i64, i32)`. `__albite_alloc` comes from
    /// `declare_runtime_functions`; the parent symbol is declared on first use,
    /// because every `.ll` prints its declarations and a region-free module's
    /// IR must not change (ADR 7.10.26s D6 a).
    fn allocation_function(&self, symbol: &str) -> inkwell::values::FunctionValue<'ctx> {
        if let Some(function) = self.module.get_function(symbol) {
            return function;
        }
        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let signature = ptr_type.fn_type(
            &[
                self.context.i64_type().into(),
                self.context.i32_type().into(),
            ],
            false,
        );
        self.module.add_function(symbol, signature, None)
    }

    /// The §2.2 shape: load the mode, branch, call either allocator, `phi`.
    fn build_mode_branched_alloc(
        &self,
        symbol: &str,
        size: inkwell::values::IntValue<'ctx>,
        class: AllocClass,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodeGenError> {
        let function = self
            .builder
            .get_insert_block()
            .and_then(inkwell::basic_block::BasicBlock::get_parent)
            .ok_or_else(|| CodeGenError::LlvmError("allocation outside a function".into()))?;
        let mode_global = self
            .module
            .get_global(ARENA_MODE_GLOBAL)
            .expect("__albite_arena_mode not declared (declare_runtime_functions)");
        let malloc_fn = self
            .module
            .get_function(MALLOC_SYMBOL)
            .expect("malloc not declared (declare_runtime_functions)");
        let i32_type = self.context.i32_type();

        let mode = self
            .builder
            .build_load(i32_type, mode_global.as_pointer_value(), "arena.mode")
            .map_err(|e| llvm_err(&e))?
            .into_int_value();
        let is_off = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                mode,
                i32_type.const_zero(),
                "arena.off",
            )
            .map_err(|e| llvm_err(&e))?;
        let malloc_block = self.context.append_basic_block(function, MALLOC_BLOCK);
        let arena_block = self.context.append_basic_block(function, "alloc.arena");
        let join_block = self.context.append_basic_block(function, "alloc.join");
        let branch = self
            .builder
            .build_conditional_branch(is_off, malloc_block, arena_block)
            .map_err(|e| llvm_err(&e))?;
        self.mark_off_arm_likely(branch)?;

        self.builder.position_at_end(malloc_block);
        let call = self
            .builder
            .build_call(malloc_fn, &[size.into()], &format!("{name}.malloc"))
            .map_err(|e| llvm_err(&e))?;
        let from_malloc = into_pointer(call)?;
        self.builder
            .build_unconditional_branch(join_block)
            .map_err(|e| llvm_err(&e))?;

        self.builder.position_at_end(arena_block);
        let from_arena = self.build_symbol_alloc(symbol, size, class, &format!("{name}.arena"))?;
        self.builder
            .build_unconditional_branch(join_block)
            .map_err(|e| llvm_err(&e))?;

        self.builder.position_at_end(join_block);
        let phi = self
            .builder
            .build_phi(
                self.context.ptr_type(inkwell::AddressSpace::default()),
                name,
            )
            .map_err(|e| llvm_err(&e))?;
        phi.add_incoming(&[(&from_malloc, malloc_block), (&from_arena, arena_block)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    /// `!prof !{!"branch_weights", i32 OFF_ARM_WEIGHT, i32 1}` on the mode
    /// branch, so the default mode is the fall-through (ADR 18.9.26c §2.2,
    /// the first escalation AC 1 names).
    fn mark_off_arm_likely(
        &self,
        branch: inkwell::values::InstructionValue<'ctx>,
    ) -> Result<(), CodeGenError> {
        let i32_type = self.context.i32_type();
        let weights = self.context.metadata_node(&[
            self.context.metadata_string("branch_weights").into(),
            i32_type.const_int(OFF_ARM_WEIGHT, false).into(),
            i32_type.const_int(1, false).into(),
        ]);
        branch
            .set_metadata(weights, self.context.get_kind_id("prof"))
            .map_err(|e| llvm_err(&e))
    }
}

#[cfg(test)]
mod tests;
