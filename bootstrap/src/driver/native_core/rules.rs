//! The native core's rules (ADR 4.10.26n D3), as a pure check.
//!
//! - **No `++`.** Under `ALBITE_NATIVE_CORE=on` a `++` calls
//!   `tg_native_string_concat`, which the core defines: a `++` inside it is
//!   infinite recursion.
//! - **No generic definition**, so the core's run produces no monomorphized
//!   instance and no `__mono` unit.
//! - **Every `extern` is a byte intrinsic or a core runtime intrinsic**
//!   (`CORE_RUNTIME_INTRINSICS`, ADR 7.10.26q D1), each lowered inline, so
//!   the core calls no Rust.
//! - **No import from outside `src/native_core/`.**
//! - **A buffer is used at most once** (ADR 6.10.26m D3), in [`linear`].
//! - **Only an owned export adopts a string** (ADR 6.10.26n D3), in
//!   [`adopt`]: `AdoptsOutsideOwned` and `UsedAfterAdopt`.
//! - **An unfilled buffer is written before it is used** (ADR 9.10.26n D3),
//!   in [`cover`]: `UncoveredBuffer`.
//!
//! The facts are read from the core's elaborated definitions and parsed
//! `use` declarations; [`check_core_rules`] is a function of the facts alone,
//! so every rule is asserted over injected definitions. The sources are fixed
//! at build time, so this runs in a bootstrap test and as a `debug_assert!`
//! in the core's code-generation run — never on a user build. The one
//! exception is the write-cover rule, which stands in for a zero-fill: the
//! core's run checks it in every build, release included, and fails the run
//! on a violation (`compile/native_core/mod.rs`, ADR 9.10.26n D3).

use albite_core::eval::byte_intrinsics::ByteIntrinsic;
use albite_core::native_core::RuntimeIntrinsic;
use albite_core::terms::Term;
use albite_core::types::Type;

use crate::ast::{Item, UseTree};
use crate::elaborate::CoreDef;

use super::{CoreSource, NATIVE_CORE_ROOT};

pub mod adopt;
#[cfg(test)]
mod adopt_tests;
pub mod cover;
#[cfg(test)]
mod cover_tests;
pub mod linear;
#[cfg(test)]
mod linear_tests;

use adopt::{AdoptHop, AdoptedArgument, Adoption, ADOPT_EXTERN, ADOPT_WRAPPER};
use linear::BufferSignatures;

/// What the rules need to know about one core definition.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefinitionFacts {
    /// The definition's name.
    pub name: String,
    /// Whether its body contains a `++`.
    pub concatenates: bool,
    /// Whether its type is quantified.
    pub generic: bool,
    /// Every extern symbol its body calls, `__c_` prefix stripped.
    pub extern_symbols: Vec<String>,
    /// Every buffer variable referenced more than once in its scope.
    pub reused_buffers: Vec<String>,
    /// Every call that adopts a string (6.10.26n D3).
    pub adoptions: Vec<Adoption>,
    /// Every way it breaks the write-cover rule (9.10.26n D3), one reason each.
    pub uncovered_buffers: Vec<String>,
}

/// One `use` in a core module: the module and its import path's first segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleImport {
    /// The importing file, relative to the core's root.
    pub module: String,
    /// The first segment of the imported path.
    pub root: String,
}

/// A broken rule, naming what broke it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreRuleViolation {
    /// `definition` uses `++`.
    Concatenates { definition: String },
    /// `definition` is generic.
    Generic { definition: String },
    /// `definition` calls an extern that is neither a byte intrinsic nor a
    /// core runtime intrinsic.
    ForeignExtern { definition: String, symbol: String },
    /// `module` imports from outside the core.
    ImportsOutside { module: String, root: String },
    /// `definition` references the buffer `variable` more than once.
    BufferReused {
        definition: String,
        variable: String,
    },
    /// `definition` adopts a string outside the two hops an owned export
    /// may use, or adopts something other than its first parameter.
    AdoptsOutsideOwned { definition: String },
    /// `definition` adopts an expression derived from its first parameter,
    /// or reads that parameter after adopting it.
    UsedAfterAdopt { definition: String },
    /// `definition` leaves a byte of an unfilled buffer unwritten, uses the
    /// buffer outside the tiling chain, or names the unfilled allocation
    /// where it may not (ADR 9.10.26n D3).
    UncoveredBuffer { definition: String, reason: String },
}

impl CoreRuleViolation {
    /// Whether this is the write-cover rule's violation — the one rule the
    /// core's run checks in release builds too (ADR 9.10.26n D3).
    #[must_use]
    pub fn is_uncovered_buffer(&self) -> bool {
        matches!(self, Self::UncoveredBuffer { .. })
    }
}

impl std::fmt::Display for CoreRuleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Concatenates { definition } => {
                write!(
                    f,
                    "{definition}: uses `++`, which would call the export being defined"
                )
            }
            Self::Generic { definition } => write!(f, "{definition}: is generic"),
            Self::ForeignExtern { definition, symbol } => {
                write!(
                    f,
                    "{definition}: calls `{symbol}`, which is neither a byte intrinsic \
                     nor a core runtime intrinsic"
                )
            }
            Self::ImportsOutside { module, root } => {
                write!(
                    f,
                    "{module}: imports `{root}`, from outside the native core"
                )
            }
            Self::BufferReused {
                definition,
                variable,
            } => write!(
                f,
                "{definition}: uses the buffer `{variable}` more than once"
            ),
            Self::AdoptsOutsideOwned { definition } => write!(
                f,
                "{definition}: adopts a string, which only an owned export may do, \
                 and only on its first parameter"
            ),
            Self::UsedAfterAdopt { definition } => write!(
                f,
                "{definition}: adopts its first parameter other than as itself, \
                 or reads it after adopting it"
            ),
            Self::UncoveredBuffer { definition, reason } => write!(
                f,
                "{definition}: an unfilled buffer is not provably written before use \
                 (UncoveredBuffer, ADR 9.10.26n D3): {reason}"
            ),
        }
    }
}

/// Read the rules' facts from one elaborated definition, given what the
/// sources declare about buffers.
#[must_use]
pub fn definition_facts(def: &CoreDef, signatures: &BufferSignatures) -> DefinitionFacts {
    let mut facts = DefinitionFacts {
        name: def.name.clone(),
        generic: matches!(def.ty, Type::Forall(..)),
        reused_buffers: linear::reused_buffers(&def.name, &def.term.term, signatures),
        adoptions: adopt::adoptions(&def.term.term),
        uncovered_buffers: cover::uncovered_buffers(&def.name, &def.term.term),
        ..DefinitionFacts::default()
    };
    collect_term_facts(&def.term.term, &mut facts);
    facts
}

fn collect_term_facts(term: &Term, facts: &mut DefinitionFacts) {
    match term {
        Term::StrConcat(..) => facts.concatenates = true,
        Term::ExternCall(symbol, _) => {
            let bare = symbol.strip_prefix("__c_").unwrap_or(symbol);
            facts.extern_symbols.push(bare.to_string());
        }
        _ => {}
    }
    term.for_each_subterm(|child| collect_term_facts(child, facts));
}

/// The core's module names: every embedded file but the root, by stem.
#[must_use]
pub fn core_module_names(sources: &[CoreSource]) -> Vec<&'static str> {
    sources
        .iter()
        .filter(|source| source.file != NATIVE_CORE_ROOT)
        .filter_map(|source| source.file.strip_suffix(".tg"))
        .collect()
}

/// Every `use` in `text`, by its path's first segment.
#[must_use]
pub fn source_imports(file: &str, text: &str) -> Vec<ModuleImport> {
    let (parsed, _) = crate::parser::parse(text);
    parsed
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Use(decl) => Some(use_root(&decl.tree)),
            _ => None,
        })
        .flatten()
        .map(|root| ModuleImport {
            module: file.to_string(),
            root,
        })
        .collect()
}

/// The first segment of a `use` tree's path.
fn use_root(tree: &UseTree) -> Option<String> {
    let path = match tree {
        UseTree::Path(path) | UseTree::Alias { path, .. } => path,
        UseTree::Group { prefix, .. } | UseTree::Glob { prefix, .. } => prefix,
    };
    path.segments.first().map(|segment| segment.name.clone())
}

/// Every rule `definitions` and `imports` break, given the core's own
/// module names.
#[must_use]
pub fn check_core_rules(
    definitions: &[DefinitionFacts],
    imports: &[ModuleImport],
    core_modules: &[&str],
) -> Vec<CoreRuleViolation> {
    let mut violations = Vec::new();
    let owned = adopt::owned_definitions();
    for def in definitions {
        if def.concatenates {
            violations.push(CoreRuleViolation::Concatenates {
                definition: def.name.clone(),
            });
        }
        if def.generic {
            violations.push(CoreRuleViolation::Generic {
                definition: def.name.clone(),
            });
        }
        let foreign = def
            .extern_symbols
            .iter()
            .filter(|symbol| !is_admitted_extern(symbol));
        violations.extend(foreign.map(|symbol| CoreRuleViolation::ForeignExtern {
            definition: def.name.clone(),
            symbol: symbol.clone(),
        }));
        violations.extend(def.reused_buffers.iter().map(|variable| {
            CoreRuleViolation::BufferReused {
                definition: def.name.clone(),
                variable: variable.clone(),
            }
        }));
        violations.extend(adoption_violations(def, &owned));
        violations.extend(def.uncovered_buffers.iter().map(|reason| {
            CoreRuleViolation::UncoveredBuffer {
                definition: def.name.clone(),
                reason: reason.clone(),
            }
        }));
    }
    let outside = imports
        .iter()
        .filter(|import| !core_modules.contains(&import.root.as_str()));
    violations.extend(outside.map(|import| CoreRuleViolation::ImportsOutside {
        module: import.module.clone(),
        root: import.root.clone(),
    }));
    violations
}

/// One violation per adopting call `def` may not make (6.10.26n D3): the
/// extern only in the wrapper, the wrapper only in an owned export's
/// definition, each on the first parameter itself and never read after.
fn adoption_violations<'a>(
    def: &'a DefinitionFacts,
    owned: &'a [&str],
) -> impl Iterator<Item = CoreRuleViolation> + 'a {
    let bare = def.name.rsplit("::").next().unwrap_or(&def.name);
    def.adoptions.iter().filter_map(move |adoption| {
        let hop_allowed = match adoption.hop {
            AdoptHop::Extern => bare == ADOPT_WRAPPER || bare == ADOPT_EXTERN,
            AdoptHop::Wrapper => owned.contains(&bare),
        };
        let definition = def.name.clone();
        if !hop_allowed || adoption.argument == AdoptedArgument::Other {
            Some(CoreRuleViolation::AdoptsOutsideOwned { definition })
        } else if adoption.argument == AdoptedArgument::DerivedFromFirst || adoption.used_after {
            Some(CoreRuleViolation::UsedAfterAdopt { definition })
        } else {
            None
        }
    })
}

/// Whether the core may call `symbol`: a byte intrinsic, or one of the fixed
/// runtime intrinsics (ADR 7.10.26q D1). Nothing else, so the list is the rule.
#[must_use]
pub fn is_admitted_extern(symbol: &str) -> bool {
    ByteIntrinsic::from_symbol(symbol).is_some() || RuntimeIntrinsic::from_symbol(symbol).is_some()
}

/// The core run's release-build refusal (ADR 9.10.26n D3): every
/// `UncoveredBuffer` in `violations`, one per line, or `None` when there is
/// none. The other rules stay debug-only, so they are not named here.
#[must_use]
pub fn write_cover_refusal(violations: &[CoreRuleViolation]) -> Option<String> {
    let lines: Vec<String> = violations
        .iter()
        .filter(|violation| violation.is_uncovered_buffer())
        .map(ToString::to_string)
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// [`check_core_rules`] over elaborated definitions and the embedded sources'
/// imports — what the bootstrap test and the core's run both call.
#[must_use]
pub fn check_core_definitions(defs: &[CoreDef], sources: &[CoreSource]) -> Vec<CoreRuleViolation> {
    let texts: Vec<&str> = sources.iter().map(|source| source.text).collect();
    let signatures = linear::buffer_signatures(&texts);
    let facts: Vec<DefinitionFacts> = defs
        .iter()
        .map(|def| definition_facts(def, &signatures))
        .collect();
    let imports: Vec<ModuleImport> = sources
        .iter()
        .flat_map(|source| source_imports(source.file, source.text))
        .collect();
    check_core_rules(&facts, &imports, &core_module_names(sources))
}
