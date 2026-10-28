//! The native core's sources and the rules about them (ADR 4.10.26n D3, D5, D6).
//!
//! The core — runtime exports written in Tungsten, `src/native_core/` — is
//! **embedded** here with `include_str!`, so it is version-locked to the
//! compiler and packaging is unchanged. The code-generation run that lowers it
//! is the binary's (`compile/native_core/`, behind the `codegen` feature); what
//! lives here is everything that run, `check`, `run` and `test` share and none
//! of which needs LLVM:
//!
//! - [`NATIVE_CORE_SOURCES`] and [`is_core_source`];
//! - [`refuse_reserved_declarations`], the project driver's refusal of the
//!   reserved byte intrinsics and the runtime intrinsics (ADR 7.10.26q D1)
//!   outside an embedded core source (D5) — here,
//!   because the evaluator cannot see which module an `ExternCall` came from;
//! - [`rules`], the pure check of the core's own rules (D3);
//! - [`switch`], `ALBITE_NATIVE_CORE`'s parsing (D6).

pub mod rules;
pub mod switch;

use std::path::{Path, PathBuf};

use albite_core::eval::byte_intrinsics::ByteIntrinsic;
use albite_core::native_core::RuntimeIntrinsic;

use super::modules::ParsedModule;
use crate::ast::Item;

/// One embedded core source: its path under `src/native_core/` and its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSource {
    /// The file name, relative to the core's root directory.
    pub file: &'static str,
    /// The file's text, as the bootstrap was built with it.
    pub text: &'static str,
}

/// The core's root, then its modules. The test suite beside them is not
/// embedded: it runs on the evaluator, never in a program.
pub const NATIVE_CORE_SOURCES: [CoreSource; 5] = [
    CoreSource {
        file: "mod.tg",
        text: include_str!("../../../../src/native_core/mod.tg"),
    },
    CoreSource {
        file: "args.tg",
        text: include_str!("../../../../src/native_core/args.tg"),
    },
    CoreSource {
        file: "bytes.tg",
        text: include_str!("../../../../src/native_core/bytes.tg"),
    },
    CoreSource {
        file: "string.tg",
        text: include_str!("../../../../src/native_core/string.tg"),
    },
    CoreSource {
        file: "trap.tg",
        text: include_str!("../../../../src/native_core/trap.tg"),
    },
];

/// The root file the core's elaboration starts from.
pub const NATIVE_CORE_ROOT: &str = "mod.tg";

/// The core's unit name: its LLVM module, and its directory in the
/// `--emit-llvm` tree. Here rather than beside the codegen run that writes it
/// because the IR audits, which skip that directory, build without the
/// `codegen` feature (ADR 28.10.26e D4).
pub const NATIVE_CORE_UNIT: &str = "__native_core";

/// Whether `text` is byte-equal to one of the embedded core sources — the
/// only modules allowed to declare a reserved intrinsic. A user who copies a
/// core file verbatim gets exactly the core's behaviour, which is harmless.
#[must_use]
pub fn is_core_source(text: &str) -> bool {
    NATIVE_CORE_SOURCES.iter().any(|source| source.text == text)
}

/// An intrinsic only the core may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedIntrinsic {
    /// A reserved byte intrinsic (4.10.26n D5, 6.10.26n D2).
    Byte(ByteIntrinsic),
    /// Every runtime intrinsic (ADR 7.10.26q D1).
    Runtime(RuntimeIntrinsic),
}

/// The reserved intrinsic `symbol` names, if it names one (accepting the
/// elaborator's `__c_` prefix).
#[must_use]
pub fn reserved_intrinsic(symbol: &str) -> Option<ReservedIntrinsic> {
    let byte = ByteIntrinsic::from_symbol(symbol)
        .filter(|op| op.is_reserved())
        .map(ReservedIntrinsic::Byte);
    byte.or_else(|| RuntimeIntrinsic::from_symbol(symbol).map(ReservedIntrinsic::Runtime))
}

/// A module that declares a reserved intrinsic without being a core source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservedDeclaration {
    /// The declaring module's file.
    pub module: PathBuf,
    /// The C symbol it declared.
    pub symbol: String,
}

impl std::fmt::Display for ReservedDeclaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let advice = match reserved_intrinsic(&self.symbol) {
            Some(ReservedIntrinsic::Byte(ByteIntrinsic::FromStringGrown)) => {
                "(ADR 6.10.26n D2); adopting a string literal would realloc read-only data"
            }
            Some(ReservedIntrinsic::Byte(ByteIntrinsic::NewUninit)) => {
                "(ADR 9.10.26n D2); an unfilled buffer is admitted only under the core's \
                 write-cover rule"
            }
            Some(ReservedIntrinsic::Runtime(_)) => {
                "(ADR 7.10.26q D1); it is the integer trap's own write and abort"
            }
            _ => "(ADR 4.10.26n D5); convert bytes with the checked `tg_bytes_into_string`",
        };
        write!(
            f,
            "{}: `{}` is reserved to the native core {advice}",
            self.module.display(),
            self.symbol
        )
    }
}

/// Every declaration of a reserved intrinsic in `tree` whose module is not a
/// core source. `text_of` reads a module's text; a module it cannot read is
/// not a core source.
pub fn reserved_declarations(
    tree: &ParsedModule,
    text_of: &dyn Fn(&Path) -> Option<String>,
) -> Vec<ReservedDeclaration> {
    let mut found = Vec::new();
    collect_reserved(tree, text_of, &mut found);
    found
}

fn collect_reserved(
    module: &ParsedModule,
    text_of: &dyn Fn(&Path) -> Option<String>,
    found: &mut Vec<ReservedDeclaration>,
) {
    let declared = module
        .source_file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::ExternFn(def) => Some(def.symbol.as_deref().unwrap_or(&def.name.name)),
            _ => None,
        });
    for symbol in declared.filter(|symbol| reserved_intrinsic(symbol).is_some()) {
        if !text_of(&module.path).is_some_and(|text| is_core_source(&text)) {
            found.push(ReservedDeclaration {
                module: module.path.clone(),
                symbol: symbol.to_string(),
            });
        }
    }
    for submodule in &module.submodules {
        collect_reserved(submodule, text_of, found);
    }
}

/// The project driver's refusal (D5): `Err` naming every offending
/// declaration, before evaluation or lowering, so `check`, `run`, `test` and
/// `compile` all refuse it.
///
/// # Errors
/// One or more modules declare a reserved intrinsic and are not core sources.
pub fn refuse_reserved_declarations(
    tree: &ParsedModule,
    text_of: &dyn Fn(&Path) -> Option<String>,
) -> Result<(), String> {
    let found = reserved_declarations(tree, text_of);
    if found.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = found.iter().map(ToString::to_string).collect();
    Err(lines.join("\n"))
}

/// Write the embedded sources into `dir` and elaborate them cold (P0.6: the
/// project driver reads its tree from disk, so the core is written out rather
/// than elaborated in memory). No build cache is read or written.
///
/// # Errors
/// A source that cannot be written, or a core that does not elaborate.
pub fn elaborate_native_core(dir: &Path) -> Result<super::ProjectOutput, super::PipelineError> {
    for source in &NATIVE_CORE_SOURCES {
        let path = dir.join(source.file);
        std::fs::write(&path, source.text).map_err(|e| {
            super::PipelineError::IoError(path.display().to_string(), e.to_string())
        })?;
    }
    super::project::elaborate_project_cold(&dir.join(NATIVE_CORE_ROOT), 1)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod trap_tests;
