//! Edge cases of the region copy-out's walk, and its C face (ADR 13.10.26q
//! AC 2).
//!
//! The walk's index arithmetic is tested where a neighbouring word would give
//! the same answer by accident: a third variant, a tag past the last one, a
//! node one past the end, and a variant whose size words sit beside words of
//! a different meaning. The export and the poison setting are process
//! globals, so they are driven under the allocator tests' mode lock.

use super::copy_out::{
    copy_out_with, CopyTally, MalformedLayout, KIND_BOX, KIND_STRING, KIND_STRUCT, KIND_TAGGED,
    NO_NODE,
};
use super::copy_out_tests::{layout, read_string, string_slot, Fixture};
use super::setting::{read_region_poison, region_poison_enabled};
use super::tests::ModeGuard;
use super::{
    __albite_alloc, __albite_region_copy_out, __albite_region_detach, __albite_region_enter,
    __albite_region_release, apply_regions_setting, detached_region_contains, parse_arena_setting,
    POISON_BYTE,
};
use crate::alloc_profile::CLASS_STRING;

/// Walk `slot` with nothing in any region, so nothing may be allocated.
///
/// # Safety
///
/// `slot` is valid for the reads `words` implies.
unsafe fn walk_outside(slot: *mut u8, words: &[u64]) -> Result<CopyTally, MalformedLayout> {
    let never = |_: usize| false;
    let mut no_alloc = |_: usize| -> *mut u8 { panic!("nothing is in the region") };
    // SAFETY: the caller's contract.
    unsafe { copy_out_with(slot, words, &never, &mut no_alloc) }
}

/// `A(String) | B(String) | C(String)` by value, the payload at 8, with each
/// variant's node given: word 1 is the union, word 10 the string.
fn three_variants(nodes: [u64; 3]) -> Vec<u64> {
    layout(&[
        KIND_TAGGED,
        8,
        3,
        16,
        nodes[0],
        16,
        nodes[1],
        16,
        nodes[2],
        KIND_STRING,
    ])
}

// 13.10.26q AC2: a child node one past the layout's end is refused at the
// word that names it, never read.
#[test]
fn a_node_one_past_the_end_is_refused_where_it_is_named() {
    let words = layout(&[KIND_STRUCT, 1, 0, 5]);
    assert_eq!(words.len(), 5, "node 5 is one past the last word");
    let mut slot = [0u64; 1];
    // SAFETY: the walk refuses before it reads a pointer.
    let refused = unsafe { walk_outside(slot.as_mut_ptr().cast(), &words) };
    assert_eq!(refused, Err(MalformedLayout { word: 4 }));
}

// 13.10.26q AC2: the third variant's own node word decides, so a pointer-free
// third variant beside two string variants copies nothing.
#[test]
fn a_pointer_free_third_variant_copies_nothing() {
    let words = three_variants([10, 10, NO_NODE]);
    let mut slot = [2u64, 0, 0];
    // SAFETY: the payload is never read.
    let walked = unsafe { walk_outside(slot.as_mut_ptr().cast(), &words) };
    assert_eq!(walked, Ok(CopyTally::default()));
}

// 13.10.26q AC2: a tag past the last variant names no payload and reads no
// word beyond the layout.
#[test]
fn a_tag_past_the_last_variant_copies_nothing() {
    let words = three_variants([NO_NODE, NO_NODE, 10]);
    let mut slot = [3u64, 0, 0];
    // SAFETY: the payload is never read.
    let walked = unsafe { walk_outside(slot.as_mut_ptr().cast(), &words) };
    assert_eq!(walked, Ok(CopyTally::default()));
}

// 13.10.26q AC2: the third variant's string is copied through its own node.
#[test]
fn the_third_variant_copies_through_its_own_node() {
    let words = three_variants([NO_NODE, NO_NODE, 10]);
    let mut fixture = Fixture::new();
    let (text, len) = fixture.string_in_region("third");
    let mut slot = [2u64, text as u64, len as u64];
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &words);
    assert_eq!(read_string(slot[1..].as_ptr().cast()), "third");
    assert_eq!(tally.copied, 1);
}

// 13.10.26q AC2: a boxed variant is copied up to its own payload, read from
// its own words — words 4 and 5 are padding where an off-by-one or -two read
// would land — and the rest of the copy is zeroed, not left as the parent's
// fill.
#[test]
fn a_boxed_variant_copies_its_own_size_and_zeroes_the_rest() {
    // Word 1: a 24-byte box of word 6, `A | B(12 bytes)` with the payload at 8.
    let words = layout(&[
        KIND_BOX,
        24,
        6,
        KIND_STRING,
        0,
        KIND_TAGGED,
        8,
        2,
        0,
        NO_NODE,
        12,
        NO_NODE,
    ]);
    let mut fixture = Fixture::new();
    let block = fixture.in_region(24);
    // SAFETY: a fresh 24-byte block.
    unsafe {
        block.cast::<u32>().write_unaligned(1);
        block.add(8).write_bytes(0x42, 12);
        block.add(20).write_bytes(0x99, 4);
    }
    let mut slot = [block as u64];
    let tally = fixture.copy_then_release(slot.as_mut_ptr().cast(), &words);
    assert_eq!(tally.bytes, 20, "the tag, the padding and B's 12 bytes");
    // SAFETY: the copy is 24 bytes in the parent.
    let copy = unsafe { core::slice::from_raw_parts(slot[0] as *const u8, 24) };
    assert_eq!(copy[..4], 1u32.to_ne_bytes());
    assert!(copy[8..20].iter().all(|&byte| byte == 0x42));
    assert!(
        copy[20..].iter().all(|&byte| byte == 0),
        "the tail is zeroed"
    );
}

// 13.10.26q AC2: what the copy leaves in the region reads as poison after the
// release, so every intact read-back in these tests is evidence of a copy.
#[test]
fn a_value_left_in_the_region_reads_as_poison() {
    let mut fixture = Fixture::new();
    let (text, len) = fixture.string_in_region("left");
    fixture.regions.detach().unwrap();
    fixture.release_with_poison();
    // SAFETY: the chunk stays mapped by the fixture's pool.
    let left = unsafe { core::slice::from_raw_parts(text, len) };
    assert!(left.iter().all(|&byte| byte == POISON_BYTE));
}

// 13.10.26q AC2: the C face copies a region string into the parent between
// detach and release, and `ALBITE_REGION_POISON` is read into the flag the
// release consults.
#[test]
fn the_export_copies_a_region_string_into_the_parent() {
    let _guard = ModeGuard::take();
    read_region_poison(Some("1"));
    assert!(region_poison_enabled());
    apply_regions_setting(parse_arena_setting(None));
    __albite_region_enter();
    let words = layout(&[KIND_STRING]);
    // SAFETY: a region string, copied out before its release; the copy is a
    // scoped-mode parent block with no region open, so `free(3)` owns it.
    unsafe {
        let text = __albite_alloc(6, CLASS_STRING).cast::<u8>();
        core::ptr::copy_nonoverlapping(b"abcdef".as_ptr(), text, 6);
        let mut slot = string_slot(text, 6);
        __albite_region_detach();
        assert!(detached_region_contains(text));
        __albite_region_copy_out(slot.as_mut_ptr().cast(), words.as_ptr());
        let copy = slot[0] as *mut u8;
        assert_ne!(copy, text, "the string moved");
        assert!(!detached_region_contains(copy), "into the parent");
        __albite_region_release();
        assert_eq!(read_string(slot.as_ptr().cast()), "abcdef");
        libc::free(copy.cast());
    }
    read_region_poison(None);
    assert!(!region_poison_enabled());
}
