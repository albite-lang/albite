//! Console, CLI, and process control FFI functions.
//!
//! Terminal I/O: print, println, eprint, `debug_tag`
//! CLI arguments: `init_args`, argc, argv
//! Process control: exit, TTY detection, subprocess execution, environment

use std::ffi::{c_char, CStr};
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicI64, AtomicPtr, Ordering};

use super::console_capture::{self, write_stderr, write_stdout};
use super::{clear_driver_error, set_driver_error};

// ============================================================================
// Console Output
// ============================================================================

/// Print to stdout (no newline).
///
/// Routed through [`super::console_capture`], so the bytes land in an installed
/// capture sink instead of the process stream. With no sink installed — the
/// default, and every native invocation — this is the original stream write.
///
/// # Safety
/// - `s` must be a valid pointer to `len` bytes
#[no_mangle]
pub extern "C" fn tg_print(s: *const c_char, len: u64) {
    if s.is_null() {
        return;
    }

    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len as usize) };
    write_stdout(&[slice]);
}

/// Print to stdout with newline.
///
/// Capture-aware; see [`tg_print`].
///
/// # Safety
/// - `s` must be a valid pointer to `len` bytes
#[no_mangle]
pub extern "C" fn tg_println(s: *const c_char, len: u64) {
    if s.is_null() {
        write_stdout(&[b"\n"]);
        return;
    }

    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len as usize) };
    write_stdout(&[slice, b"\n"]);
}

/// Print to stderr with newline.
///
/// Capture-aware; see [`tg_print`]. Captured stderr is kept in its own buffer —
/// merging the two streams at the source would destroy a distinction a consumer
/// cannot recover afterwards.
///
/// # Safety
/// - `s` must be a valid pointer to `len` bytes
#[no_mangle]
pub extern "C" fn tg_eprintln(s: *const c_char, len: u64) {
    if s.is_null() {
        write_stderr(&[b"\n"]);
        return;
    }

    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len as usize) };
    write_stderr(&[slice, b"\n"]);
}

/// Debug print a u8 value to stderr.
/// Used for tracing tag values in sum types.
#[no_mangle]
pub extern "C" fn tg_debug_tag(label: *const c_char, label_len: u64, tag: u8) {
    let label_slice = if label.is_null() {
        b"tag" as &[u8]
    } else {
        unsafe { std::slice::from_raw_parts(label.cast::<u8>(), label_len as usize) }
    };

    eprintln!(
        "[DEBUG] {}: {}",
        std::str::from_utf8(label_slice).unwrap_or("???"),
        tag
    );
}

// ============================================================================
// CLI Arguments
// ============================================================================

// The argument store (ADR 7.10.26r D1): the count and `argv` itself, as C
// data symbols, so the native core's init (`tg_native_init_args_c`, whose
// `tg_core_publish_args` lowering writes them as external globals) and Rust's
// `tg_init_args_c` publish into one store that the Rust readers read. No
// argument is copied at startup: C keeps `argv`'s strings alive until the
// program ends (C11 5.1.2.2.1), and `tg_argv` copies on every read anyway.

/// The published argument count; meaningful only once the vector is non-null.
/// Its C symbol, `tg_core_args_count`, is what the core's IR names.
#[export_name = "tg_core_args_count"]
pub static PUBLISHED_ARGS_COUNT: AtomicI64 = AtomicI64::new(0);

/// The published `argv`; null until the first publish. C symbol
/// `tg_core_args_vector`.
#[export_name = "tg_core_args_vector"]
pub static PUBLISHED_ARGS_VECTOR: AtomicPtr<*const c_char> = AtomicPtr::new(ptr::null_mut());

/// Publish `(count, argv)` once (D1): a second publish is a no-op, as
/// `CLI_ARGS`'s `OnceLock` was. The count is stored before the vector is
/// exchanged in, so a reader that sees the vector sees its count. Only
/// `main` publishes, before any thread starts.
fn publish_args(count: i64, argv: *const *const c_char) {
    if !PUBLISHED_ARGS_VECTOR.load(Ordering::Acquire).is_null() {
        return;
    }
    PUBLISHED_ARGS_COUNT.store(count, Ordering::Release);
    let _ = PUBLISHED_ARGS_VECTOR.compare_exchange(
        ptr::null_mut(),
        argv.cast_mut(),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

/// The published pair, vector first: `(0, null)` before any publish.
fn published_args() -> (i64, *const *const c_char) {
    let argv = PUBLISHED_ARGS_VECTOR.load(Ordering::Acquire);
    if argv.is_null() {
        return (0, ptr::null());
    }
    (
        PUBLISHED_ARGS_COUNT.load(Ordering::Acquire),
        argv.cast_const(),
    )
}

/// The reader's decision (D1): a fresh copy of `argv[index]`, freed by
/// `tg_free_string`, or null for a null `argv`, a negative index, an index
/// at or past `count`, or a null entry.
///
/// # Safety
/// `argv`, when non-null, points to at least `count` entries, each null or a
/// NUL-terminated string.
#[must_use]
pub unsafe fn argv_at(count: i64, argv: *const *const c_char, index: i64) -> *mut c_char {
    if argv.is_null() || index < 0 || index >= count {
        return ptr::null_mut();
    }
    let Ok(offset) = usize::try_from(index) else {
        return ptr::null_mut();
    };
    let entry = *argv.add(offset);
    if entry.is_null() {
        return ptr::null_mut();
    }
    CStr::from_ptr(entry).to_owned().into_raw()
}

/// Initialize CLI args (C ABI version for main wrapper).
///
/// This is called from the C `main()` wrapper with (i32, ptr) arguments,
/// under `ALBITE_NATIVE_CORE=off` (D8) or when the core's row is off.
///
/// # Safety
/// - `argc` must match the number of pointers in `argv`
/// - `argv` must be an array of valid null-terminated C strings that
///   outlives every reader, as C's `main` guarantees
/// - Subsequent calls are no-ops
#[no_mangle]
#[allow(clippy::similar_names)] // Reason: argc/argv/args are the C convention
pub unsafe extern "C" fn tg_init_args_c(argc: i32, argv: *const *const c_char) {
    tg_init_args(i64::from(argc), argv as i64);
}

/// Initialize CLI args (Tungsten ABI version): publish the pair, no copy.
///
/// # Safety
/// As [`tg_init_args_c`], with `argv` cast to `i64`.
#[no_mangle]
#[allow(clippy::similar_names)] // Reason: argc/argv/args are the C convention
pub unsafe extern "C" fn tg_init_args(argc: i64, argv: i64) {
    publish_args(argc, argv as *const *const c_char);
}

/// Get argument count; 0 before any init.
#[no_mangle]
pub extern "C" fn tg_argc() -> i64 {
    published_args().0
}

/// Get argument at index.
///
/// Returns null if index is out of bounds or args not initialized.
/// The returned string must be freed with `tg_free_string`.
#[no_mangle]
pub extern "C" fn tg_argv(index: i64) -> *mut c_char {
    let (count, argv) = published_args();
    // Safety: a published `argv` is `main`'s, `count` entries long (D1).
    unsafe { argv_at(count, argv, index) }
}

// ============================================================================
// Process Control
// ============================================================================

#[cfg(unix)]
extern "C" {
    /// C library's _Exit function - immediate termination without cleanup
    fn _Exit(status: i32) -> !;
}

/// Exit the process with given code.
///
/// This function does not return.
/// Uses C's _`Exit()` for immediate termination without running cleanup handlers,
/// avoiding issues with Rust's atexit handlers or other cleanup code.
///
/// A profiled program prints its allocation report first (ADR 30.9.26b D1):
/// `_Exit` never returns to the `__albite_alloc_profile_report` call codegen
/// places after `__albite_inner_main`, so a program ending in `process_exit`
/// would otherwise lose the table. Unprofiled, this is one load and a branch.
#[cfg(unix)]
#[no_mangle]
pub extern "C" fn tg_exit(code: i32) -> ! {
    if report_on_exit(albite_runtime::alloc_profile_is_active()) {
        albite_runtime::__albite_alloc_profile_report();
    }
    unsafe { _Exit(code) }
}

/// Whether `tg_exit` prints the allocation report before exiting — only when
/// the profiler is active, which only a `--alloc-profile` build ever makes it
/// (ADR 30.9.26b D2). Pure, so it is tested without ending a process.
#[must_use]
pub fn report_on_exit(profiler_active: bool) -> bool {
    profiler_active
}

/// Abort on an `Int` trap (ADR 14.9.26c §2.3): overflow, a zero divisor, or a
/// bridge out of range. `kind` is [`IntTrapKind::code`]; the line printed is
/// [`IntTrapKind::message`], byte-identical to what the evaluator renders for
/// the same stop, which is the `diff exec` parity contract (AC 3).
///
/// An FFI call rather than a bare `llvm.trap` so both paths share one table.
/// `abort()` rather than `_Exit`: the language has no unwinding, and a trap is
/// a program error, not a status.
///
/// [`IntTrapKind::code`]: crate::eval::IntTrapKind::code
/// [`IntTrapKind::message`]: crate::eval::IntTrapKind::message
#[no_mangle]
pub extern "C" fn tg_int_trap(kind: u32) -> ! {
    let line = int_trap_line(kind);
    write_stderr(&[line.as_bytes(), b"\n"]);
    std::process::abort()
}

/// The stderr line for a trap code — the pure half of [`tg_int_trap`].
///
/// Carries the `error: ` prefix `tungsten run` puts in front of every
/// `EvalStopped`, so the two stderr lines are byte-identical (AC 3).
#[must_use]
pub fn int_trap_line(kind: u32) -> String {
    match crate::eval::IntTrapKind::from_code(kind) {
        Some(kind) => format!("error: {}", kind.message()),
        None => format!("error: integer trap (unknown kind {kind})"),
    }
}

/// Terminate immediately on a target with no `_Exit` (ADR 28.7.26a Phase 4).
///
/// The `extern "C" { fn _Exit }` above is a *declaration*, so on
/// `wasm32-unknown-unknown` it compiles cleanly and then leaves an unresolved
/// `env::_Exit` import — a module the browser refuses to instantiate. A green
/// `cargo build` therefore does not prove a loadable artifact, which is why
/// this arm exists and why the build is smoke-run.
///
/// `abort` is the honest equivalent: wasm has no exit status to deliver, and
/// aborting the instance is what the embedder observes. The `code` is dropped
/// because there is nowhere to put it.
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn tg_exit(_code: i32) -> ! {
    std::process::abort()
}

// ============================================================================
// TTY Detection (for colored output)
// ============================================================================

/// Whether to report a stream as a terminal, given whether it actually is one.
///
/// Always 0 while a capture sink is installed, whatever the real stream is: the
/// captured bytes are handed to a consumer that renders them itself, and colour
/// escapes in a buffer would show up literally (ADR 28.7.26a §2.2).
///
/// Separated from the two FFI entry points because it is the part with a
/// decision in it. Whether the process's stdout is a terminal cannot be
/// controlled from an in-process test — it is never one under a test harness —
/// so folding this rule into the entry points would make it permanently
/// unassertable.
pub(super) fn reported_tty(stream_is_tty: bool) -> i32 {
    if console_capture::is_active() {
        return 0;
    }
    i32::from(stream_is_tty)
}

/// Check if stdout supports color output (is a TTY).
///
/// Returns 1 if TTY, 0 if not — and always 0 under capture, per
/// [`reported_tty`].
#[no_mangle]
pub extern "C" fn tg_stdout_is_tty() -> i32 {
    reported_tty(atty::is(atty::Stream::Stdout))
}

/// Check if stderr supports color output (is a TTY).
///
/// Returns 1 if TTY, 0 if not — and always 0 under capture, per
/// [`reported_tty`].
#[no_mangle]
pub extern "C" fn tg_stderr_is_tty() -> i32 {
    reported_tty(atty::is(atty::Stream::Stderr))
}

// ============================================================================
// Subprocess Execution
// ============================================================================

/// Execute a program with arguments.
///
/// `program` is the executable path (null-terminated C string).
/// `args` is a newline-delimited C string of arguments (each line is one arg).
///   An empty string means no arguments.
///
/// The subprocess inherits stdout/stderr, so its output flows through naturally.
///
/// Returns the process exit code (0 = success), or -1 if the process fails to
/// spawn. On spawn failure, call `tg_driver_get_last_error` for details.
///
/// All arguments are constructed from trusted paths; no user input is interpolated.
#[no_mangle]
pub extern "C" fn tg_exec_process(program: *const c_char, args: *const c_char) -> i64 {
    clear_driver_error();

    if program.is_null() {
        set_driver_error("program path is null");
        return -1;
    }

    let program_str = match unsafe { CStr::from_ptr(program) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_driver_error(format!("invalid program path: {e}"));
            return -1;
        }
    };

    let mut cmd = Command::new(program_str);

    // Parse newline-delimited arguments
    if !args.is_null() {
        if let Ok(args_str) = unsafe { CStr::from_ptr(args) }.to_str() {
            for arg in args_str.split('\n') {
                if !arg.is_empty() {
                    cmd.arg(arg);
                }
            }
        }
    }

    match cmd.status() {
        Ok(status) => i64::from(status.code().unwrap_or(-1)),
        Err(e) => {
            set_driver_error(format!("failed to execute '{program_str}': {e}"));
            -1
        }
    }
}
