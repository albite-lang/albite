//! The eighth intrinsic, `tg_bytes_from_string_grown`, on the evaluator
//! (ADR 6.10.26n AC 2): the prefix is kept, the tail is unwritten (9.10.26n
//! D2), the ceiling traps before allocating, and the handle is live.

use super::unwritten::ByteStop;
use super::{apply, grown_exceeds_ceiling, ByteBuffers, ByteIntrinsic, BYTES_MAX_LEN};
use crate::eval::env::IntTrapKind;
use crate::eval::{nat_to_term, term_to_nat};
use crate::terms::Term;

/// Run the adoption through `apply`, the evaluator's one arm.
fn grow(table: &mut ByteBuffers, text: &str, extra: u64) -> Result<u64, IntTrapKind> {
    let values = [
        Term::StringLit(text.to_string()),
        nat_to_term(extra as usize),
    ];
    match apply(table, ByteIntrinsic::FromStringGrown, &values).expect("claimed") {
        Ok(handle) => Ok(term_to_nat(&handle).expect("a handle is a Nat") as u64),
        Err(ByteStop::Trap(kind)) => Err(kind),
        Err(other) => panic!("a grow reads no byte: {other:?}"),
    }
}

// 6.10.26n AC 2, 9.10.26n D2: the string's bytes first, then `extra`
// unwritten bytes, in a live buffer: the prefix reads back, the tail stops.
#[test]
fn the_grown_buffer_keeps_the_prefix_and_leaves_the_tail_unwritten() {
    let mut table = ByteBuffers::default();
    let handle = grow(&mut table, "aé", 3).expect("in range");
    assert_eq!(table.len(handle), Ok(6));
    let prefix: Vec<u64> = (0..3)
        .map(|i| table.read_written(handle, i).expect("written"))
        .collect();
    assert_eq!(prefix, vec![0x61, 0xC3, 0xA9]);
    for offset in 3..6 {
        assert_eq!(
            table.read_written(handle, offset),
            Err(ByteStop::UnwrittenRead { offset })
        );
    }
    assert_eq!(table.get(handle, 6), Err(IntTrapKind::ByteIndexOutOfRange));
}

// 6.10.26n AC 2: the handle is live — it can be written, blitted and converted.
#[test]
fn the_grown_handle_is_live_and_converts_once() {
    let mut table = ByteBuffers::default();
    let handle = grow(&mut table, "ab", 2).expect("in range");
    assert_eq!(table.blit_string(handle, 2, "cd"), Ok(handle));
    assert_eq!(table.into_string(handle), Ok("abcd".to_string()));
    assert_eq!(table.len(handle), Err(IntTrapKind::ByteBufferConsumed));
}

// 6.10.26n D1's edges: an empty left, an empty right, and both.
#[test]
fn an_empty_string_or_no_extra_still_makes_a_buffer() {
    let mut table = ByteBuffers::default();
    let empty = grow(&mut table, "", 0).expect("in range");
    assert_eq!(table.len(empty), Ok(0));
    let unchanged = grow(&mut table, "xyz", 0).expect("in range");
    assert_eq!(table.into_string(unchanged), Ok("xyz".to_string()));
    let only_tail = grow(&mut table, "", 2).expect("in range");
    assert_eq!(table.len(only_tail), Ok(2));
    assert_ne!(empty, unchanged, "each adoption issues its own handle");
}

// 6.10.26n AC 2: above the ceiling traps `ByteBufferTooLarge`, before
// allocating, and issues no handle.
#[test]
fn growing_past_the_ceiling_traps_and_issues_nothing() {
    let mut table = ByteBuffers::default();
    assert_eq!(
        grow(&mut table, "ab", BYTES_MAX_LEN),
        Err(IntTrapKind::ByteBufferTooLarge)
    );
    assert_eq!(
        grow(&mut table, "", u64::MAX),
        Err(IntTrapKind::ByteBufferTooLarge)
    );
    let next = grow(&mut table, "", 0).expect("in range");
    assert_eq!(next, 1, "a refusal issued no handle");
}

// 6.10.26n AC 2: the bound itself, without allocating 4 GiB. A string of
// `BYTES_MAX_LEN + 1` with `extra = 0` refuses — the case a wrapping
// subtraction `BYTES_MAX_LEN - len` would pass.
#[test]
fn the_grown_bound_is_inclusive_and_cannot_wrap() {
    assert!(!grown_exceeds_ceiling(0, 0));
    assert!(!grown_exceeds_ceiling(0, BYTES_MAX_LEN));
    assert!(!grown_exceeds_ceiling(BYTES_MAX_LEN, 0));
    assert!(!grown_exceeds_ceiling(BYTES_MAX_LEN - 5, 5));
    assert!(grown_exceeds_ceiling(BYTES_MAX_LEN - 5, 6));
    assert!(grown_exceeds_ceiling(0, BYTES_MAX_LEN + 1));
    assert!(grown_exceeds_ceiling(BYTES_MAX_LEN + 1, 0));
    assert!(grown_exceeds_ceiling(u64::MAX, 0));
    assert!(grown_exceeds_ceiling(1, u64::MAX));
}

// A shape the `.tg` declaration cannot produce is not claimed.
#[test]
fn a_non_string_first_argument_is_not_claimed() {
    let mut table = ByteBuffers::default();
    let values = [nat_to_term(1), nat_to_term(1)];
    assert_eq!(
        apply(&mut table, ByteIntrinsic::FromStringGrown, &values),
        None
    );
    assert_eq!(
        apply(
            &mut table,
            ByteIntrinsic::FromStringGrown,
            &[Term::StringLit("a".into())]
        ),
        None
    );
}
