//! ADR 16.10.26j AC 2: the amortised policy is a pure function, its leaf is
//! tested over an injected allocator, and the export keeps every other arm's
//! contract — mode `Bump` and an entered region are `__albite_realloc`.
//!
//! Tests that set the arena mode or enter a region hold the runtime's
//! `ModeGuard`, which leaves the mode `Off` on drop.

use core::cell::RefCell;

use super::*;
use crate::alloc::tests::ModeGuard;
use crate::alloc::{
    __albite_alloc, __albite_realloc, __albite_region_detach, __albite_region_enter,
    __albite_region_release, apply_arena_setting, apply_regions_setting, current_arena_stats,
    parse_arena_setting, ArenaMode, ArenaSetting,
};
use crate::alloc_profile::{CLASS_MU, CLASS_STRING};

// ── the pure policy ─────────────────────────────────────────────────────────

/// ADR 16.10.26j AC 2: the factor is 5/2 — P0.2 qualified 2, 5/2 and 3, and
/// D8's over-budget remedy took the next after 2.
#[test]
fn the_growth_factor_is_p0_2s_pick() {
    assert_eq!(
        AMORTISED_GROWTH_FACTOR,
        GrowthFactor {
            numerator: 5,
            denominator: 2
        }
    );
}

/// ADR 16.10.26j AC 2: a block that already holds the request makes no call.
#[test]
fn a_grow_within_the_room_asks_for_nothing() {
    assert_eq!(amortised_request(64, 64), None);
    assert_eq!(amortised_request(64, 1), None);
    assert_eq!(amortised_request(0, 0), None);
}

/// ADR 16.10.26j AC 2: one byte past the room asks for `F ×` the room.
#[test]
fn a_grow_past_the_room_asks_for_the_factor() {
    assert_eq!(amortised_request(64, 65), Some(160));
    assert_eq!(amortised_request(24, 40), Some(60));
}

/// ADR 16.10.26j AC 2: a request larger than `F ×` the room asks for itself.
#[test]
fn a_request_beyond_the_factor_asks_for_itself() {
    assert_eq!(amortised_request(64, 161), Some(161));
    assert_eq!(amortised_request(0, 24), Some(24));
}

/// ADR 16.10.26j AC 2: the multiply saturates rather than wrapping.
#[test]
fn the_factor_saturates_at_usize_max() {
    let half = usize::MAX / 2 + 1;
    assert_eq!(amortised_request(half, half + 1), Some(usize::MAX));
    assert_eq!(scaled(half, AMORTISED_GROWTH_FACTOR), usize::MAX);
    let third = GrowthFactor {
        numerator: 3,
        denominator: 2,
    };
    assert_eq!(scaled(10, third), 15, "the ratio divides after multiplying");
}

// ── the leaf over an injected allocator ─────────────────────────────────────

/// Every `realloc` the leaf made, and an answer per call.
struct FakeAllocator {
    usable: usize,
    calls: RefCell<Vec<(*mut u8, usize)>>,
    answers: RefCell<Vec<*mut u8>>,
    usable_asked: RefCell<usize>,
}

impl FakeAllocator {
    fn new(usable: usize, answers: Vec<*mut u8>) -> Self {
        Self {
            usable,
            calls: RefCell::new(Vec::new()),
            answers: RefCell::new(answers),
            usable_asked: RefCell::new(0),
        }
    }

    fn grow(&self, block: *mut u8, new: usize) -> *mut u8 {
        // SAFETY: the fake's blocks are never dereferenced.
        unsafe {
            amortised_leaf(
                block,
                new,
                &mut |_| {
                    *self.usable_asked.borrow_mut() += 1;
                    self.usable
                },
                &mut |ptr, size| {
                    self.calls.borrow_mut().push((ptr, size));
                    self.answers.borrow_mut().remove(0)
                },
            )
        }
    }
}

fn fake_block(address: usize) -> *mut u8 {
    core::ptr::without_provenance_mut(address)
}

/// ADR 16.10.26j AC 2: within the usable size the block is returned, no call.
#[test]
fn the_leaf_returns_the_block_when_it_fits() {
    let block = fake_block(0x1000);
    let fake = FakeAllocator::new(40, Vec::new());
    assert_eq!(fake.grow(block, 40), block);
    assert!(fake.calls.borrow().is_empty(), "no realloc when it fits");
    assert_eq!(*fake.usable_asked.borrow(), 1);
}

/// ADR 16.10.26j AC 2: past it, one realloc of `max(new, F × usable)`.
#[test]
fn the_leaf_asks_for_the_amortised_size_when_it_moves() {
    let (block, moved) = (fake_block(0x1000), fake_block(0x2000));
    let fake = FakeAllocator::new(40, vec![moved]);
    assert_eq!(fake.grow(block, 41), moved);
    assert_eq!(*fake.calls.borrow(), vec![(block, 100)]);
}

/// ADR 16.10.26j AC 2: a null block is an allocation of exactly `new`, and
/// its usable size is never asked.
#[test]
fn a_null_block_is_an_exact_allocation() {
    let fresh = fake_block(0x3000);
    let fake = FakeAllocator::new(999, vec![fresh]);
    assert_eq!(fake.grow(core::ptr::null_mut(), 24), fresh);
    assert_eq!(*fake.calls.borrow(), vec![(core::ptr::null_mut(), 24)]);
    assert_eq!(
        *fake.usable_asked.borrow(),
        0,
        "malloc_size(NULL) is undocumented"
    );
}

/// ADR 16.10.26j AC 2: when the amortised request fails, the leaf retries
/// exactly `new` — a failed realloc leaves the block valid.
#[test]
fn out_of_memory_retries_the_exact_request() {
    let (block, moved) = (fake_block(0x1000), fake_block(0x4000));
    let fake = FakeAllocator::new(40, vec![core::ptr::null_mut(), moved]);
    assert_eq!(fake.grow(block, 41), moved);
    assert_eq!(*fake.calls.borrow(), vec![(block, 100), (block, 41)]);
}

/// ADR 16.10.26j AC 2: an exact request that fails is not retried; the null
/// goes back to the caller, which aborts.
#[test]
fn a_failed_exact_request_is_not_retried() {
    let block = fake_block(0x1000);
    let fake = FakeAllocator::new(40, vec![core::ptr::null_mut()]);
    assert!(fake.grow(block, 200).is_null());
    assert_eq!(*fake.calls.borrow(), vec![(block, 200)]);
}

// ── the export on the real allocator ────────────────────────────────────────

/// ADR 16.10.26j AC 2: in mode `Off` a grow within the usable size stays put
/// with its bytes, and a grow past it gets at least `F ×` the old room.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[test]
fn off_mode_grows_in_the_slack_then_by_the_factor() {
    let _guard = ModeGuard::take();
    // SAFETY: `malloc` blocks, freed at the end.
    unsafe {
        let block = __albite_alloc(24, CLASS_STRING).cast::<u8>();
        block.write_bytes(0x61, 24);
        let room = platform_usable_size(block);
        // The usable size is the allocator's answer, never below the request
        // and never absurd: glibc's chunk for 24 bytes is 24, macOS's 32.
        assert!((24..=64).contains(&room), "usable size {room}");
        let same = __albite_realloc_amortised(block.cast(), 24, room as u64).cast::<u8>();
        assert_eq!(same, block, "within the usable size: no move");
        assert!(std::slice::from_raw_parts(same, 24)
            .iter()
            .all(|&b| b == 0x61));
        let grown =
            __albite_realloc_amortised(same.cast(), room as u64, room as u64 + 1).cast::<u8>();
        assert!(
            platform_usable_size(grown) >= room * 5 / 2,
            "room for the next grows"
        );
        assert!(std::slice::from_raw_parts(grown, 24)
            .iter()
            .all(|&b| b == 0x61));
        libc::free(grown.cast());
    }
}

/// ADR 16.10.26j AC 2: in mode `Bump` the export is `__albite_realloc` —
/// in place at the cursor, using exactly `new` bytes, then a copy.
#[test]
fn bump_mode_is_the_exact_grow() {
    let _guard = ModeGuard::take();
    apply_arena_setting(ArenaSetting::Recognised {
        mode: ArenaMode::Bump,
        chunk_bytes: 1024 * 1024,
    });
    // SAFETY: arena blocks, never freed.
    unsafe {
        let block = __albite_alloc(8, CLASS_STRING).cast::<u8>();
        block.write_bytes(0x42, 8);
        let before = current_arena_stats().used;
        let grown = __albite_realloc_amortised(block.cast(), 8, 40).cast::<u8>();
        assert_eq!(grown, block, "the last block grows in place");
        assert_eq!(
            current_arena_stats().used - before,
            32,
            "exactly new, no factor"
        );
        let _later = __albite_alloc(16, CLASS_MU);
        let moved = __albite_realloc_amortised(grown.cast(), 40, 80).cast::<u8>();
        assert_ne!(moved, grown, "no longer last: a copy");
        let twin = __albite_alloc(40, CLASS_STRING);
        assert_eq!(
            twin as usize - moved as usize,
            80,
            "the copy took exactly 80 bytes"
        );
        assert_eq!(
            __albite_realloc(twin, 40, 80),
            twin,
            "as __albite_realloc's last block"
        );
        assert!(std::slice::from_raw_parts(moved, 8)
            .iter()
            .all(|&b| b == 0x42));
    }
}

/// ADR 16.10.26j AC 2: with a region entered the export grows in the region
/// at its cursor, as `__albite_realloc` does, and never reaches libc.
#[test]
fn an_entered_region_is_the_exact_grow() {
    let _guard = ModeGuard::take();
    apply_regions_setting(parse_arena_setting(None));
    __albite_region_enter();
    // SAFETY: region blocks, released below.
    unsafe {
        let block = __albite_alloc(8, CLASS_STRING).cast::<u8>();
        block.write_bytes(0x33, 8);
        let grown = __albite_realloc_amortised(block.cast(), 8, 40).cast::<u8>();
        assert_eq!(grown, block, "the region's last block grows in place");
        let next = __albite_alloc(8, CLASS_MU).cast::<u8>();
        assert_eq!(
            next as usize - grown as usize,
            48,
            "exactly 40 bytes, 16-aligned"
        );
        assert!(std::slice::from_raw_parts(grown, 8)
            .iter()
            .all(|&b| b == 0x33));
    }
    __albite_region_detach();
    __albite_region_release();
}
