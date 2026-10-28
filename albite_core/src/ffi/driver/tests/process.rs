//! Process FFI: subprocess execution, environment, tty detection.

use std::ffi::CString;

use crate::ffi::driver::console::{tg_exec_process, tg_stderr_is_tty, tg_stdout_is_tty};
use crate::ffi::driver::environment::tg_getenv;
use crate::ffi::driver::io::tg_cstring_to_string;
use crate::ffi::driver::tg_free_string;
use crate::ffi::test_support::ScratchDir;

/// 14.9.26c AC 3: the line `tg_int_trap` prints is the evaluator's line for
/// the same stop, code for code — the parity the `diff exec` fixture relies on.
#[test]
fn int_trap_line_matches_the_evaluator_message_for_every_code() {
    use crate::eval::{EvalStopped, IntTrapKind};
    use crate::ffi::driver::console::int_trap_line;
    for code in 0..14 {
        let kind = IntTrapKind::from_code(code).expect("dense table");
        // `tungsten run` prints `error: {stopped}` — the prefix is part of the line.
        let rendered = format!("error: {}", EvalStopped::IntTrap { kind });
        assert_eq!(int_trap_line(code), rendered, "code {code}");
    }
    assert_eq!(int_trap_line(0), "error: integer overflow in +");
    // ADR 3.10.26n D6: the five byte-buffer lines, operand-free, as the
    // `run/bytes_trap_*` goldens pin them.
    assert_eq!(int_trap_line(8), "error: byte index out of range");
    assert_eq!(int_trap_line(9), "error: byte value out of range");
    assert_eq!(
        int_trap_line(10),
        "error: byte buffer used after conversion"
    );
    assert_eq!(int_trap_line(11), "error: byte buffer too large");
    assert_eq!(int_trap_line(12), "error: byte buffer is not valid UTF-8");
    // 16.10.26h AC 3: the substring cut, operand-free like the byte lines.
    assert_eq!(
        int_trap_line(13),
        "error: substring cuts inside a character"
    );
    assert_eq!(int_trap_line(42), "error: integer trap (unknown kind 42)");
}

#[test]
fn test_tty_detection() {
    // These should return 0 or 1 without crashing
    let stdout_tty = tg_stdout_is_tty();
    let stderr_tty = tg_stderr_is_tty();
    assert!(stdout_tty == 0 || stdout_tty == 1);
    assert!(stderr_tty == 0 || stderr_tty == 1);
}

#[test]
fn test_exec_process_success() {
    let program = CString::new("echo").unwrap();
    let args = CString::new("hello").unwrap();
    let code = tg_exec_process(program.as_ptr(), args.as_ptr());
    assert_eq!(code, 0);
}

#[test]
fn test_exec_process_failure() {
    let program = CString::new("false").unwrap();
    let args = CString::new("").unwrap();
    let code = tg_exec_process(program.as_ptr(), args.as_ptr());
    assert_ne!(code, 0);
}

#[test]
fn test_exec_process_nonexistent() {
    let program = CString::new("/nonexistent/binary/xxxxx").unwrap();
    let args = CString::new("").unwrap();
    let code = tg_exec_process(program.as_ptr(), args.as_ptr());
    assert_eq!(code, -1);
}

#[test]
fn test_exec_process_multi_args() {
    // echo with multiple args separated by newlines
    let program = CString::new("echo").unwrap();
    let args = CString::new("arg1\narg2\narg3").unwrap();
    let code = tg_exec_process(program.as_ptr(), args.as_ptr());
    assert_eq!(code, 0);
}

#[test]
fn test_exec_process_null_args() {
    let program = CString::new("true").unwrap();
    let code = tg_exec_process(program.as_ptr(), std::ptr::null());
    assert_eq!(code, 0);
}

#[test]
fn test_getenv_exists() {
    // PATH should always be set
    let name = CString::new("PATH").unwrap();
    let result = tg_getenv(name.as_ptr());
    assert!(!result.is_null());
    // Clean up
    tg_free_string(result);
}

/// The UTF-8 text of a reader's fresh copy, freed here; `None` for null.
fn take_arg(copy: *mut std::ffi::c_char) -> Option<String> {
    if copy.is_null() {
        return None;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(copy) }
        .to_str()
        .expect("utf-8")
        .to_string();
    tg_free_string(copy);
    Some(text)
}

/// ADR 7.10.26r AC 2: `argv_at` over an injected array returns each
/// argument as a fresh copy, and null for a negative index, at the count, for
/// a null entry, and for a null `argv` (ADR 28.9.26h's bounds, kept).
#[test]
fn argv_at_reads_each_argument_and_refuses_everything_else() {
    use crate::ffi::driver::console::argv_at;
    let owned = [
        CString::new("prog").unwrap(),
        CString::new("").unwrap(),
        CString::new("héllo").unwrap(),
    ];
    let mut pointers: Vec<*const std::ffi::c_char> = owned.iter().map(|a| a.as_ptr()).collect();
    pointers.push(std::ptr::null());
    let argv = pointers.as_ptr();
    let read = |count, index| take_arg(unsafe { argv_at(count, argv, index) });
    assert_eq!(read(3, 0).as_deref(), Some("prog"));
    assert_eq!(read(3, 1).as_deref(), Some(""));
    assert_eq!(read(3, 2).as_deref(), Some("héllo"));
    assert_eq!(read(3, -1), None, "a negative index");
    assert_eq!(read(3, 3), None, "at the count");
    assert_eq!(read(2, 2), None, "past a smaller count");
    assert_eq!(read(4, 3), None, "a null entry below the count");
    let copy = unsafe { argv_at(3, argv, 0) };
    assert_ne!(copy.cast_const(), pointers[0], "a copy, not the entry");
    tg_free_string(copy);
    assert!(unsafe { argv_at(3, std::ptr::null(), 0) }.is_null());
}

/// Resets the process-wide store to `(0, null)` when dropped, so the one
/// test that publishes leaves no dangling `argv` behind (ADR 7.10.26r D4).
struct PublishedArgsReset;

impl Drop for PublishedArgsReset {
    fn drop(&mut self) {
        use crate::ffi::driver::console::{PUBLISHED_ARGS_COUNT, PUBLISHED_ARGS_VECTOR};
        use std::sync::atomic::Ordering;
        PUBLISHED_ARGS_VECTOR.store(std::ptr::null_mut(), Ordering::Release);
        PUBLISHED_ARGS_COUNT.store(0, Ordering::Release);
    }
}

/// ADR 7.10.26r AC 2: `tg_init_args_c` publishes into the statics, the
/// readers read them, and a second publish is a no-op. The only test that
/// touches the store, under a reset guard.
#[test]
fn the_rust_init_publishes_once_and_the_readers_read_the_store() {
    use crate::ffi::driver::console::{
        tg_argc, tg_argv, tg_init_args_c, PUBLISHED_ARGS_COUNT, PUBLISHED_ARGS_VECTOR,
    };
    use std::sync::atomic::Ordering;
    let reset = PublishedArgsReset;
    assert_eq!(tg_argc(), 0, "nothing published yet");
    assert!(tg_argv(0).is_null());
    let owned = [
        CString::new("prog").unwrap(),
        CString::new("--flag").unwrap(),
    ];
    let pointers: Vec<*const std::ffi::c_char> = owned.iter().map(|a| a.as_ptr()).collect();
    unsafe { tg_init_args_c(2, pointers.as_ptr()) };
    assert_eq!(PUBLISHED_ARGS_COUNT.load(Ordering::Acquire), 2);
    assert_eq!(
        PUBLISHED_ARGS_VECTOR.load(Ordering::Acquire).cast_const(),
        pointers.as_ptr()
    );
    assert_eq!(tg_argc(), 2);
    assert_eq!(take_arg(tg_argv(1)).as_deref(), Some("--flag"));
    assert_eq!(take_arg(tg_argv(2)), None, "one past the end");
    assert_eq!(take_arg(tg_argv(-1)), None, "a negative index");

    let later = [CString::new("other").unwrap()];
    let later_pointers: Vec<*const std::ffi::c_char> = later.iter().map(|a| a.as_ptr()).collect();
    unsafe { tg_init_args_c(1, later_pointers.as_ptr()) };
    assert_eq!(tg_argc(), 2, "a second publish is a no-op");
    assert_eq!(take_arg(tg_argv(0)).as_deref(), Some("prog"));

    // The guard's own contract: dropping it leaves `(0, null)`, so `owned`
    // never outlives the store that points into it.
    drop(reset);
    assert!(PUBLISHED_ARGS_VECTOR.load(Ordering::Acquire).is_null());
    assert_eq!(PUBLISHED_ARGS_COUNT.load(Ordering::Acquire), 0);
    assert_eq!(tg_argc(), 0);
}

#[test]
fn test_getenv_not_set() {
    let name = CString::new("TUNGSTEN_TEST_NONEXISTENT_VAR_12345").unwrap();
    let result = tg_getenv(name.as_ptr());
    assert!(result.is_null());
}

/// 1.10.26i AC 5: the child's stdout lands in the file, its stderr does not,
/// and its exit code comes back.
#[test]
fn capture_exec_writes_child_stdout_to_the_file() {
    use crate::ffi::driver::process_capture::tg_exec_process_capture;
    let dir = ScratchDir::new("capture-writes");
    let out_path = dir.join("frames.out");
    let program = CString::new("sh").unwrap();
    let args = CString::new("-c\nprintf 'to-out'; printf 'to-err' >&2; exit 3").unwrap();
    let path = CString::new(out_path.to_str().unwrap()).unwrap();
    let code = tg_exec_process_capture(program.as_ptr(), args.as_ptr(), path.as_ptr());
    assert_eq!(code, 3);
    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "to-out");
}

/// 1.10.26i AC 5: a program that cannot be spawned is `-1` with the driver
/// error set; so is a null argument or a file that cannot be created.
#[test]
fn capture_exec_reports_spawn_failure() {
    use crate::ffi::driver::process_capture::tg_exec_process_capture;
    use crate::ffi::driver::tg_driver_get_last_error;
    let dir = ScratchDir::new("capture-fails");
    let path = CString::new(dir.join("o").to_str().unwrap()).unwrap();
    let missing = CString::new("/nonexistent/binary/xxxxx").unwrap();
    let code = tg_exec_process_capture(missing.as_ptr(), std::ptr::null(), path.as_ptr());
    assert_eq!(code, -1);
    let err = tg_driver_get_last_error();
    assert!(!err.is_null(), "the reason is recorded");
    let reason = unsafe { std::ffi::CStr::from_ptr(err) }
        .to_str()
        .unwrap()
        .to_string();
    tg_free_string(err);
    assert!(reason.contains("failed to execute"), "{reason}");
    let program = CString::new("true").unwrap();
    assert_eq!(
        tg_exec_process_capture(program.as_ptr(), std::ptr::null(), std::ptr::null()),
        -1
    );
    let unwritable = CString::new("/nonexistent/dir/o").unwrap();
    assert_eq!(
        tg_exec_process_capture(program.as_ptr(), std::ptr::null(), unwritable.as_ptr()),
        -1
    );
    assert_eq!(
        tg_exec_process_capture(program.as_ptr(), std::ptr::null(), path.as_ptr()),
        0
    );
}
