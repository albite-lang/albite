//! The trigger (D9), the staleness test (D5) and the bound-change rule (D6).

use crate::record::RECORD_PATH;
use crate::{Bound, Environment};

/// D9's six prefixes: `albite1`'s source, the runtime it links, the code
/// generator that compiles it (with the type tables codegen expands), and the
/// link line.
pub const TRIGGER_PREFIXES: [&str; 6] = [
    "src/compiler/",
    "albite_core/src/",
    "albite_codegen/src/",
    "bootstrap/src/compile/",
    "bootstrap/src/driver/output/",
    "tools/albite-dev/src/commands/self_compile/",
];

/// True when the window's diff can change what `albite1 check main.tg`
/// costs, or touches the record itself (so D6's widening check runs).
#[must_use]
pub fn applies(paths: &[&str]) -> bool {
    paths.iter().any(|path| {
        *path == RECORD_PATH
            || (TRIGGER_PREFIXES.iter().any(|p| path.starts_with(p)) && !is_suite_file(path))
    })
}

/// P0.6: the `src/compiler/` suites (`test_*.tg`, `mustfail_*.tg`) are entry
/// files of their own, outside `main.tg`'s import graph, so a test-only window
/// pays no self-compile. A `test.tg` MODULE (`driver::ffi::test`) is imported,
/// and stays a trigger.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // `.tg` is never upper-case here
fn is_suite_file(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("src/compiler/") else {
        return false;
    };
    let file = rest.rsplit('/').next().unwrap_or(rest);
    file.ends_with(".tg") && (file.starts_with("test_") || file.starts_with("mustfail_"))
}

/// One `[environment]` field (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Kernel,
    Os,
    CpuModel,
    Llc,
    Rustc,
    Nproc,
    MemTotalKb,
}

impl Field {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Field::Kernel => "kernel",
            Field::Os => "os",
            Field::CpuModel => "cpu_model",
            Field::Llc => "llc",
            Field::Rustc => "rustc",
            Field::Nproc => "nproc",
            Field::MemTotalKb => "mem_total_kb",
        }
    }
}

/// D5: every field on which the running container differs from the record.
/// Any one makes the stored numbers meaningless.
#[must_use]
pub fn stale_fields(recorded: &Environment, current: &Environment) -> Vec<Field> {
    let checks = [
        (Field::Kernel, recorded.kernel == current.kernel),
        (Field::Os, recorded.os == current.os),
        (Field::CpuModel, recorded.cpu_model == current.cpu_model),
        (Field::Llc, recorded.llc == current.llc),
        (Field::Rustc, recorded.rustc == current.rustc),
        (Field::Nproc, recorded.nproc == current.nproc),
        (
            Field::MemTotalKb,
            recorded.mem_total_kb == current.mem_total_kb,
        ),
    ];
    checks
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(field, _)| field)
        .collect()
}

/// D6: how the tree's `[bound]` relates to the base's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundChange {
    Unchanged,
    /// Stricter, or a widening that names a new `adr`.
    Accepted,
    /// Looser with `adr` unchanged: a refusal, naming each widened field.
    Widened(Vec<&'static str>),
}

/// A larger pct, noise or drift ceiling, a smaller stored margin, or fewer
/// runs is a widening. Whether a new `adr` justified one is a reviewer's
/// question, not this gate's.
#[must_use]
pub fn bound_change(base: &Bound, tree: &Bound) -> BoundChange {
    if base == tree {
        return BoundChange::Unchanged;
    }
    let widened: Vec<&'static str> = [
        ("wall_pct", tree.wall_pct > base.wall_pct),
        ("rss_pct", tree.rss_pct > base.rss_pct),
        ("noise_pct", tree.noise_pct > base.noise_pct),
        (
            "drift_ceiling_pct",
            tree.drift_ceiling_pct > base.drift_ceiling_pct,
        ),
        (
            "stored_wall_margin_pct",
            tree.stored_wall_margin_pct < base.stored_wall_margin_pct,
        ),
        ("runs", tree.runs < base.runs),
    ]
    .into_iter()
    .filter(|(_, looser)| *looser)
    .map(|(name, _)| name)
    .collect();
    if widened.is_empty() || tree.adr != base.adr {
        BoundChange::Accepted
    } else {
        BoundChange::Widened(widened)
    }
}
