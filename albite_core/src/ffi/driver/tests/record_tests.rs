//! The machine-format record bracket (ADR 1.10.26i D2/D3, AC 2).
//!
//! A child of `ffi::driver::test` (declared there by `#[path]`), so the
//! counter and the injected-writer half are reachable without widening them.

use std::collections::HashMap;

use super::*;
use crate::ffi::console_capture::{self, test_exclusive::exclusive_sink, CapturedOutput};

fn captured(stdout: &[u8], stderr: &[u8]) -> CapturedOutput {
    CapturedOutput {
        stdout: stdout.to_vec(),
        stderr: stderr.to_vec(),
    }
}

/// 1.10.26i AC 2: every field of the frame comes from its own argument.
#[test]
fn frame_from_reads_flag_count_and_capture() {
    let frame = frame_from(&captured(b"out\n", b"err\n"), true, 3, 12, "test_x");
    assert_eq!(
        frame,
        Frame {
            name: "test_x".to_string(),
            failed: true,
            assertions: 3,
            duration_ms: 12,
            stdout: "out\n".to_string(),
            stderr: "err\n".to_string(),
        }
    );
    assert!(!frame_from(&captured(b"", b""), false, 0, 0, "t").failed);
}

/// 1.10.26i AC 2: D3's header, then stdout, then stderr, lengths in bytes.
#[test]
fn frame_bytes_writes_header_then_payloads() {
    let frame = frame_from(
        &captured("é\x1e\n".as_bytes(), b"e\t"),
        false,
        1,
        7,
        "test_a",
    );
    let expected = "\x1etgtest\t1\ttest_a\t0\t1\t7\t4\t2\né\x1e\ne\t";
    assert_eq!(String::from_utf8(frame_bytes(&frame)).unwrap(), expected);
    let failed = frame_from(&captured(b"", b""), true, 0, 0, "test_b");
    assert_eq!(
        frame_bytes(&failed),
        b"\x1etgtest\t1\ttest_b\t1\t0\t0\t0\t0\n".to_vec()
    );
}

/// 1.10.26i AC 2, under D3's fallback: a NUL and invalid UTF-8 arrive as
/// U+FFFD, so the frames file is readable through `read_file`, and the length
/// counts the replacement's three bytes.
#[test]
fn frame_bytes_carries_nul_and_invalid_utf8() {
    let frame = frame_from(&captured(b"a\0b\xff\xfe", b""), false, 0, 0, "test_n");
    assert_eq!(frame.stdout, "a\u{FFFD}b\u{FFFD}\u{FFFD}");
    let text = String::from_utf8(frame_bytes(&frame)).expect("the frame is valid UTF-8");
    assert!(!text.contains('\0'));
    assert!(
        text.starts_with("\x1etgtest\t1\ttest_n\t0\t0\t0\t11\t0\n"),
        "{text:?}"
    );
}

/// 1.10.26i AC 2: `tg_test_record_end`'s effectful half writes exactly the
/// frame the bracket built — the body's output, the flag and the count — and
/// takes the sink so later writes reach the real streams.
#[test]
fn record_end_writes_the_built_frame() {
    let _guard = exclusive_sink();
    let name = b"test_rec";
    tg_test_record_begin(name.as_ptr().cast(), name.len() as u64);
    assert!(console_capture::is_active(), "begin installs a sink");
    crate::ffi::tg_println(b"hi".as_ptr().cast(), 2);
    tg_assert_eq_nat(1, 2);
    tg_assert_eq_bool(1, 1);
    let mut written = Vec::new();
    write_record_end(&mut written);
    assert!(!console_capture::is_active(), "end takes the sink");
    let text = String::from_utf8(written).unwrap();
    let diff = mismatch_text("assert_eq_nat", "1", "2");
    assert!(
        text.starts_with("\x1etgtest\t1\ttest_rec\t1\t2\t"),
        "{text:?}"
    );
    // `contains`, not equality: this module's sibling tests fail assertions
    // without the sink lock, and a concurrent one appends its diff here.
    assert!(text.contains("\t3\t"), "stdout_len is 3: {text:?}");
    assert!(text.contains(&format!("\nhi\n{diff}")), "{text:?}");
}

/// 1.10.26i AC 2: the FFI entry itself closes the bracket — it takes the sink,
/// so a later write reaches the process stream rather than a dead frame. (Its
/// frame goes to this test binary's real stdout, which is harmless.)
#[test]
fn record_end_entry_point_takes_the_sink() {
    let _guard = exclusive_sink();
    let name = b"test_entry";
    tg_test_record_begin(name.as_ptr().cast(), name.len() as u64);
    assert!(console_capture::is_active());
    tg_test_record_end();
    assert!(
        !console_capture::is_active(),
        "the entry point closed the bracket"
    );
}

/// 1.10.26i D2: a sink a previous test left installed is discarded, not a
/// reason for `install` to refuse — so the new test's output is its own.
#[test]
fn record_begin_discards_a_stale_sink() {
    let _guard = exclusive_sink();
    console_capture::install().expect("no sink yet");
    crate::ffi::tg_println(b"stale".as_ptr().cast(), 5);
    let name = b"test_fresh";
    tg_test_record_begin(name.as_ptr().cast(), name.len() as u64);
    assert!(console_capture::is_active());
    assert_eq!(ASSERTIONS.with(Cell::get), 0, "begin resets the count");
    let frame = end_frame();
    assert_eq!(frame.stdout, "", "the stale sink's bytes are gone");
    assert_eq!(frame.name, "test_fresh");
    assert!(!frame.failed);
}

/// 1.10.26i AC 2 / D4: the core's counter is not the evaluator's. An
/// assertion outside a bracket bumps the core counter and leaves an
/// evaluator env's own count where it was.
#[test]
fn an_assertion_outside_a_bracket_leaves_the_evaluator_count_alone() {
    let env = crate::eval::EvalEnv::new(HashMap::new());
    let before = ASSERTIONS.with(Cell::get);
    tg_assert_eq_nat(4, 4);
    tg_assert_eq_int(-2, -2);
    assert_eq!(ASSERTIONS.with(Cell::get), before + 2);
    assert_eq!(env.assertions_executed(), 0);
}
