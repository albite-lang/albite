//! The native core's fixed C symbols (ADR 4.10.26n D2, D3).
//!
//! A `.tg` function compiles to `<name>`, `<name>$direct`, `<name>$direct_mt`
//! and lambdas, none of which a Rust or C caller can name. [`NATIVE_CORE_EXPORTS`]
//! is the compiler-side table that closes that gap: each row names a C symbol
//! and the core definition it exports, and [`CodeGen::emit_native_core_exports`]
//! defines the symbol as a shim whose body is one call to the definition's
//! `$direct` with a null environment.
//!
//! The core is compiled by its own code-generation run (the bootstrap's
//! `compile/native_core/`), which cannot see the program's names, so
//! [`CodeGen::prefix_native_core_symbols`] forces [`NATIVE_CORE_PREFIX`] onto
//! every externally linked definition the run made — `<name>`, `$direct`,
//! `$direct_mt`, lambdas and the `__wrap_` helpers alike — except the table's
//! own rows. Renaming the finished module, rather than every naming site,
//! is what makes "every" true: LLVM rewrites each reference with the name.

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::module::Linkage;
use inkwell::types::{BasicTypeEnum, FunctionType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};

mod optimize;
mod runtime;
pub use optimize::NATIVE_CORE_PIPELINE;

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;

// The table is plain data in `albite_core`, so readers with no LLVM
// (`tungsten info ffi exports`) read the same rows this module emits.
pub use albite_core::native_core::{
    is_native_core_export, native_core_symbol_name, shadowing_symbol, EnabledRows,
    NativeCoreExport, NATIVE_CORE_EXPORTS, NATIVE_CORE_PREFIX,
};

/// The Rust init the main wrapper calls (ADR 10.5.26c), and the init row's
/// shadowed export (ADR 7.10.26r D3).
pub const INIT_ARGS_SYMBOL: &str = "tg_init_args_c";

/// `ALBITE_NATIVE_CORE`, as the compile driver parsed it (4.10.26n D6,
/// 6.10.26n D5): which export rows a `++` may call, and whether the core is
/// linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeCoreSwitch {
    /// Today's IR: the Rust exports, and no core run.
    #[default]
    Off,
    /// The rows with `on_by_default`; the core is linked.
    On,
    /// Every row, so parity and measurement can run before a row is on by
    /// default; the core is linked.
    All,
}

impl NativeCoreSwitch {
    /// The rows this setting enables.
    #[must_use]
    pub fn enabled_rows(self) -> EnabledRows {
        match self {
            NativeCoreSwitch::Off => EnabledRows::None,
            NativeCoreSwitch::On => EnabledRows::Defaults,
            NativeCoreSwitch::All => EnabledRows::Every,
        }
    }

    /// Whether the core's run is compiled and linked.
    #[must_use]
    pub fn links_core(self) -> bool {
        self != NativeCoreSwitch::Off
    }
}

/// The shadowed Rust export's declared type, which the shim takes as its own.
fn shadowed_signature<'ctx>(
    codegen: &CodeGen<'ctx>,
    row: &NativeCoreExport,
) -> Result<FunctionType<'ctx>, CodeGenError> {
    codegen
        .module
        .get_function(row.shadows)
        .map(inkwell::values::FunctionValue::get_type)
        .ok_or_else(|| {
            CodeGenError::LlvmError(format!(
                "{}: the shadowed export {} is not declared",
                row.symbol, row.shadows
            ))
        })
}

impl<'ctx> CodeGen<'ctx> {
    /// Give this run the switch the compile driver read (D6).
    pub fn set_native_core_switch(&mut self, switch: NativeCoreSwitch) {
        self.native_core.switch = switch;
    }

    /// Whether any definition the program reaches opens a region, so the
    /// main wrapper selects the scoped arena mode (ADR 9.10.26o D7).
    pub fn set_regions_used(&mut self, used: bool) {
        self.regions.regions_used = used;
    }

    /// Mark this run as the native core's own (D3, D5).
    pub fn set_native_core_run(&mut self) {
        self.native_core.is_core_run = true;
    }

    /// Define each [`NATIVE_CORE_EXPORTS`] row as a shim over its
    /// definition's `$direct` (a unary definition's own entry):
    /// `call $direct(null, args…)`, then `ret` — or,
    /// for a `void` export, `ret void`, or `unreachable` when the shadowed
    /// declaration is `noreturn` (ADR 7.10.26q D3).
    ///
    /// # Errors
    /// A row whose definition has no `$direct`, or whose `$direct` does not
    /// take an environment and then the shadowed export's parameters.
    pub fn emit_native_core_exports(&mut self) -> Result<(), CodeGenError> {
        // The trap row shadows `tg_int_trap`, which only a trap site declared
        // until now: declare it here, so its shim never depends on one.
        self.rust_int_trap_function();
        // Likewise the init row's `tg_init_args_c`, which only the main
        // wrapper declares (ADR 7.10.26r D3).
        self.rust_init_args_function();
        for row in NATIVE_CORE_EXPORTS {
            self.emit_native_core_shim(row)?;
        }
        Ok(())
    }

    fn emit_native_core_shim(&mut self, row: &NativeCoreExport) -> Result<(), CodeGenError> {
        let llvm_err = |e: &dyn std::fmt::Display| CodeGenError::LlvmError(e.to_string());
        let signature = shadowed_signature(self, row)?;
        let direct_name = format!("{}$direct", row.definition);
        // A unary definition has no `$direct` (`declare_direct_entry` starts at
        // arity 2): its own entry already takes the environment and the one
        // argument, so the trap's shim calls `int_trap` itself.
        let direct = self
            .module
            .get_function(&direct_name)
            .or_else(|| {
                self.module
                    .get_function(row.definition)
                    .filter(|entry| entry.count_params() == 2)
            })
            .ok_or_else(|| {
                CodeGenError::LlvmError(format!(
                    "{}: the core defines no {direct_name}",
                    row.symbol
                ))
            })?;
        if direct.count_params() != signature.count_param_types() + 1 {
            return Err(CodeGenError::LlvmError(format!(
                "{}: {direct_name} takes {} parameter(s), not an environment and {}",
                row.symbol,
                direct.count_params(),
                signature.count_param_types()
            )));
        }
        let shim = self.shim_function(row, signature)?;
        let entry = self.context.append_basic_block(shim, "entry");
        self.builder.position_at_end(entry);
        let null_env = self
            .context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null();
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> =
            vec![BasicValueEnum::from(null_env).into()];
        for (param, wanted) in shim
            .get_param_iter()
            .zip(direct.get_type().get_param_types().into_iter().skip(1))
        {
            args.push(self.widen_shim_argument(param, wanted)?.into());
        }
        let call = self
            .builder
            .build_call(direct, &args, "native_core_result")
            .map_err(|e| llvm_err(&e))?;
        if signature.get_return_type().is_none() {
            if is_noreturn(self, row) {
                self.builder.build_unreachable().map_err(|e| llvm_err(&e))?;
            } else {
                self.builder.build_return(None).map_err(|e| llvm_err(&e))?;
            }
            return Ok(());
        }
        let result = call.try_as_basic_value().left().ok_or_else(|| {
            CodeGenError::LlvmError(format!("{}: {direct_name} returns void", row.symbol))
        })?;
        self.builder
            .build_return(Some(&result))
            .map_err(|e| llvm_err(&e))?;
        Ok(())
    }

    /// The function the shim's body goes into: the declaration a trap site
    /// already made in this module, if any — the core's own byte checks
    /// declare `tg_native_int_trap` under `on` — else a new one. Adding a
    /// second would be uniqued to `<symbol>.1`, leaving the export undefined.
    fn shim_function(
        &self,
        row: &NativeCoreExport,
        signature: FunctionType<'ctx>,
    ) -> Result<FunctionValue<'ctx>, CodeGenError> {
        match self.module.get_function(row.symbol) {
            Some(declared) if declared.count_basic_blocks() == 0 => {
                declared.set_linkage(Linkage::External);
                Ok(declared)
            }
            Some(_) => Err(CodeGenError::LlvmError(format!(
                "{}: the shim is already defined",
                row.symbol
            ))),
            None => Ok(self
                .module
                .add_function(row.symbol, signature, Some(Linkage::External))),
        }
    }

    /// `param` as `$direct` takes it: an integer narrower than the
    /// definition's (the trap's `i32` kind, a `Nat` there) is zero-extended;
    /// a pointer the definition takes as an integer (the init's `argv`, a
    /// `Nat`, ADR 7.10.26r D3) is converted with `ptrtoint`; anything else
    /// passes through unchanged.
    fn widen_shim_argument(
        &self,
        param: BasicValueEnum<'ctx>,
        wanted: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodeGenError> {
        match (param, wanted) {
            (BasicValueEnum::PointerValue(pointer), BasicTypeEnum::IntType(int)) => {
                let address = self
                    .builder
                    .build_ptr_to_int(pointer, int, "native_core_address")
                    .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
                Ok(address.into())
            }
            (BasicValueEnum::IntValue(narrow), BasicTypeEnum::IntType(wide))
                if narrow.get_type().get_bit_width() < wide.get_bit_width() =>
            {
                let widened = self
                    .builder
                    .build_int_z_extend(narrow, wide, "native_core_widened")
                    .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
                Ok(widened.into())
            }
            _ => Ok(param),
        }
    }

    /// `tg_init_args_c(i32, ptr) -> void`, the Rust init the main wrapper
    /// calls under `off`, declared once per module (ADR 10.5.26c, 7.10.26r D3).
    pub(crate) fn rust_init_args_function(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function(INIT_ARGS_SYMBOL)
            .unwrap_or_else(|| {
                let fn_type = self.context.void_type().fn_type(
                    &[
                        self.context.i32_type().into(),
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                    ],
                    false,
                );
                self.module.add_function(INIT_ARGS_SYMBOL, fn_type, None)
            })
    }

    /// The function the main wrapper's prologue calls: the switch's shadowing
    /// symbol for `tg_init_args_c`, so `off` IR is today's byte for byte and
    /// an enabled init row calls `tg_native_init_args_c` (ADR 7.10.26r D3).
    pub(crate) fn init_args_function(&self) -> FunctionValue<'ctx> {
        let rust = self.rust_init_args_function();
        let symbol = shadowing_symbol(
            NATIVE_CORE_EXPORTS,
            self.native_core.switch.enabled_rows(),
            INIT_ARGS_SYMBOL,
        );
        self.native_export_function(symbol).unwrap_or(rust)
    }

    /// Force [`NATIVE_CORE_PREFIX`] onto every externally linked definition
    /// in this module except the export rows (D3). Declarations — the runtime
    /// functions the core calls — keep their names.
    pub fn prefix_native_core_symbols(&mut self) {
        for function in self.module.get_functions() {
            let name = function.get_name().to_string_lossy().into_owned();
            let defined = function.count_basic_blocks() > 0;
            if defined && is_external(function.get_linkage()) {
                function
                    .as_global_value()
                    .set_name(&native_core_symbol_name(&name));
            }
        }
        for global in self.module.get_globals() {
            let name = global.get_name().to_string_lossy().into_owned();
            if global.get_initializer().is_some() && is_external(global.get_linkage()) {
                global.set_name(&native_core_symbol_name(&name));
            }
        }
    }

    /// The declaration of export row `symbol`'s shim, added on first use so
    /// an `off` module never mentions it (4.10.26n D6: `off` IR is today's,
    /// byte for byte). It takes the shadowed Rust export's declared type.
    /// `None` when `symbol` is no row, or its shadowed export is undeclared.
    pub(crate) fn native_export_function(&self, symbol: &str) -> Option<FunctionValue<'ctx>> {
        let row = NATIVE_CORE_EXPORTS
            .iter()
            .find(|row| row.symbol == symbol)?;
        if let Some(function) = self.module.get_function(row.symbol) {
            return Some(function);
        }
        let shadowed = self.module.get_function(row.shadows)?;
        Some(
            self.module
                .add_function(row.symbol, shadowed.get_type(), None),
        )
    }
}

/// Whether the Rust export `row` shadows is declared `noreturn`, as
/// `tg_int_trap` is: its shim must not return either.
fn is_noreturn(codegen: &CodeGen<'_>, row: &NativeCoreExport) -> bool {
    let kind = Attribute::get_named_enum_kind_id("noreturn");
    codegen
        .module
        .get_function(row.shadows)
        .is_some_and(|f| f.get_enum_attribute(AttributeLoc::Function, kind).is_some())
}

/// Whether a definition with `linkage` is visible to the linker.
fn is_external(linkage: Linkage) -> bool {
    !matches!(linkage, Linkage::Private | Linkage::Internal)
}

// Tests: runtime_tests.rs — the runtime intrinsics' lowerings and the trap shim.
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
#[cfg(test)]
mod tests;
