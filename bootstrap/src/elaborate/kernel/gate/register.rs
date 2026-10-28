//! The kernel exemption register (ADR 2.10.26d D4): the theorems the kernel
//! may refuse without failing the build, each named by its file, its class
//! and the ADR that will remove it.
//!
//! A data file beside the crate's `Cargo.toml`, embedded with `include_str!`,
//! so a built binary carries the register it was built with. Shrink-only by a
//! test (`KERNEL_EXEMPTION_CEILING`) plus review. A row is keyed by the
//! theorem's **defining** file relative to this repository's root, so a file
//! outside the repository matches no row and no user project can be exempted
//! by this register.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// The register as checked in.
const REGISTER_TOML: &str = include_str!("../../../../kernel_exemptions.toml");

/// The only schema the loader reads.
const SCHEMA: u32 = 1;

#[cfg(test)]
thread_local! {
    /// A drill's register, for its own thread only ([`Register::inject`]).
    static INJECTED: std::cell::Cell<Option<&'static Register>> =
        const { std::cell::Cell::new(None) };
}

/// A register class (D5). Two are admitted to the loader today; the other two
/// are named so a refusal can say what it is, and are admitted by the ADR that
/// brings each one's first row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegisterClass {
    /// The pair converts only by unfolding a global (K-δ).
    NeedsDelta,
    /// An admitted theorem that cites exempt ones (D6).
    CitesExempt,
    /// A bare type variable naming a concrete type.
    NamedType,
    /// The theorem's own name, unbound in its context.
    SelfReference,
}

impl RegisterClass {
    /// Every class, in D5's table order.
    pub const ALL: [RegisterClass; 4] = [
        RegisterClass::NeedsDelta,
        RegisterClass::CitesExempt,
        RegisterClass::NamedType,
        RegisterClass::SelfReference,
    ];

    /// The spelling the register and the diagnostics use.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RegisterClass::NeedsDelta => "needs-delta",
            RegisterClass::CitesExempt => "cites-exempt",
            RegisterClass::NamedType => "named-type",
            RegisterClass::SelfReference => "self-reference",
        }
    }

    /// Whether the loader admits a row of this class today (D5).
    #[must_use]
    pub fn admitted_today(self) -> bool {
        matches!(self, RegisterClass::NeedsDelta | RegisterClass::CitesExempt)
    }
}

/// One row: a theorem the kernel need not admit, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exemption {
    /// The defining file, relative to the repository root, `/`-separated.
    pub file: String,
    pub theorem: String,
    pub class: RegisterClass,
    /// The ADR that will remove the row.
    pub removed_by: String,
    /// For a `cites-exempt` row, the exempt theorems it relies on.
    pub cites: Vec<String>,
}

/// The loaded register.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Register {
    rows: Vec<Exemption>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegister {
    schema: u32,
    #[serde(default)]
    exemption: Vec<RawExemption>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExemption {
    file: String,
    theorem: String,
    cause: String,
    removed_by: Option<String>,
    #[allow(dead_code)] // Reason: read by humans; parsed so a typo in its key is refused
    note: Option<String>,
    cites: Option<Vec<String>>,
}

impl Register {
    /// No rows: the register of a path that has none to consult (D1).
    pub const EMPTY: Register = Register { rows: Vec::new() };

    /// Parse a register, refusing everything D4's load-time rules refuse.
    ///
    /// # Errors
    ///
    /// The first violated rule, naming the row.
    pub fn parse(source: &str) -> Result<Self, String> {
        let raw: RawRegister = toml::from_str(source).map_err(|e| e.to_string())?;
        if raw.schema != SCHEMA {
            return Err(format!("schema {} is not {SCHEMA}", raw.schema));
        }
        let mut rows = Vec::with_capacity(raw.exemption.len());
        for row in raw.exemption {
            rows.push(parse_row(row)?);
        }
        let mut keys = BTreeSet::new();
        for row in &rows {
            if !keys.insert((row.file.as_str(), row.theorem.as_str())) {
                return Err(format!("`{}` in {} is listed twice", row.theorem, row.file));
            }
        }
        let theorems: BTreeSet<&str> = rows.iter().map(|row| row.theorem.as_str()).collect();
        for row in &rows {
            if let Some(cited) = row.cites.iter().find(|c| !theorems.contains(c.as_str())) {
                return Err(format!(
                    "`{}` cites `{cited}`, which is not a row",
                    row.theorem
                ));
            }
        }
        Ok(Register { rows })
    }

    /// The register this binary was built with. A malformed one is a compiler
    /// bug, which `the_real_register_loads` keeps from shipping.
    #[must_use]
    pub fn real() -> &'static Register {
        static REAL: OnceLock<Register> = OnceLock::new();
        REAL.get_or_init(|| {
            Register::parse(REGISTER_TOML)
                .unwrap_or_else(|e| panic!("kernel_exemptions.toml is malformed: {e}"))
        })
    }

    /// The register the build consults: the real one, or — in a test, on
    /// this thread — one [`Register::inject`] put in its place.
    #[must_use]
    pub fn active() -> &'static Register {
        #[cfg(test)]
        if let Some(injected) = INJECTED.with(std::cell::Cell::get) {
            return injected;
        }
        Register::real()
    }

    /// Run `body` with `register` as this thread's active register, so a
    /// drill reaches the gate through the shared path (AC 4). The admission
    /// step runs on the caller thread, so a thread-local reaches it.
    #[cfg(test)]
    pub fn inject<R>(register: Register, body: impl FnOnce() -> R) -> R {
        let leaked: &'static Register = Box::leak(Box::new(register));
        let previous = INJECTED.with(|slot| slot.replace(Some(leaked)));
        let result = body();
        INJECTED.with(|slot| slot.set(previous));
        result
    }

    #[must_use]
    pub fn rows(&self) -> &[Exemption] {
        &self.rows
    }

    /// The row for `theorem` defined in `file`, if any.
    #[must_use]
    pub fn row(&self, file: &str, theorem: &str) -> Option<&Exemption> {
        self.rows
            .iter()
            .find(|row| row.file == file && row.theorem == theorem)
    }

    /// The exempt theorems of the files this run built: every row's theorem,
    /// `cites-exempt` rows included, so "exempt" is transitive (D4).
    #[must_use]
    pub fn exempt_names(&self, built_files: &BTreeSet<String>) -> BTreeSet<String> {
        self.rows
            .iter()
            .filter(|row| built_files.contains(&row.file))
            .map(|row| row.theorem.clone())
            .collect()
    }
}

fn parse_row(row: RawExemption) -> Result<Exemption, String> {
    let named = |rule: &str| format!("`{}` in {}: {rule}", row.theorem, row.file);
    let class = RegisterClass::ALL
        .into_iter()
        .find(|class| class.label() == row.cause)
        .ok_or_else(|| named(&format!("`{}` is not a register class", row.cause)))?;
    if !class.admitted_today() {
        return Err(named(&format!("`{}` is not admitted today", row.cause)));
    }
    let removed_by = row
        .removed_by
        .clone()
        .ok_or_else(|| named("no `removed_by`"))?;
    let cites = match (class, row.cites.clone()) {
        (RegisterClass::CitesExempt, Some(cites)) if !cites.is_empty() => cites,
        (RegisterClass::CitesExempt, _) => return Err(named("a cites-exempt row needs `cites`")),
        (_, Some(_)) => return Err(named("only a cites-exempt row has `cites`")),
        (_, None) => Vec::new(),
    };
    Ok(Exemption {
        file: row.file,
        theorem: row.theorem,
        class,
        removed_by,
        cites,
    })
}

/// The repository root: the parent of this crate's manifest directory.
#[must_use]
pub fn repository_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

/// `file`'s register key: its path under `root`, `/`-separated, or `None`
/// for a file outside it. Both paths must already be canonical.
#[must_use]
pub fn register_key(file: &Path, root: &Path) -> Option<String> {
    let relative = file.strip_prefix(root).ok()?;
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}
