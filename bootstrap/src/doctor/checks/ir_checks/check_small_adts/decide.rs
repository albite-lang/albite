//! The small-ADT census's decision rule (ADR 28.9.26f Decisions, §2.2): four
//! integer comparisons that turn the self-compile counts into B19's scope.
//!
//! The shares are judgement, and this is the one place to overrule them.
//! Integer cross-multiplication rather than floats keeps every boundary
//! mutation-killable.

use super::scan::SmallAdtCounts;

/// Every arm needs at least this many sites, whatever its share.
pub const FLOOR: u64 = 20;
/// Nullary cells: one heap fold in ten stores an empty constructor.
pub const NULLARY_SHARE_DEN: u64 = 10;
/// Small-sum registers: one memory-buffer parameter in twenty is ≤ 8 bytes.
pub const SMALL_SUM_SHARE_DEN: u64 = 20;
/// Pointer niche: one blob construction in twenty wraps a single pointer.
pub const NICHE_SHARE_DEN: u64 = 20;

/// One arm's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmVerdict {
    Admit,
    Park,
}

impl ArmVerdict {
    fn from_admitted(admitted: bool) -> Self {
        if admitted {
            Self::Admit
        } else {
            Self::Park
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admit => "admit",
            Self::Park => "park",
        }
    }
}

/// What the emit log said about `ptr` fallbacks. Never changes `b19`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackVerdict {
    None,
    RouteToCorrectness,
    /// No `--emit-log` was given.
    Unknown,
}

impl FallbackVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RouteToCorrectness => "route-to-correctness",
            Self::Unknown => "unknown",
        }
    }
}

/// The plan item's fate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum B19Verdict {
    Rescope,
    Close,
}

impl B19Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rescope => "rescope",
            Self::Close => "close",
        }
    }
}

/// The whole verdict block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub nullary_cells: ArmVerdict,
    pub small_sum_registers: ArmVerdict,
    pub pointer_niche: ArmVerdict,
    pub fallbacks: FallbackVerdict,
    pub b19: B19Verdict,
}

/// An arm is admitted when its part meets the absolute floor AND is at least
/// one `share_den`-th of the whole.
pub fn admitted(part: u64, whole: u64, share_den: u64) -> bool {
    part >= FLOOR && part * share_den >= whole
}

/// Apply the rule. Pure.
pub fn decide(counts: &SmallAdtCounts) -> Verdict {
    let nullary_cells = ArmVerdict::from_admitted(admitted(
        counts.nullary_mu_alloc_sites,
        counts.mu_alloc_sites,
        NULLARY_SHARE_DEN,
    ));
    let small_sum_registers = ArmVerdict::from_admitted(admitted(
        counts.small_blob_buffer_sites(),
        counts.blob_buffer_sites(),
        SMALL_SUM_SHARE_DEN,
    ));
    let pointer_niche = ArmVerdict::from_admitted(admitted(
        counts.pointer_payload_constructions,
        counts.blob_constructions(),
        NICHE_SHARE_DEN,
    ));
    let fallbacks = match counts.pointer_fallbacks {
        None => FallbackVerdict::Unknown,
        Some(0) => FallbackVerdict::None,
        Some(_) => FallbackVerdict::RouteToCorrectness,
    };
    let any_admitted =
        [nullary_cells, small_sum_registers, pointer_niche].contains(&ArmVerdict::Admit);
    Verdict {
        nullary_cells,
        small_sum_registers,
        pointer_niche,
        fallbacks,
        b19: if any_admitted {
            B19Verdict::Rescope
        } else {
            B19Verdict::Close
        },
    }
}
