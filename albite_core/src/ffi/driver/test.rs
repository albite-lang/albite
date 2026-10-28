//! Test runner FFI functions — assertion checking and failure tracking.
//!
//! These functions support `tungsten test`:
//! - `tg_test_begin`: reset failure state for a new test
//! - `tg_assert_eq_nat`: compare two Nat values
//! - `tg_assert_eq_string`: compare two String values (ptr+len)
//! - `tg_assert_eq_bool`: compare two Bool values (0/1)
//! - `tg_test_check_failure`: query whether the current test failed
//! - `tg_test_record_begin` / `tg_test_record_end`: bracket a test in a
//!   machine format, framing its captured output (ADR 1.10.26i D2)

use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::time::Instant;

use super::console_capture::{self, CapturedOutput};

thread_local! {
    static TEST_FAILED: Cell<bool> = const { Cell::new(false) };
    static TEST_NAME: RefCell<String> = const { RefCell::new(String::new()) };
    /// Assertions executed since the last `tg_test_record_begin` (D4). Every
    /// `tg_assert_*` bumps it; only the record bracket resets or reads it, so
    /// the bootstrap evaluator, which counts its own way, is unaffected.
    static ASSERTIONS: Cell<u64> = const { Cell::new(0) };
    static RECORD_STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn count_assertion() {
    ASSERTIONS.with(|count| count.set(count.get() + 1));
}

/// Reset failure flag for a new test. Stores the test name for diagnostics.
///
/// # Safety
/// - `name` must be a valid pointer to `len` bytes of UTF-8
#[no_mangle]
pub extern "C" fn tg_test_begin(name: *const c_char, len: u64) {
    TEST_FAILED.with(|f| f.set(false));
    if !name.is_null() {
        let slice = unsafe { std::slice::from_raw_parts(name.cast::<u8>(), len as usize) };
        let name_str = std::str::from_utf8(slice).unwrap_or("<invalid utf8>");
        TEST_NAME.with(|n| *n.borrow_mut() = name_str.to_string());
    }
}

/// The three-line diff an assertion mismatch prints: the assertion's name,
/// then each side as already rendered. Pure, so the wording is assertable.
fn mismatch_text(assertion: &str, left: &str, right: &str) -> String {
    format!("  assertion failed: {assertion}\n    left:  {left}\n    right: {right}\n")
}

/// Write a mismatch diff to stderr — or, when a console capture sink is
/// installed, into its stderr buffer (ADR 29.9.26i D8a). `tungsten test
/// --format json` installs one around each body so the diff reaches the
/// failing test's record instead of the process's stderr; with no sink the
/// bytes are exactly what the three `eprintln!`s used to write.
fn report_mismatch(assertion: &str, left: &str, right: &str) {
    super::console_capture::write_stderr(&[mismatch_text(assertion, left, right).as_bytes()]);
}

/// Compare two values that render themselves, failing the test and printing
/// the diff on a mismatch. The shared body of the numeric assertions.
fn assert_displayed_eq<T: Copy + PartialEq + std::fmt::Display>(
    assertion: &str,
    left: T,
    right: T,
) {
    count_assertion();
    if left != right {
        report_mismatch(assertion, &left.to_string(), &right.to_string());
        TEST_FAILED.with(|f| f.set(true));
    }
}

/// Compare two Nat values. Sets failure flag and prints diff on mismatch.
#[no_mangle]
pub extern "C" fn tg_assert_eq_nat(left: u64, right: u64) {
    assert_displayed_eq("assert_eq_nat", left, right);
}

/// Compare two Int values (ADR 14.9.26c). Sets failure flag and prints the
/// signed diff on mismatch.
#[no_mangle]
pub extern "C" fn tg_assert_eq_int(left: i64, right: i64) {
    assert_displayed_eq("assert_eq_int", left, right);
}

/// Compare two String values (ptr+len pairs). Sets failure flag on mismatch.
///
/// # Safety
/// - `left` must be a valid pointer to `left_len` bytes
/// - `right` must be a valid pointer to `right_len` bytes
#[no_mangle]
pub extern "C" fn tg_assert_eq_string(
    left: *const c_char,
    left_len: u64,
    right: *const c_char,
    right_len: u64,
) {
    count_assertion();
    let sl = if left.is_null() {
        ""
    } else {
        let slice = unsafe { std::slice::from_raw_parts(left.cast::<u8>(), left_len as usize) };
        std::str::from_utf8(slice).unwrap_or("<invalid utf8>")
    };

    let sr = if right.is_null() {
        ""
    } else {
        let slice = unsafe { std::slice::from_raw_parts(right.cast::<u8>(), right_len as usize) };
        std::str::from_utf8(slice).unwrap_or("<invalid utf8>")
    };

    if sl != sr {
        report_mismatch(
            "assert_eq_string",
            &format!("\"{sl}\""),
            &format!("\"{sr}\""),
        );
        TEST_FAILED.with(|f| f.set(true));
    }
}

/// Compare two Bool values (encoded as 0=false, 1=true).
/// Sets failure flag and prints diff on mismatch.
#[no_mangle]
pub extern "C" fn tg_assert_eq_bool(left: u64, right: u64) {
    count_assertion();
    if left != right {
        let dl = if left == 0 { "false" } else { "true" };
        let dr = if right == 0 { "false" } else { "true" };
        report_mismatch("assert_eq_bool", dl, dr);
        TEST_FAILED.with(|f| f.set(true));
    }
}

/// Check whether the current test has failed (1 = failed, 0 = passed).
#[no_mangle]
pub extern "C" fn tg_test_check_failure() -> u64 {
    TEST_FAILED.with(|f| u64::from(f.get()))
}

// ============================================================================
// Record bracket (ADR 1.10.26i D2/D3)
// ============================================================================

/// The frame format's version, the third header field.
pub const FRAME_VERSION: u32 = 1;

/// One test's result as the harness reports it to the driver.
///
/// The payloads are already lossily decoded, with NUL mapped to U+FFFD (D3's
/// fallback): the driver reads the frames file through `read_file`, which
/// cannot carry a NUL or invalid UTF-8, and `.tg` has no out-parameter to
/// reach `tg_read_file_bytes` with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub name: String,
    pub failed: bool,
    pub assertions: u64,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
}

/// Build a frame from what the bracket collected. Pure.
#[must_use]
pub fn frame_from(
    captured: &CapturedOutput,
    failed: bool,
    assertions: u64,
    duration_ms: u64,
    name: &str,
) -> Frame {
    Frame {
        name: name.to_string(),
        failed,
        assertions,
        duration_ms,
        stdout: frame_payload(&captured.stdout),
        stderr: frame_payload(&captured.stderr),
    }
}

fn frame_payload(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\0', "\u{FFFD}")
}

/// Serialise a frame: `\x1e` + a tab-separated header line, then exactly
/// `stdout_len` bytes of stdout and `stderr_len` bytes of stderr. Lengths are
/// in bytes, so a payload holding `\x1e` or a newline cannot desynchronise the
/// stream. Pure.
#[must_use]
pub fn frame_bytes(frame: &Frame) -> Vec<u8> {
    let header = format!(
        "\x1etgtest\t{FRAME_VERSION}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        frame.name,
        u8::from(frame.failed),
        frame.assertions,
        frame.duration_ms,
        frame.stdout.len(),
        frame.stderr.len(),
    );
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(frame.stdout.as_bytes());
    bytes.extend_from_slice(frame.stderr.as_bytes());
    bytes
}

/// Open a machine-format test: everything `tg_test_begin` does, then reset
/// the assertion counter, start the clock and install a fresh capture sink —
/// discarding a stale one first, so a previous test that never closed cannot
/// make `install` refuse.
///
/// # Safety
/// - `name` must be a valid pointer to `len` bytes of UTF-8
#[no_mangle]
pub extern "C" fn tg_test_record_begin(name: *const c_char, len: u64) {
    tg_test_begin(name, len);
    ASSERTIONS.with(|count| count.set(0));
    RECORD_STARTED.with(|started| started.set(Some(Instant::now())));
    let _ = console_capture::take();
    let _ = console_capture::install();
}

/// Close a machine-format test: write its frame to the real stdout.
#[no_mangle]
pub extern "C" fn tg_test_record_end() {
    let mut stdout = std::io::stdout().lock();
    write_record_end(&mut stdout);
}

/// The frame the open bracket has collected; takes the sink.
fn end_frame() -> Frame {
    let captured = console_capture::take().unwrap_or_default();
    let elapsed = RECORD_STARTED.with(Cell::take).map_or(0, |started| {
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
    });
    let failed = TEST_FAILED.with(Cell::get);
    let assertions = ASSERTIONS.with(Cell::get);
    let name = TEST_NAME.with(|n| n.borrow().clone());
    frame_from(&captured, failed, assertions, elapsed, &name)
}

/// `tg_test_record_end` over an injected writer, so the bytes are assertable.
fn write_record_end(out: &mut impl std::io::Write) {
    let _ = out.write_all(&frame_bytes(&end_frame()));
    let _ = out.flush();
}

// The record bracket's tests (ADR 1.10.26i AC 2), a child of this module so
// they reach the private counter and writer; kept beside the other driver
// tests rather than inline so this file stays under the .rs size cap.
#[cfg(test)]
#[path = "tests/record_tests.rs"]
mod record_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nat_eq_pass() {
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_nat(42, 42);
        assert_eq!(tg_test_check_failure(), 0);
    }

    #[test]
    fn test_nat_eq_fail() {
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_nat(1, 2);
        assert_eq!(tg_test_check_failure(), 1);
    }

    /// 14.9.26c AC 2: the signed assertion compares as a SIGNED value — `-1`
    /// and `u64::MAX` share bits, and only one of them equals `-1`.
    #[test]
    fn int_eq_compares_signed_values() {
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_int(-1, -1);
        assert_eq!(tg_test_check_failure(), 0);
        tg_assert_eq_int(-1, 1);
        assert_eq!(tg_test_check_failure(), 1);
    }

    #[test]
    fn test_begin_resets_failure() {
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_nat(1, 2); // fail
        assert_eq!(tg_test_check_failure(), 1);
        tg_test_begin(std::ptr::null(), 0); // reset
        assert_eq!(tg_test_check_failure(), 0);
    }

    #[test]
    fn test_bool_eq_pass() {
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_bool(1, 1);
        assert_eq!(tg_test_check_failure(), 0);
    }

    #[test]
    fn test_bool_eq_fail() {
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_bool(0, 1);
        assert_eq!(tg_test_check_failure(), 1);
    }

    #[test]
    fn test_string_eq_pass() {
        tg_test_begin(std::ptr::null(), 0);
        let a = b"hello";
        let b = b"hello";
        tg_assert_eq_string(
            a.as_ptr().cast(),
            a.len() as u64,
            b.as_ptr().cast(),
            b.len() as u64,
        );
        assert_eq!(tg_test_check_failure(), 0);
    }

    #[test]
    fn mismatch_text_is_the_three_line_diff() {
        assert_eq!(
            mismatch_text("assert_eq_nat", "2", "3"),
            "  assertion failed: assert_eq_nat\n    left:  2\n    right: 3\n"
        );
    }

    /// 29.9.26i AC 3 / D8a: with a sink installed, each of the four
    /// assertions' diffs lands in its stderr buffer and not on the process's
    /// stderr; a passing assertion writes nothing.
    #[test]
    fn a_mismatch_diff_goes_to_an_installed_capture_sink() {
        use crate::ffi::console_capture;
        let _guard = console_capture::test_exclusive::exclusive_sink();
        console_capture::install().expect("no sink should be installed");
        tg_test_begin(std::ptr::null(), 0);
        tg_assert_eq_nat(7, 7);
        tg_assert_eq_nat(2, 3);
        tg_assert_eq_int(-1, 1);
        tg_assert_eq_bool(0, 1);
        let (a, b) = (b"x", b"y");
        tg_assert_eq_string(a.as_ptr().cast(), 1, b.as_ptr().cast(), 1);
        let captured = console_capture::take().expect("a sink was installed");
        let stderr = String::from_utf8(captured.stderr).unwrap();
        // `contains`, not equality: this module's other tests fail assertions
        // without holding the sink lock, and one running concurrently would
        // append its own diff here.
        for expected in [
            mismatch_text("assert_eq_nat", "2", "3"),
            mismatch_text("assert_eq_int", "-1", "1"),
            mismatch_text("assert_eq_bool", "false", "true"),
            mismatch_text("assert_eq_string", "\"x\"", "\"y\""),
        ] {
            assert!(
                stderr.contains(&expected),
                "{expected:?} missing from {stderr:?}"
            );
        }
        assert!(!stderr.contains("assert_eq_nat\n    left:  7"), "{stderr}");
        assert!(captured.stdout.is_empty());
        assert_eq!(tg_test_check_failure(), 1);
    }

    #[test]
    fn test_string_eq_fail() {
        tg_test_begin(std::ptr::null(), 0);
        let a = b"hello";
        let b = b"world";
        tg_assert_eq_string(
            a.as_ptr().cast(),
            a.len() as u64,
            b.as_ptr().cast(),
            b.len() as u64,
        );
        assert_eq!(tg_test_check_failure(), 1);
    }
}
