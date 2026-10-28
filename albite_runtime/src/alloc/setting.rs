//! How `ALBITE_ARENA` is read (ADR 14.9.26b), moved unchanged from
//! `alloc/mod.rs` (ADR 7.10.26s §2.2), plus the mapping a region-using
//! program's prologue applies (D4), and `ALBITE_REGION_POISON` (ADR
//! 13.10.26q D3). Both are read through [`crate::settings`] (ADR 20.10.26e).

use core::sync::atomic::{AtomicBool, Ordering};

/// The byte a poisoned release fills each chunk with (ADR 13.10.26q D3).
pub const POISON_BYTE: u8 = 0xA5;

/// Set once by a region-using program's prologue from
/// `ALBITE_REGION_POISON`; never cleared.
static REGION_POISON: AtomicBool = AtomicBool::new(false);

/// Whether `ALBITE_REGION_POISON` asks for poisoned releases: exactly `1`.
#[must_use]
pub fn region_poison_requested(value: Option<&str>) -> bool {
    value.map(str::trim) == Some("1")
}

/// Read the poison setting once, at init.
pub fn read_region_poison(value: Option<&str>) {
    REGION_POISON.store(region_poison_requested(value), Ordering::Relaxed);
}

/// Whether releases poison their chunks.
#[must_use]
pub fn region_poison_enabled() -> bool {
    REGION_POISON.load(Ordering::Relaxed)
}

use super::{ArenaMode, ARENA_MODE, CHUNK_BYTES, DEFAULT_CHUNK_BYTES, MODE_OFF};

/// How `ALBITE_ARENA` was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaSetting {
    /// A recognised value: the mode and, for `Bump`, the chunk size in bytes.
    Recognised { mode: ArenaMode, chunk_bytes: usize },
    /// An unrecognised value; the mode stays `Off` and one line says so.
    Unrecognised,
}

/// Parse an `ALBITE_ARENA` value: unset, empty or `off` → `Off`; `bump` →
/// `Bump` at the default chunk size; `bump:<n>` → `Bump` with `n` MiB chunks
/// (`n` ≥ 1). Anything else is `Unrecognised`, never a silent `Off`.
#[must_use]
pub fn parse_arena_setting(value: Option<&str>) -> ArenaSetting {
    let value = value.map_or("", str::trim);
    if value.is_empty() || value == "off" {
        return ArenaSetting::Recognised {
            mode: ArenaMode::Off,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
        };
    }
    let Some(rest) = value.strip_prefix("bump") else {
        return ArenaSetting::Unrecognised;
    };
    let chunk_bytes = match rest.strip_prefix(':') {
        None if rest.is_empty() => DEFAULT_CHUNK_BYTES,
        Some(mib) => match mib.parse::<usize>() {
            Ok(n) if n >= 1 => n.saturating_mul(1024 * 1024),
            _ => return ArenaSetting::Unrecognised,
        },
        None => return ArenaSetting::Unrecognised,
    };
    ArenaSetting::Recognised {
        mode: ArenaMode::Bump,
        chunk_bytes,
    }
}

/// Apply a parsed setting to the process globals — the programmatic half of
/// [`super::__albite_arena_init`], for hosts that embed the runtime and for
/// tests that need a mode without an environment. Process-global: a test that
/// sets `Bump` must serialise against every other test that frees a block.
pub fn apply_arena_setting(setting: ArenaSetting) {
    match setting {
        ArenaSetting::Recognised { mode, chunk_bytes } => {
            CHUNK_BYTES.store(chunk_bytes, Ordering::Relaxed);
            ARENA_MODE.store(mode as u32, Ordering::Relaxed);
        }
        ArenaSetting::Unrecognised => {
            eprintln!("tungsten runtime: ALBITE_ARENA not recognised (want off | bump[:chunk_mib]); arena stays off");
            ARENA_MODE.store(MODE_OFF, Ordering::Relaxed);
        }
    }
}

/// The line a region-using program prints for an unrecognised
/// `ALBITE_ARENA` (ADR 7.10.26s D4). It names `scoped` as the fallback:
/// [`apply_arena_setting`]'s "arena stays off" would be false here.
pub const UNRECOGNISED_REGIONS_LINE: &str =
    "tungsten runtime: ALBITE_ARENA not recognised (want off | bump[:chunk_mib]); falling back to scoped";

/// The mode a region-using program's prologue selects for a parsed setting
/// (ADR 7.10.26s D4): `bump[:n]` stays `Bump`, because regions work under it
/// too; `off`, unset and an unrecognised value all select `Scoped`, never
/// `Off`, because a region-using program must reach the allocation symbol.
/// The `bool` says whether the setting was unrecognised, so the caller prints
/// [`UNRECOGNISED_REGIONS_LINE`].
#[must_use]
pub fn regions_setting(setting: ArenaSetting) -> (ArenaMode, usize, bool) {
    match setting {
        ArenaSetting::Recognised {
            mode: ArenaMode::Bump,
            chunk_bytes,
        } => (ArenaMode::Bump, chunk_bytes, false),
        ArenaSetting::Recognised { chunk_bytes, .. } => (ArenaMode::Scoped, chunk_bytes, false),
        ArenaSetting::Unrecognised => (ArenaMode::Scoped, DEFAULT_CHUNK_BYTES, true),
    }
}

/// Apply [`regions_setting`] to the process globals, printing the scoped
/// fallback line for an unrecognised value.
pub fn apply_regions_setting(setting: ArenaSetting) {
    let (mode, chunk_bytes, unrecognised) = regions_setting(setting);
    if unrecognised {
        eprintln!("{UNRECOGNISED_REGIONS_LINE}");
    }
    CHUNK_BYTES.store(chunk_bytes, Ordering::Relaxed);
    ARENA_MODE.store(mode as u32, Ordering::Relaxed);
}
