//! ADR 14.9.26b AC 2 / AC 8: env parsing, `off` delegating to the platform
//! allocator, `bump` bumping the thread arena, the realloc shim never handing
//! an arena pointer to libc, mode `off` when init never ran, and the aliased
//! profiler symbol no longer self-activating.
//!
//! `ARENA_MODE` and `CHUNK_BYTES` are process globals, so every test that sets
//! or reads them holds `MODE_LOCK` and leaves the mode `Off` on the way out —
//! the state a process in which `__albite_arena_init` never ran is in.

use super::*;
use crate::alloc_profile::{alloc_profile_is_active, CLASS_MU, CLASS_STRING};
use std::sync::{Mutex, MutexGuard};

static MODE_LOCK: Mutex<()> = Mutex::new(());

/// Hold the global-mode lock and restore `Off` on drop, even on a panic.
/// `pub(super)` so the copy-out's export test takes the same lock.
pub(super) struct ModeGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

impl ModeGuard {
    pub(super) fn take() -> Self {
        Self(
            MODE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Set the mode under the lock the guard holds — `&self` is the proof.
    #[allow(clippy::unused_self)] // Reason: `&self` is the proof the mode lock is held
    fn set(&self, setting: ArenaSetting) {
        apply_arena_setting(setting);
    }
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        apply_arena_setting(parse_arena_setting(None));
    }
}

const BUMP_1MIB: ArenaSetting = ArenaSetting::Recognised {
    mode: ArenaMode::Bump,
    chunk_bytes: 1024 * 1024,
};

#[test]
fn unset_empty_and_off_parse_as_off() {
    let off = ArenaSetting::Recognised {
        mode: ArenaMode::Off,
        chunk_bytes: DEFAULT_CHUNK_BYTES,
    };
    assert_eq!(parse_arena_setting(None), off);
    assert_eq!(parse_arena_setting(Some("")), off);
    assert_eq!(parse_arena_setting(Some("  ")), off);
    assert_eq!(parse_arena_setting(Some("off")), off);
    assert_eq!(parse_arena_setting(Some(" off ")), off);
}

#[test]
fn bump_parses_with_default_and_explicit_chunk_sizes() {
    assert_eq!(
        parse_arena_setting(Some("bump")),
        ArenaSetting::Recognised {
            mode: ArenaMode::Bump,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
        }
    );
    assert_eq!(
        parse_arena_setting(Some("bump:16")),
        ArenaSetting::Recognised {
            mode: ArenaMode::Bump,
            chunk_bytes: 16 * 1024 * 1024,
        }
    );
    assert_eq!(parse_arena_setting(Some("bump:1")), BUMP_1MIB);
}

#[test]
fn unrecognised_values_are_reported_not_silently_off() {
    for bad in ["on", "bmup", "bump:", "bump:0", "bump:x", "bump16", "arena"] {
        assert_eq!(
            parse_arena_setting(Some(bad)),
            ArenaSetting::Unrecognised,
            "{bad:?}"
        );
    }
}

#[test]
fn mode_is_off_when_init_never_ran() {
    let _guard = ModeGuard::take();
    assert_eq!(arena_mode(), ArenaMode::Off);
    assert_eq!(current_arena_stats().mode, MODE_OFF);
}

/// ADR 18.9.26c AC 2: generated code branches on the exported flag, so it must
/// hold exactly the discriminant `apply_arena_setting` last stored.
#[test]
fn exported_mode_flag_tracks_apply_arena_setting() {
    let guard = ModeGuard::take();
    assert_eq!(ARENA_MODE.load(Ordering::Relaxed), MODE_OFF);
    guard.set(BUMP_1MIB);
    assert_eq!(ARENA_MODE.load(Ordering::Relaxed), MODE_BUMP);
    guard.set(ArenaSetting::Unrecognised);
    assert_eq!(ARENA_MODE.load(Ordering::Relaxed), MODE_OFF);
    guard.set(BUMP_1MIB);
    guard.set(parse_arena_setting(Some("off")));
    assert_eq!(ARENA_MODE.load(Ordering::Relaxed), MODE_OFF);
}

#[test]
fn applying_an_unrecognised_setting_leaves_the_mode_off() {
    let guard = ModeGuard::take();
    guard.set(BUMP_1MIB);
    assert_eq!(arena_mode(), ArenaMode::Bump);
    guard.set(ArenaSetting::Unrecognised);
    assert_eq!(arena_mode(), ArenaMode::Off);
}

#[test]
fn off_delegates_to_the_platform_allocator_and_takes_no_chunk() {
    let _guard = ModeGuard::take();
    let before = current_arena_stats();
    // SAFETY: `malloc` contract; freed below.
    let p = unsafe { __albite_alloc(64, CLASS_MU) };
    assert!(!p.is_null());
    assert_eq!(
        current_arena_stats().chunks,
        before.chunks,
        "no arena chunk"
    );
    assert_eq!(current_arena_stats().used, before.used);
    // A libc block is a libc block: `free` is the proof it did not come from
    // the arena (an arena pointer here would corrupt the heap).
    // SAFETY: `p` came from `platform_malloc` in mode `Off`.
    unsafe { libc::free(p) };
}

#[test]
fn bump_hands_out_aligned_blocks_from_the_thread_arena() {
    let guard = ModeGuard::take();
    guard.set(BUMP_1MIB);
    let before = current_arena_stats();
    // SAFETY: arena blocks; never freed.
    let a = unsafe { __albite_alloc(24, CLASS_MU) };
    let b = unsafe { __albite_alloc(8, CLASS_STRING) };
    assert!(!a.is_null() && !b.is_null());
    assert_eq!(a as usize % 16, 0);
    assert_eq!(b as usize % 16, 0);
    assert_eq!(b as usize - a as usize, 32, "24 rounds up to the next 16");
    let after = current_arena_stats();
    assert_eq!(after.mode, MODE_BUMP);
    assert!(after.chunks >= 1);
    assert_eq!(after.used - before.used, 32 + 8);
    assert!(after.high_water >= after.used);
}

#[test]
fn bump_realloc_grows_the_last_block_in_place_and_copies_otherwise() {
    let guard = ModeGuard::take();
    guard.set(BUMP_1MIB);
    // SAFETY: arena blocks; never freed.
    let block = unsafe { __albite_alloc(8, CLASS_STRING) }.cast::<u8>();
    unsafe { block.write_bytes(0x42, 8) };
    let grown = unsafe { __albite_realloc(block.cast(), 8, 40) }.cast::<u8>();
    assert_eq!(grown, block, "last block: in place");
    let _later = unsafe { __albite_alloc(16, CLASS_MU) };
    let moved = unsafe { __albite_realloc(grown.cast(), 40, 80) }.cast::<u8>();
    assert_ne!(moved, grown, "no longer last: a copy");
    // SAFETY: the first 8 bytes were written and copied twice.
    assert!(unsafe { std::slice::from_raw_parts(moved, 8) }
        .iter()
        .all(|&x| x == 0x42));
}

#[test]
fn bump_realloc_of_null_is_an_allocation() {
    let guard = ModeGuard::take();
    guard.set(BUMP_1MIB);
    let before = current_arena_stats().used;
    // SAFETY: null is allowed; the block is never freed.
    let p = unsafe { __albite_realloc(core::ptr::null_mut(), 0, 24) };
    assert!(!p.is_null());
    assert_eq!(current_arena_stats().used - before, 24);
}

#[test]
fn off_realloc_is_libc_realloc() {
    let _guard = ModeGuard::take();
    // SAFETY: `malloc`/`realloc`/`free` contracts.
    unsafe {
        let p = __albite_alloc(4, CLASS_STRING).cast::<u8>();
        p.write_bytes(0x7, 4);
        let q = __albite_realloc(p.cast(), 4, 1024).cast::<u8>();
        assert!(!q.is_null());
        assert!(std::slice::from_raw_parts(q, 4).iter().all(|&x| x == 0x7));
        libc::free(q.cast());
    }
}

#[test]
fn zero_byte_requests_are_non_null_in_bump_mode() {
    let guard = ModeGuard::take();
    guard.set(BUMP_1MIB);
    // SAFETY: a zero-byte block.
    let p = unsafe { __albite_alloc(0, CLASS_MU) };
    assert!(!p.is_null(), "generated code never null-checks");
}

#[test]
fn reset_returns_every_chunk_but_the_first() {
    let guard = ModeGuard::take();
    guard.set(BUMP_1MIB);
    // Three requests of 3/4 MiB force three chunks of 1 MiB.
    for _ in 0..3 {
        // SAFETY: arena blocks; released by reset.
        assert!(!unsafe { __albite_alloc(768 * 1024, CLASS_MU) }.is_null());
    }
    let before = current_arena_stats();
    assert!(before.chunks >= 3);
    __albite_arena_reset();
    let after = current_arena_stats();
    assert_eq!(after.chunks, 1);
    assert_eq!(after.used, 0);
    assert_eq!(after.high_water, before.high_water);
}

#[test]
fn arena_stats_writes_through_the_out_pointer_and_ignores_null() {
    let _guard = ModeGuard::take();
    let mut out = ArenaStatsOut {
        mode: 99,
        ..ArenaStatsOut::default()
    };
    // SAFETY: `out` is a valid, writable local.
    unsafe { __albite_arena_stats(&mut out) };
    assert_eq!(out.mode, MODE_OFF);
    assert_eq!(out, current_arena_stats());
    // SAFETY: null is documented as a no-op.
    unsafe { __albite_arena_stats(core::ptr::null_mut()) };
}

#[test]
fn mode_discriminants_are_the_published_constants() {
    assert_eq!(ArenaMode::Off as u32, MODE_OFF);
    assert_eq!(ArenaMode::Bump as u32, MODE_BUMP);
    assert_ne!(MODE_OFF, MODE_BUMP);
}

/// ADR 14.9.26b AC 8: the aliased profiler symbol records only through the
/// activation gate, so calling it does not switch the profiler on.
#[test]
fn aliased_profiler_symbol_no_longer_self_activates() {
    let _guard = ModeGuard::take();
    assert!(
        !alloc_profile_is_active(),
        "precondition: nothing activated it"
    );
    // SAFETY: `malloc` contract; freed below.
    let p = unsafe { __albite_alloc_profile_malloc_class(32, CLASS_MU) };
    let q = unsafe { __albite_alloc_profile_malloc(32) };
    assert!(!p.is_null() && !q.is_null());
    assert!(
        !alloc_profile_is_active(),
        "the alias must not activate the profiler as a side effect"
    );
    // SAFETY: both came from `platform_malloc` in mode `Off`.
    unsafe {
        libc::free(p);
        libc::free(q);
    }
}

/// `__albite_arena_init` reads `ALBITE_ARENA` from the process
/// environment — the prologue's route. Set under the lock, restored after.
#[test]
fn arena_init_reads_the_environment_variable() {
    let _guard = ModeGuard::take();
    // SAFETY: the environment is process-global; every test that reads the
    // mode holds `MODE_LOCK`, and the variable is removed before the lock drops.
    unsafe { std::env::set_var("ALBITE_ARENA", "bump:2") };
    __albite_arena_init();
    unsafe { std::env::remove_var("ALBITE_ARENA") };
    assert_eq!(arena_mode(), ArenaMode::Bump);
    assert_eq!(CHUNK_BYTES.load(Ordering::Relaxed), 2 * 1024 * 1024);
    __albite_arena_init();
    assert_eq!(arena_mode(), ArenaMode::Off, "unset reads as off");
}

/// ADR 20.10.26e: the legacy `TUNGSTEN_ARENA` spelling is still honoured by
/// the prologue's route when the new name is unset.
#[test]
fn arena_init_still_reads_the_legacy_name() {
    let _guard = ModeGuard::take();
    // SAFETY: as above — set and removed under `MODE_LOCK`.
    unsafe { std::env::set_var(crate::settings::legacy_name("ARENA"), "bump:3") };
    __albite_arena_init();
    unsafe { std::env::remove_var(crate::settings::legacy_name("ARENA")) };
    assert_eq!(arena_mode(), ArenaMode::Bump);
    assert_eq!(CHUNK_BYTES.load(Ordering::Relaxed), 3 * 1024 * 1024);
    __albite_arena_init();
    assert_eq!(arena_mode(), ArenaMode::Off, "unset reads as off");
}

/// The abort decision: null for a non-zero request fails; null for zero bytes
/// is a legal `malloc(0)` answer; a block never fails.
#[test]
fn allocation_failed_only_for_a_null_non_zero_request() {
    let block = 16usize as *mut u8;
    assert!(allocation_failed(core::ptr::null_mut(), 1));
    assert!(!allocation_failed(core::ptr::null_mut(), 0));
    assert!(!allocation_failed(block, 0));
    assert!(!allocation_failed(block, 1));
}

/// ADR 7.10.26s AC 3: a region-using prologue selects `Scoped` for unset,
/// `off` and an unrecognised value, and keeps `Bump` with its chunk size.
#[test]
fn the_regions_mapping_selects_scoped_unless_bump_is_named() {
    let scoped = (ArenaMode::Scoped, DEFAULT_CHUNK_BYTES, false);
    assert_eq!(regions_setting(parse_arena_setting(None)), scoped);
    assert_eq!(regions_setting(parse_arena_setting(Some("off"))), scoped);
    assert_eq!(
        regions_setting(parse_arena_setting(Some("arena"))),
        (ArenaMode::Scoped, DEFAULT_CHUNK_BYTES, true)
    );
    assert_eq!(
        regions_setting(parse_arena_setting(Some("bump:4"))),
        (ArenaMode::Bump, 4 * 1024 * 1024, false)
    );
    assert!(UNRECOGNISED_REGIONS_LINE.contains("falling back to scoped"));
    assert!(!UNRECOGNISED_REGIONS_LINE.contains("arena stays off"));
}

/// ADR 7.10.26s AC 3: the third discriminant is 2, distinct, stored by the
/// regions init and decoded back as `Scoped` rather than falling to `Off`.
#[test]
fn scoped_is_mode_two_and_reads_back_as_scoped() {
    let _guard = ModeGuard::take();
    assert_eq!(MODE_SCOPED, 2);
    assert_ne!(MODE_SCOPED, MODE_OFF);
    assert_ne!(MODE_SCOPED, MODE_BUMP);
    apply_regions_setting(parse_arena_setting(Some("nonsense")));
    assert_eq!(ARENA_MODE.load(Ordering::Relaxed), MODE_SCOPED);
    assert_eq!(arena_mode(), ArenaMode::Scoped);
    apply_regions_setting(parse_arena_setting(Some("bump:2")));
    assert_eq!(arena_mode(), ArenaMode::Bump);
    assert_eq!(CHUNK_BYTES.load(Ordering::Relaxed), 2 * 1024 * 1024);
    // SAFETY: the environment is process-global; read under `MODE_LOCK`.
    unsafe { std::env::remove_var("ALBITE_ARENA") };
    __albite_arena_init_regions();
    assert_eq!(arena_mode(), ArenaMode::Scoped, "unset reads as scoped");
}

/// ADR 7.10.26s AC 3: `Scoped` with no region entered is `malloc(3)` and
/// `realloc(3)`, taking no arena chunk, as `off` does.
#[test]
fn scoped_with_an_empty_stack_takes_no_arena_chunk() {
    let _guard = ModeGuard::take();
    apply_regions_setting(parse_arena_setting(None));
    let before = current_arena_stats();
    // SAFETY: `malloc`/`realloc`/`free` contracts — mode `Scoped`, no region.
    unsafe {
        let block = __albite_alloc(48, CLASS_MU).cast::<u8>();
        block.write_bytes(0x3, 48);
        let grown = __albite_realloc(block.cast(), 48, 4096).cast::<u8>();
        assert!(std::slice::from_raw_parts(grown, 48)
            .iter()
            .all(|&x| x == 0x3));
        libc::free(grown.cast());
    }
    assert_eq!(current_arena_stats().chunks, before.chunks);
    assert_eq!(current_arena_stats().mode, MODE_SCOPED);
}

/// ADR 7.10.26s AC 3: the parent pair takes nothing from an entered region,
/// in either the scoped or the bump mode, while `__albite_alloc` does.
#[test]
fn the_parent_pair_ignores_an_entered_region() {
    let _guard = ModeGuard::take();
    for (setting, scoped) in [(None, true), (Some("bump:1"), false)] {
        apply_regions_setting(parse_arena_setting(setting));
        assert_eq!(arena_mode() == ArenaMode::Scoped, scoped);
        __albite_region_enter();
        // SAFETY: blocks from the parent pair and the region; the region's
        // are released below, the scoped parent block freed by libc.
        unsafe {
            let inside = __albite_alloc(32, CLASS_STRING);
            let parent = __albite_alloc_parent(32, CLASS_STRING);
            assert!(!parent.is_null(), "the parent allocator hands out a block");
            parent.cast::<u8>().write_bytes(0x5c, 32);
            let grown = __albite_realloc_parent(parent, 32, 8192);
            assert!(!grown.is_null(), "the parent realloc hands out a block");
            assert!(std::slice::from_raw_parts(grown.cast::<u8>(), 32)
                .iter()
                .all(|&b| b == 0x5c));
            __albite_region_detach();
            assert!(detached_region_contains(inside.cast()));
            assert!(!detached_region_contains(parent.cast()));
            assert!(!detached_region_contains(grown.cast()));
            if scoped {
                libc::free(grown);
            }
        }
        __albite_region_release();
        assert!(!detached_region_contains(core::ptr::null()));
    }
}
