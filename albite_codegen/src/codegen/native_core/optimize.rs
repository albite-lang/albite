//! The native core's pass pipeline (ADR 7.10.26p D2).
//!
//! Program modules are written at LLVM's `None` level, and so was the core,
//! which left every hop of one join — the shim, `string_concat$direct`, each
//! `bytes_*$direct` wrapper and the `__wrap_tg_bytes_*` stub the byte
//! intrinsic lowers into — a real call that spills its arguments. The core's
//! sources are embedded and fixed, so running LLVM's `default<O2>` over its
//! module alone folds those helpers into the joins at a fixed IR, and changes
//! no program's code.
//!
//! The pipeline runs after the forced prefix and before either output form
//! (object or `.ll`), so `tungsten compile` and `tungsten1`'s `llc` link both
//! carry the optimised core.
//!
//! One hop survived it: the owned shim's call of `string_concat_owned$direct_mt`,
//! which is external and has two callers, so the inliner had no last-call
//! bonus to fold it with (ADR 13.10.26p P0.3). [`NATIVE_CORE_INLINED_INTO_SHIM`]
//! names the definitions whose `$direct_mt` is marked `alwaysinline` first.

use albite_core::native_core::native_core_symbol_name;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::passes::PassBuilderOptions;
use inkwell::OptimizationLevel;

use crate::codegen::backend::{native_target_machine, CodeGenError};
use crate::codegen::CodeGen;

/// The new-pass-manager pipeline the core's module takes.
pub const NATIVE_CORE_PIPELINE: &str = "default<O2>";

/// The core definitions whose `$direct_mt` the pipeline must inline into
/// their shim (ADR 13.10.26p D2): the owned join only.
pub const NATIVE_CORE_INLINED_INTO_SHIM: &[&str] = &["string_concat_owned"];

impl CodeGen<'_> {
    /// Run [`NATIVE_CORE_PIPELINE`] over this module, with the target machine
    /// its data layout came from.
    ///
    /// # Errors
    /// No target machine for the host, or LLVM rejects the pipeline.
    pub fn optimize_native_core(&self) -> Result<(), CodeGenError> {
        self.mark_shim_callees_always_inline();
        let machine = native_target_machine(OptimizationLevel::Default)?;
        self.module
            .run_passes(NATIVE_CORE_PIPELINE, &machine, PassBuilderOptions::create())
            .map_err(|e| {
                CodeGenError::LlvmError(format!("native core {NATIVE_CORE_PIPELINE}: {e}"))
            })
    }

    /// `alwaysinline` on each [`NATIVE_CORE_INLINED_INTO_SHIM`] definition's
    /// prefixed `$direct_mt`, where the module defines it.
    fn mark_shim_callees_always_inline(&self) {
        let kind = Attribute::get_named_enum_kind_id("alwaysinline");
        for definition in NATIVE_CORE_INLINED_INTO_SHIM {
            let name = native_core_symbol_name(&format!("{definition}$direct_mt"));
            if let Some(function) = self.module.get_function(&name) {
                function.add_attribute(
                    AttributeLoc::Function,
                    self.context.create_enum_attribute(kind, 0),
                );
            }
        }
    }
}

// Tests: optimize_tests.rs — the shim fold's `alwaysinline` mark.
#[cfg(test)]
#[path = "optimize_tests.rs"]
mod tests;
