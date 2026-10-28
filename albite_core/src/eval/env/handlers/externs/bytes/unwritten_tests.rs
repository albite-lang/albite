//! The ninth intrinsic and the evaluator's unwritten-byte stop (ADR 9.10.26n
//! D2, D3; AC 3's dynamic drill): a buffer `tg_bytes_new_uninit` opens is
//! unwritten until a store reaches each byte, and a read of one stops.

use super::unwritten::{ByteStop, UnwrittenBytes};
use super::{apply, step_bytes_extern, ByteBuffers, ByteIntrinsic};
use crate::eval::env::{EvalEnv, EvalStopped, IntTrapKind};
use crate::eval::{nat_to_term, term_to_nat, StepResult};
use crate::terms::Term;

fn nat(n: u64) -> Term {
    nat_to_term(n as usize)
}

fn step(env: &EvalEnv, name: &str, values: &[Term]) -> Term {
    match step_bytes_extern(name, values, env) {
        Some(StepResult::Stepped(term)) => term,
        other => panic!("{name} did not step: {other:?}"),
    }
}

// 9.10.26n AC 3 (dynamic): one partial blit into an unfilled buffer, then a
// conversion, stops evaluation with `UnwrittenByteRead` at the first hole.
#[test]
fn a_partial_blit_then_a_conversion_stops_on_the_first_unwritten_byte() {
    let env = EvalEnv::new(std::collections::HashMap::new());
    let handle = step(&env, "__c_tg_bytes_new_uninit", &[nat(7)]);
    let text = Term::StringLit("fox".to_string());
    step(
        &env,
        "tg_bytes_blit_string",
        &[handle.clone(), nat(0), text],
    );
    let converted = step_bytes_extern("tg_bytes_into_string_unchecked", &[handle], &env);
    assert!(matches!(converted, Some(StepResult::Stuck)));
    assert_eq!(
        env.recorded_stop(),
        Some(EvalStopped::UnwrittenByteRead { offset: 3 })
    );
    assert_eq!(
        EvalStopped::UnwrittenByteRead { offset: 3 }.to_string(),
        "byte buffer reads an unwritten byte at offset 3"
    );
}

// 9.10.26n D3: the concat's own shape, two copies that tile the buffer,
// converts — the backstop does not refuse what the rule admits.
#[test]
fn copies_that_tile_the_buffer_convert() {
    let mut table = ByteBuffers::default();
    let handle = table.new_uninit_buffer(7).expect("in range");
    assert_eq!(table.len(handle), Ok(7));
    table.blit_string(handle, 0, "fox").expect("fits");
    table.blit_string(handle, 3, "dog!").expect("fits");
    assert_eq!(table.convert_written(handle), Ok("foxdog!".to_string()));
    assert_eq!(
        table.convert_written(handle),
        Err(ByteStop::Trap(IntTrapKind::ByteBufferConsumed)),
        "liveness is checked before the unwritten bytes"
    );
}

// 9.10.26n D3: a `get` of an unwritten byte stops; after a `set` it reads.
#[test]
fn a_get_stops_on_an_unwritten_byte_until_a_set_writes_it() {
    let mut table = ByteBuffers::default();
    let handle = table.new_uninit_buffer(2).expect("in range");
    assert_eq!(
        table.read_written(handle, 1),
        Err(ByteStop::UnwrittenRead { offset: 1 })
    );
    table.set(handle, 1, 66).expect("in range");
    assert_eq!(table.read_written(handle, 1), Ok(66));
    assert_eq!(
        table.read_written(handle, 2),
        Err(ByteStop::Trap(IntTrapKind::ByteIndexOutOfRange)),
        "the index is checked before the unwritten bytes"
    );
    let get = apply(&mut table, ByteIntrinsic::Get, &[nat(handle), nat(0)]);
    assert_eq!(get, Some(Err(ByteStop::UnwrittenRead { offset: 0 })));
}

// 9.10.26n D2: `tg_bytes_new` stays filled, and `new_uninit` keeps its
// ceiling, issuing no handle above it.
#[test]
fn the_filled_new_has_no_unwritten_bytes_and_the_ceiling_still_traps() {
    let mut table = ByteBuffers::default();
    let filled = table.new_buffer(2).expect("in range");
    assert_eq!(table.convert_written(filled), Ok("\0\0".to_string()));
    let too_large = apply(&mut table, ByteIntrinsic::NewUninit, &[nat(u64::MAX)]);
    assert_eq!(
        too_large,
        Some(Err(ByteStop::Trap(IntTrapKind::ByteBufferTooLarge)))
    );
    let empty = apply(&mut table, ByteIntrinsic::NewUninit, &[nat(0)]).expect("claimed");
    let empty = term_to_nat(&empty.expect("in range")).expect("a handle") as u64;
    assert_eq!(empty, 2, "the refusal issued no handle");
    assert_eq!(table.convert_written(empty), Ok(String::new()));
}

// The marks themselves: an opened range, a partial write, and the entry
// dropped once the last byte is written.
#[test]
fn marks_open_clear_and_close() {
    let mut marks = UnwrittenBytes::default();
    marks.open(1, 4, 2);
    assert_eq!(marks.first_in(1, 0, 4), Some(2));
    assert_eq!(marks.first_in(1, 0, 2), None);
    marks.write(1, 2, 3);
    assert_eq!(marks.first_in(1, 0, 4), Some(3));
    marks.write(1, 3, 4);
    assert_eq!(marks.first_in(1, 0, 4), None);
    marks.open(2, 3, 3);
    assert_eq!(
        marks.first_in(2, 0, 3),
        None,
        "an empty range opens nothing"
    );
    marks.open(3, 1, 0);
    marks.close(3);
    assert_eq!(marks.first_in(3, 0, 1), None);
    assert_eq!(
        ByteStop::UnwrittenRead { offset: 5 }.into_stop(),
        EvalStopped::UnwrittenByteRead { offset: 5 }
    );
    assert_eq!(
        ByteStop::from(IntTrapKind::ByteBufferTooLarge).into_stop(),
        EvalStopped::IntTrap {
            kind: IntTrapKind::ByteBufferTooLarge
        }
    );
}
