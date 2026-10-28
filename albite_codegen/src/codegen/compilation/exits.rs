//! Exits: early `return` (ADR 13.5.26d) and `region { e }` (ADR 9.10.26o D5,
//! 13.10.26q D5).
//!
//! A region calls `__albite_region_enter` on entry and, at every exit, in
//! G7's order, `__albite_region_detach`, the copy, then
//! `__albite_region_release`. Tungsten has no loops and no `break`, so the
//! exits are fall-through and `return` — a `?` lowers to a match and a
//! `Term::Return`, so it reaches `compile_return` too. A trap aborts and
//! releases nothing (26a §9).
//!
//! The copy (ADR 13.10.26q): a value that holds a pointer is stored to an
//! entry-block slot and handed to `__albite_region_copy_out` with the static
//! layout of its type (`data::region_copy`), which moves everything it reaches
//! inside the detached region into the parent. A value that holds no pointer
//! has no layout and no copy, so a scalar region's IR is unchanged.
//!
//! The open regions are a stack of the LLVM functions each was entered in,
//! with the type a crossing `return` carries. A `return` releases only the
//! ones its own function opened, innermost first: a lambda or a `try` (an
//! immediately invoked lambda) inside a region is a different function and
//! releases nothing, so no save/reset/restore is owed at the sites that switch
//! `current_fn`. Both elaborators build a `try` as `(λ__try_unit:Unit.
//! Ok(body)) ()`, so a `?` inside one returns from that lambda and does not
//! cross the region (ADR 13.10.26o). A value copied out of an inner region
//! lands in the outer one, which copies it again at its own exit: correct,
//! and recorded rather than optimised.

use crate::codegen::backend::CodeGenError;
use crate::codegen::musttail_report::{Decision, MusttailDecision, ReasonCode};
use crate::codegen::state::OpenRegion;
use crate::codegen::CodeGen;
use albite_core::terms::{RegionTypes, Term};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;

const REGION_ENTER: &str = "__albite_region_enter";
const REGION_DETACH: &str = "__albite_region_detach";
const REGION_COPY_OUT: &str = "__albite_region_copy_out";
const REGION_RELEASE: &str = "__albite_region_release";

/// A crossing `return` whose region recorded no exit type: elaboration
/// always records one, so this names an internal fault, not a program's.
pub const MISSING_EXIT_TYPE: &str =
    "internal: a `return` crosses a region that recorded no exit type (ADR 13.10.26q D5)";

impl<'ctx> CodeGen<'ctx> {
    /// Compile `region { body }`: enter, the body (never a tail call, since
    /// the exit follows it), then detach, copy out and release.
    pub(super) fn compile_region(
        &mut self,
        types: &RegionTypes,
        body: &Term,
        is_tail: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let function = self.compilation.current_fn.ok_or_else(|| {
            CodeGenError::Unsupported("a region outside any function".to_string())
        })?;
        if is_tail {
            self.record_tail_inside_region(function);
        }
        self.call_region_runtime(REGION_ENTER, &[])?;
        self.compilation.open_regions.push(OpenRegion {
            function,
            exit: types.exit.clone(),
        });
        // `compile_term` resets the tail flag, so nothing in the body is
        // lowered as a tail call.
        let value = self.compile_term(body);
        self.compilation.open_regions.pop();
        let value = value?;
        let size = self.types.type_size(value.get_type());
        let layout = self.region_copy_layout(&types.value, size)?;
        self.emit_region_exits(function, value, layout, 1)
    }

    /// How many regions the current function has open.
    pub(crate) fn open_region_count(&self) -> usize {
        match self.compilation.current_fn {
            Some(function) => self
                .compilation
                .open_regions
                .iter()
                .filter(|opened| opened.function == function)
                .count(),
            None => 0,
        }
    }

    /// The exit type a `return` from the current function carries across its
    /// open regions: the innermost one's.
    fn crossing_exit_type(&self) -> Option<albite_core::types::Type> {
        let function = self.compilation.current_fn?;
        self.compilation
            .open_regions
            .iter()
            .rev()
            .find(|opened| opened.function == function)
            .and_then(|opened| opened.exit.clone())
    }

    /// G7's order, `count` times, innermost first: detach, copy (only with a
    /// layout), release. The value is the slot's contents after the last
    /// copy, loaded before the last release, which the caller uses in place
    /// of `value`.
    fn emit_region_exits(
        &mut self,
        function: FunctionValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        layout: Option<PointerValue<'ctx>>,
        count: usize,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let Some(layout) = layout else {
            for _ in 0..count {
                self.call_region_runtime(REGION_DETACH, &[])?;
                self.call_region_runtime(REGION_RELEASE, &[])?;
            }
            return Ok(value);
        };
        let slot = self.entry_block_slot(function, value.get_type())?;
        self.builder
            .build_store(slot, value)
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        let mut copied = value;
        for remaining in (0..count).rev() {
            self.call_region_runtime(REGION_DETACH, &[])?;
            self.call_region_runtime(REGION_COPY_OUT, &[slot, layout])?;
            if remaining == 0 {
                copied = self
                    .builder
                    .build_load(value.get_type(), slot, "region_value")
                    .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
            }
            self.call_region_runtime(REGION_RELEASE, &[])?;
        }
        Ok(copied)
    }

    /// An `alloca` at the top of `function`'s entry block, so a region inside
    /// a recursive body does not grow the stack per call.
    fn entry_block_slot(
        &mut self,
        function: FunctionValue<'ctx>,
        ty: BasicTypeEnum<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodeGenError> {
        let entry = function.get_first_basic_block().ok_or_else(|| {
            CodeGenError::Unsupported("a region in a function with no entry block".to_string())
        })?;
        let builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        builder
            .build_alloca(ty, "region_slot")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))
    }

    /// `void <symbol>(ptr…)`, declared on first use so a region-free module's
    /// declarations are unchanged.
    fn call_region_runtime(
        &mut self,
        symbol: &str,
        args: &[PointerValue<'ctx>],
    ) -> Result<(), CodeGenError> {
        let callee = self.module.get_function(symbol).unwrap_or_else(|| {
            let ptr = self.context.ptr_type(AddressSpace::default()).into();
            let params = vec![ptr; args.len()];
            let signature = self.context.void_type().fn_type(&params, false);
            self.module.add_function(symbol, signature, None)
        });
        let args: Vec<_> = args.iter().map(|arg| (*arg).into()).collect();
        self.builder
            .build_call(callee, &args, "")
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
        Ok(())
    }

    /// A tail position inside a region is not a tail call (D5): say so in the
    /// musttail report, so `--trace-musttail` names why.
    fn record_tail_inside_region(&mut self, function: FunctionValue<'ctx>) {
        self.musttail_report.push(MusttailDecision {
            function: function.get_name().to_string_lossy().into_owned(),
            decision: Decision::Skip,
            reasons: vec![ReasonCode::InsideRegion],
            blockers: Vec::new(),
            lowered_sig: String::new(),
            param_abi: Vec::new(),
            sret: false,
            word_ret: None,
            slot_attrs: Vec::new(),
        });
    }

    pub(super) fn compile_return(
        &mut self,
        inner: &Term,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        let mut result = self.compile_term(inner)?;

        // A `return` crossing this function's open regions copies out of and
        // releases each, innermost first, before the `ret` (D5).
        let crossing = self.open_region_count();
        if let (true, Some(function)) = (crossing > 0, self.compilation.current_fn) {
            let exit = self
                .crossing_exit_type()
                .ok_or_else(|| CodeGenError::Unsupported(MISSING_EXIT_TYPE.to_string()))?;
            let size = self.types.type_size(result.get_type());
            let layout = self.region_copy_layout(&exit, size)?;
            result = self.emit_region_exits(function, result, layout, crossing)?;
        }

        // Cast to the function's expected return type if needed
        if let Some(function) = self.compilation.current_fn {
            if let Some(ret_ty) = function.get_type().get_return_type() {
                // A word-returning `$direct_mt` packs instead (ADR 2.10.26h D4).
                let result = self.early_return_value(function, ret_ty, result)?;
                self.builder
                    .build_return(Some(&result))
                    .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
            } else if let Some(sret_ty) = self.compilation.current_sret_type {
                let result = self.cast_to_type(result, sret_ty)?;
                let fn_name = function.get_name().to_string_lossy().into_owned();
                self.emit_sret_return(function, &fn_name, &result)?;
            } else {
                self.builder
                    .build_return(None)
                    .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
            }

            // Create dead block for any subsequent code
            let dead_bb = self.context.append_basic_block(function, "after_return");
            self.builder.position_at_end(dead_bb);
        }

        // Return unit as the "value" of the return expression (type is ⊥, never used)
        Ok(self.compile_unit())
    }
}
