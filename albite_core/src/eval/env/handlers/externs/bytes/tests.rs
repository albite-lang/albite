//! The checked byte buffer on the evaluator (ADR 3.10.26n AC 2, AC 3).
//!
//! Most of these drive [`ByteBuffers`] directly, a fresh table per test, so a
//! rule is asserted without the thread-local. The dispatch tests go through
//! `step_bytes_extern`, so the trap is shown to reach the env; the registry's
//! `every_registry_entry_is_claimed_by_dispatch` drives the eight through
//! `step_extern_call_env` with [`registry_sample_arguments`], and
//! `registry_tests/bytes.rs` pins their registry entries.

use super::unwritten::ByteStop;
use super::{apply, exceeds_ceiling, step_bytes_extern, ByteBuffers, ByteIntrinsic, BYTES_MAX_LEN};
use crate::eval::env::{EvalEnv, EvalStopped, IntTrapKind};
use crate::eval::{nat_to_term, term_to_nat, StepResult};
use crate::terms::Term;

const TWO_TO_63: u64 = 1 << 63;

fn table_with(len: u64) -> (ByteBuffers, u64) {
    let mut table = ByteBuffers::default();
    let handle = table.new_buffer(len).expect("in range");
    (table, handle)
}

fn nat(n: u64) -> Term {
    nat_to_term(n as usize)
}

/// Plausible arguments for each `Bytes` registry entry, each over a **live**
/// buffer from a real `tg_bytes_new`, so "claimed" means the arm works.
pub(in crate::eval::env::handlers::externs) fn registry_sample_arguments(name: &str) -> Vec<Term> {
    let live = || {
        let env = EvalEnv::new(std::collections::HashMap::new());
        match step_bytes_extern("tg_bytes_new", &[nat(4)], &env) {
            Some(StepResult::Stepped(handle)) => handle,
            other => panic!("tg_bytes_new did not step: {other:?}"),
        }
    };
    match name {
        "tg_bytes_new" => vec![nat(4)],
        "tg_bytes_len" | "tg_bytes_into_string" | "tg_bytes_into_string_unchecked" => {
            vec![live()]
        }
        "tg_bytes_get" => vec![live(), nat(0)],
        "tg_bytes_set" => vec![live(), nat(0), nat(65)],
        "tg_bytes_blit_string" => vec![live(), nat(0), Term::StringLit("ab".to_string())],
        "tg_bytes_from_string_grown" => vec![Term::StringLit("ab".to_string()), nat(2)],
        "tg_bytes_new_uninit" => vec![nat(4)],
        other => panic!("no sample arguments for registry entry {other} — add them here"),
    }
}

// ── In range (AC 3, last paragraph) ─────────────────────────────────────────

#[test]
fn a_new_buffer_is_zero_filled_and_has_its_length() {
    let (table, handle) = table_with(3);
    assert_eq!(table.len(handle), Ok(3));
    for index in 0..3 {
        assert_eq!(table.get(handle, index), Ok(0));
    }
}

#[test]
fn every_byte_value_round_trips_through_set_and_get() {
    let (mut table, handle) = table_with(256);
    for value in 0..=255 {
        assert_eq!(table.set(handle, value, value), Ok(handle));
    }
    for value in 0..=255 {
        assert_eq!(table.get(handle, value), Ok(value));
    }
}

#[test]
fn blits_land_where_asked_and_convert_to_the_bytes_written() {
    let (mut table, handle) = table_with(7);
    assert_eq!(table.blit_string(handle, 0, "ab"), Ok(handle));
    assert_eq!(table.blit_string(handle, 2, "é€"), Ok(handle));
    assert_eq!(table.into_string(handle), Ok("abé€".to_string()));
}

#[test]
fn a_blit_that_exactly_fills_the_tail_and_an_empty_blit_at_the_end_are_in_range() {
    let (mut table, handle) = table_with(3);
    assert_eq!(table.blit_string(handle, 1, "yz"), Ok(handle));
    assert_eq!(table.blit_string(handle, 3, ""), Ok(handle));
    assert_eq!(table.into_string(handle), Ok("\0yz".to_string()));
}

#[test]
fn an_empty_buffer_is_legal_and_handles_are_distinct() {
    let mut table = ByteBuffers::default();
    let first = table.new_buffer(0).expect("empty is fine");
    let second = table.new_buffer(1).expect("in range");
    assert_ne!(first, second);
    assert_ne!(first, 0, "0 never names a buffer");
    assert_eq!(table.into_string(first), Ok(String::new()));
}

// ── Refusals (AC 3) ────────────────────────────────────────────────────────

#[test]
fn an_index_equal_to_the_length_and_an_index_of_two_to_the_63_refuse() {
    let (mut table, handle) = table_with(4);
    for index in [4, TWO_TO_63, u64::MAX] {
        assert_eq!(
            table.get(handle, index),
            Err(IntTrapKind::ByteIndexOutOfRange)
        );
        assert_eq!(
            table.set(handle, index, 1),
            Err(IntTrapKind::ByteIndexOutOfRange)
        );
    }
    assert_eq!(table.get(handle, 3), Ok(0), "the last index is in range");
}

#[test]
fn a_byte_of_256_refuses_and_255_does_not() {
    let (mut table, handle) = table_with(1);
    assert_eq!(
        table.set(handle, 0, 256),
        Err(IntTrapKind::ByteValueOutOfRange)
    );
    assert_eq!(table.set(handle, 0, 255), Ok(handle));
}

#[test]
fn a_blit_at_u64_max_and_a_blit_one_byte_too_long_refuse() {
    let (mut table, handle) = table_with(4);
    let refused = Err(IntTrapKind::ByteIndexOutOfRange);
    assert_eq!(table.blit_string(handle, u64::MAX, "a"), refused);
    assert_eq!(table.blit_string(handle, u64::MAX, ""), refused);
    assert_eq!(table.blit_string(handle, 5, ""), refused);
    assert_eq!(table.blit_string(handle, 2, "abc"), refused);
    assert_eq!(table.blit_string(handle, 0, "abcde"), refused);
}

#[test]
fn new_above_the_ceiling_refuses_without_allocating() {
    let mut table = ByteBuffers::default();
    for len in [BYTES_MAX_LEN + 1, u64::MAX] {
        assert_eq!(table.new_buffer(len), Err(IntTrapKind::ByteBufferTooLarge));
    }
    assert!(table.buffers.is_empty(), "nothing was allocated");
    assert_eq!(table.last_handle, 0, "no handle was issued");
}

/// The ceiling is inclusive: `BYTES_MAX_LEN` itself is a legal length, one
/// more is not. Asserted on the predicate, since a 4 GiB buffer is no fixture.
#[test]
fn the_ceiling_is_a_legal_length_and_one_past_it_is_not() {
    assert!(!exceeds_ceiling(BYTES_MAX_LEN));
    assert!(exceeds_ceiling(BYTES_MAX_LEN + 1));
    assert!(!exceeds_ceiling(0));
}

#[test]
fn every_operation_on_a_consumed_buffer_refuses() {
    let (mut table, handle) = table_with(2);
    assert_eq!(table.into_string(handle), Ok("\0\0".to_string()));
    let consumed = IntTrapKind::ByteBufferConsumed;
    assert_eq!(table.len(handle), Err(consumed));
    assert_eq!(table.get(handle, 0), Err(consumed));
    assert_eq!(table.set(handle, 0, 1), Err(consumed));
    assert_eq!(table.blit_string(handle, 0, "a"), Err(consumed));
    assert_eq!(table.into_string(handle), Err(consumed));
}

#[test]
fn liveness_is_checked_before_the_index_so_both_sides_print_one_line() {
    let (mut table, handle) = table_with(1);
    table.into_string(handle).expect("valid");
    assert_eq!(table.get(handle, 99), Err(IntTrapKind::ByteBufferConsumed));
    assert_eq!(
        table.set(handle, 99, 999),
        Err(IntTrapKind::ByteBufferConsumed)
    );
    let (mut table, handle) = table_with(1);
    assert_eq!(
        table.set(handle, 9, 999),
        Err(IntTrapKind::ByteIndexOutOfRange)
    );
}

#[test]
fn a_conversion_of_invalid_utf8_refuses_and_leaves_the_buffer_live() {
    let (mut table, handle) = table_with(2);
    table.set(handle, 0, 0xC3).expect("in range");
    assert_eq!(
        table.into_string(handle),
        Err(IntTrapKind::ByteBufferNotUtf8)
    );
    assert_eq!(
        table.len(handle),
        Ok(2),
        "a refused conversion consumes nothing"
    );
}

#[test]
fn a_forged_handle_refuses_on_the_evaluator() {
    let (table, handle) = table_with(1);
    for forged in [0, handle + 1, u64::MAX] {
        assert_eq!(table.len(forged), Err(IntTrapKind::ByteBufferConsumed));
    }
}

// ── Marshalling and dispatch ───────────────────────────────────────────────

#[test]
fn apply_marshals_each_arity_and_declines_a_shape_it_cannot_take() {
    let mut table = ByteBuffers::default();
    let handle = match apply(&mut table, ByteIntrinsic::New, &[nat(2)]) {
        Some(Ok(term)) => term,
        other => panic!("new did not step: {other:?}"),
    };
    let set = apply(
        &mut table,
        ByteIntrinsic::Set,
        &[handle.clone(), nat(1), nat(66)],
    );
    assert_eq!(set, Some(Ok(handle.clone())));
    let get = apply(&mut table, ByteIntrinsic::Get, &[handle.clone(), nat(1)]);
    assert_eq!(
        get.and_then(Result::ok).and_then(|t| term_to_nat(&t)),
        Some(66)
    );
    let text = Term::StringLit("A".to_string());
    let blit = apply(
        &mut table,
        ByteIntrinsic::BlitString,
        &[handle.clone(), nat(0), text],
    );
    assert_eq!(blit, Some(Ok(handle.clone())));
    let len = apply(
        &mut table,
        ByteIntrinsic::Len,
        std::slice::from_ref(&handle),
    );
    assert_eq!(
        len.and_then(Result::ok).and_then(|t| term_to_nat(&t)),
        Some(2)
    );
    let string = apply(&mut table, ByteIntrinsic::IntoString, &[handle]);
    assert_eq!(string, Some(Ok(Term::StringLit("AB".to_string()))));

    assert_eq!(
        apply(&mut table, ByteIntrinsic::New, &[]),
        None,
        "wrong arity"
    );
    let not_a_nat = Term::StringLit("x".to_string());
    assert_eq!(apply(&mut table, ByteIntrinsic::Len, &[not_a_nat]), None);
}

#[test]
fn a_refusal_through_dispatch_records_its_trap_and_is_stuck() {
    let env = EvalEnv::new(std::collections::HashMap::new());
    let stepped = step_bytes_extern("__c_tg_bytes_new", &[nat(u64::MAX)], &env);
    assert!(matches!(stepped, Some(StepResult::Stuck)));
    let expected = EvalStopped::IntTrap {
        kind: IntTrapKind::ByteBufferTooLarge,
    };
    assert_eq!(env.recorded_stop(), Some(expected));
    assert!(step_bytes_extern("tg_bytes_nope", &[], &env).is_none());
}

#[test]
fn symbols_round_trip_and_the_c_prefix_is_accepted() {
    for op in ByteIntrinsic::ALL {
        assert_eq!(ByteIntrinsic::from_symbol(op.symbol()), Some(op));
        let prefixed = format!("__c_{}", op.symbol());
        assert_eq!(ByteIntrinsic::from_symbol(&prefixed), Some(op));
    }
    assert_eq!(ByteIntrinsic::from_symbol("tg_bytes_nope"), None);
}

// ── The reserved, unchecked conversion (ADR 4.10.26n AC 5) ─────────────────

/// Run the unchecked conversion through `apply`, the evaluator's one arm.
fn convert_unchecked(table: &mut ByteBuffers, handle: u64) -> Result<Term, IntTrapKind> {
    apply(table, ByteIntrinsic::IntoStringUnchecked, &[nat(handle)])
        .expect("claimed")
        .map_err(|stop| match stop {
            ByteStop::Trap(kind) => kind,
            other => panic!("a filled buffer has no unwritten byte: {other:?}"),
        })
}

// 4.10.26n AC 5: on the evaluator the unchecked conversion converts valid bytes.
#[test]
fn the_unchecked_conversion_converts_valid_bytes_on_the_evaluator() {
    let (mut table, handle) = table_with(3);
    table.blit_string(handle, 0, "aé").expect("fits");
    assert_eq!(
        convert_unchecked(&mut table, handle),
        Ok(Term::StringLit("aé".to_string()))
    );
    assert_eq!(table.len(handle), Err(IntTrapKind::ByteBufferConsumed));
}

// 4.10.26n AC 5: an evaluator String must be valid UTF-8, so invalid bytes
// still trap here, and a consumed handle traps as `into_string`'s does.
#[test]
fn the_unchecked_conversion_still_traps_on_invalid_bytes_and_a_consumed_handle() {
    let (mut table, handle) = table_with(1);
    table.set(handle, 0, 0xC3).expect("in range");
    assert_eq!(
        convert_unchecked(&mut table, handle),
        Err(IntTrapKind::ByteBufferNotUtf8)
    );
    table.set(handle, 0, 0x41).expect("still live");
    assert_eq!(
        convert_unchecked(&mut table, handle),
        Ok(Term::StringLit("A".to_string()))
    );
    assert_eq!(
        convert_unchecked(&mut table, handle),
        Err(IntTrapKind::ByteBufferConsumed)
    );
}

// 4.10.26n D5, 6.10.26n D2, 9.10.26n D2: exactly the seventh, eighth and
// ninth intrinsics are reserved to the native core.
#[test]
fn only_the_unchecked_conversion_the_adoption_and_the_unfilled_new_are_reserved() {
    let reserved: Vec<ByteIntrinsic> = ByteIntrinsic::ALL
        .into_iter()
        .filter(|op| op.is_reserved())
        .collect();
    assert_eq!(
        reserved,
        vec![
            ByteIntrinsic::IntoStringUnchecked,
            ByteIntrinsic::FromStringGrown,
            ByteIntrinsic::NewUninit
        ]
    );
    assert_eq!(
        ByteIntrinsic::from_symbol("__c_tg_bytes_into_string_unchecked"),
        Some(ByteIntrinsic::IntoStringUnchecked)
    );
}
