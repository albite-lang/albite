//! Which symbol a `++` calls (ADR 4.10.26n D6, 6.10.26n D4).
//!
//! A pure function of three facts: whether the left operand is a dead heap
//! temporary (the owned, realloc-in-place path, 18.5.26f), the
//! `ALBITE_NATIVE_CORE` switch, and the export rows with their defaults.
//! Each path moves to its row's shim exactly when the switch enables that row.

use crate::codegen::native_core::{NativeCoreExport, NativeCoreSwitch};
use albite_core::native_core::{owned_implies_concat, shadowing_symbol};

/// The Rust export every non-owned `++` calls with its row disabled.
pub(crate) const RUST_CONCAT: &str = "tg_string_concat";
/// The Rust export an owned `++` calls with its row disabled.
pub(crate) const RUST_CONCAT_OWNED: &str = "tg_string_concat_owned";
/// The export written in Tungsten a non-owned `++` calls with its row enabled.
#[cfg(test)]
pub(crate) const NATIVE_CONCAT: &str = "tg_native_string_concat";
/// The export written in Tungsten an owned `++` calls with its row enabled.
#[cfg(test)]
pub(crate) const NATIVE_CONCAT_OWNED: &str = "tg_native_string_concat_owned";

/// The symbol a `++` calls: `owned` when its left operand is a dead heap
/// temporary.
///
/// The owned shim adopts its left operand's block as `core_buffer_layout`'s
/// (6.10.26n D2), which holds only when that operand came from the core's own
/// run — so a row table enabling the owned row without the concat row is a
/// bug, asserted in every debug build that lowers a `++`.
pub(crate) fn concat_symbol(
    owned: bool,
    switch: NativeCoreSwitch,
    rows: &[NativeCoreExport],
) -> &'static str {
    let enabled = switch.enabled_rows();
    debug_assert!(
        owned_implies_concat(rows, enabled),
        "an owned native concat is enabled without the native concat ({switch:?})"
    );
    let shadowed = if owned {
        RUST_CONCAT_OWNED
    } else {
        RUST_CONCAT
    };
    shadowing_symbol(rows, enabled, shadowed)
}
