//! Code generation errors and LLVM backend - object file generation and IR output.

use super::CodeGen;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::OptimizationLevel;
use std::path::Path;

use std::sync::Once;

/// Code generation errors.
#[derive(Debug, Clone)]
pub enum CodeGenError {
    /// Variable not found in scope.
    UnboundVariable(String),
    /// Type error during code generation.
    TypeError(String),
    /// LLVM error.
    LlvmError(String),
    /// Unsupported feature.
    Unsupported(String),
}

impl std::fmt::Display for CodeGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeGenError::UnboundVariable(v) => write!(f, "unbound variable: {v}"),
            CodeGenError::TypeError(msg) => write!(f, "type error: {msg}"),
            CodeGenError::LlvmError(msg) => write!(f, "LLVM error: {msg}"),
            CodeGenError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
        }
    }
}

impl std::error::Error for CodeGenError {}

/// The host's `TargetMachine` at `opt`: the one source of a module's triple
/// and data layout (`CodeGen::new`), of its object file, and of the native
/// core's pass pipeline (ADR 7.10.26p D2), so a pipeline never runs against a
/// layout the module was not given.
///
/// # Errors
///
/// [`CodeGenError::LlvmError`] if the host triple has no LLVM target or a
/// target machine cannot be created.
///
/// # Panics
///
/// Panics if LLVM's native target fails to initialize (first call only).
pub(crate) fn native_target_machine(opt: OptimizationLevel) -> Result<TargetMachine, CodeGenError> {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        Target::initialize_native(&InitializationConfig::default())
            .expect("Failed to initialize native LLVM target");
    });

    let target_triple = TargetMachine::get_default_triple();
    let target =
        Target::from_triple(&target_triple).map_err(|e| CodeGenError::LlvmError(e.to_string()))?;
    target
        .create_target_machine(
            &target_triple,
            "generic",
            "",
            opt,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| CodeGenError::LlvmError("could not create target machine".to_string()))
}

impl CodeGen<'_> {
    /// Write the module to an object file with the specified optimization level.
    ///
    /// # Errors
    ///
    /// Returns [`CodeGenError::LlvmError`] if the host triple has no LLVM
    /// target, a target machine cannot be created, or the object file cannot
    /// be written to `path`.
    ///
    /// # Panics
    ///
    /// Panics if LLVM's native target fails to initialize (first call only).
    pub fn write_object_file_with_opt(
        &self,
        path: &Path,
        opt: OptimizationLevel,
    ) -> Result<(), CodeGenError> {
        native_target_machine(opt)?
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| CodeGenError::LlvmError(e.to_string()))?;

        Ok(())
    }

    /// Write the module to an object file (default optimization: O2).
    ///
    /// # Errors
    ///
    /// As [`Self::write_object_file_with_opt`].
    pub fn write_object_file(&self, path: &Path) -> Result<(), CodeGenError> {
        self.write_object_file_with_opt(path, OptimizationLevel::Default)
    }

    /// Print the LLVM IR to stderr (for debugging).
    pub fn dump_ir(&self) {
        self.module.print_to_stderr();
    }

    /// Get the LLVM IR as a string.
    pub fn get_ir_string(&self) -> String {
        self.module.print_to_string().to_string()
    }
}
