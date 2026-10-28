//! POSIX stack-overflow signal handler for compiled Tungsten programs.
//!
//! Catches SIGSEGV/SIGBUS caused by stack overflow and prints a diagnostic
//! message instead of crashing silently. Selected by [`super`] on `unix`
//! targets; see [`super::unsupported`] for the arm used elsewhere.
//!
//! Constraints:
//!   - Signal handler uses only async-signal-safe functions (`write`, `_exit`)
//!   - Runs on an alternate signal stack (`SA_ONSTACK`) since main stack is exhausted
//!   - Disabled when `ALBITE_NO_SIGNAL_HANDLER=1`
//!   - If sigaltstack allocation fails, program continues without handler
//!
//! See ADR 18.4.26g §5 for design rationale.

use core::ffi::c_void;
use core::ptr;

/// Alternate signal stack size: 64 KB.
const ALT_STACK_SIZE: usize = 64 * 1024;

/// Stack guard page detection: fault within this distance of stack limit.
const GUARD_THRESHOLD: usize = 64 * 1024;

/// Saved stack boundaries for guard page detection.
///
/// Written once during `install_signal_handlers` (single-threaded at that point),
/// read from the signal handler (inherently single-threaded).
static mut STACK_BASE: *mut c_void = ptr::null_mut();
static mut STACK_SIZE: usize = 0;
/// The guard below the stack's low end. The program body runs on a thread
/// `__albite_run_on_stack` gives a 1 MiB guard (ADR 19.10.26c D2), which a
/// large `-O0` frame can fault anywhere inside — well past the 64 KiB window.
static mut STACK_GUARD: usize = 0;

// ---- async-signal-safe write helpers ----

/// Write a byte slice to stderr (async-signal-safe).
fn safe_write(s: &[u8]) {
    unsafe {
        libc::write(libc::STDERR_FILENO, s.as_ptr().cast::<c_void>(), s.len());
    }
}

/// Write a u64 as "0x" + 16 hex digits to stderr (async-signal-safe).
fn safe_write_hex(val: u64) {
    let mut buf = [0u8; 18]; // "0x" + 16 hex digits
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let shift = (15 - i) * 4;
        let digit = ((val >> shift) & 0xF) as u8;
        buf[2 + i] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
    }
    safe_write(&buf);
}

/// Write a u64 as decimal to stderr (async-signal-safe).
fn safe_write_dec(mut val: u64) {
    if val == 0 {
        safe_write(b"0");
        return;
    }
    let mut buf = [0u8; 20]; // max 20 digits for u64
    let mut i = 20;
    while val > 0 {
        i -= 1;
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    safe_write(&buf[i..]);
}

// ---- stack boundary detection ----

/// Detect the current thread's stack base and size (macOS).
///
/// On macOS, `pthread_get_stackaddr_np` returns the TOP of the stack
/// (high address; stack grows down).
#[cfg(target_os = "macos")]
unsafe fn detect_stack_bounds() {
    let thread = libc::pthread_self();
    STACK_BASE = libc::pthread_get_stackaddr_np(thread).cast();
    STACK_SIZE = libc::pthread_get_stacksize_np(thread);
    // No `pthread_getattr_np` here to read it back: assume the guard the
    // runtime's entry asks for. On the fallback's main thread the real guard
    // is smaller, and a fault below it is no less an overflow.
    STACK_GUARD = crate::stack::STACK_GUARD_BYTES;
}

/// Detect the current thread's stack base and size (Linux).
///
/// On Linux, `pthread_attr_getstack` returns the LOW address of the stack.
/// Falls back to `getrlimit` if pthread attrs are unavailable.
#[cfg(target_os = "linux")]
#[allow(clippy::cast_possible_truncation)] // Reason: rlim_cur is a u64 and the Linux targets that link compiled programs are 64-bit, so the cast is lossless (ADR 30.9.26l D6)
unsafe fn detect_stack_bounds() {
    let mut attr: libc::pthread_attr_t = core::mem::zeroed();
    if libc::pthread_attr_init(&raw mut attr) == 0 {
        if libc::pthread_getattr_np(libc::pthread_self(), &raw mut attr) == 0 {
            let mut base: *mut c_void = ptr::null_mut();
            let mut size: libc::size_t = 0;
            libc::pthread_attr_getstack(&raw const attr, &raw mut base, &raw mut size);
            STACK_BASE = base;
            STACK_SIZE = size;
            let mut guard: usize = 0;
            if crate::stack::pthread_attr_getguardsize(&raw const attr, &raw mut guard) == 0 {
                STACK_GUARD = guard;
            }
        }
        libc::pthread_attr_destroy(&raw mut attr);
    }
    // Fallback: use getrlimit
    if STACK_SIZE == 0 {
        let mut rl: libc::rlimit = core::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_STACK, &raw mut rl) == 0
            && rl.rlim_cur != libc::RLIM_INFINITY
        {
            STACK_SIZE = rl.rlim_cur as usize;
        } else {
            STACK_SIZE = 8 * 1024 * 1024; // default 8 MB
        }
    }
}

// ---- guard page detection ----

/// Check if a fault address is near the stack guard page.
unsafe fn is_stack_guard_fault(fault_addr: *mut c_void) -> bool {
    if STACK_SIZE == 0 {
        return false;
    }

    // On macOS, STACK_BASE is the TOP (high address); stack grows down.
    // On Linux, STACK_BASE is the LOW address from pthread_attr_getstack.
    #[cfg(target_os = "macos")]
    let stack_low = (STACK_BASE as usize).wrapping_sub(STACK_SIZE);

    #[cfg(target_os = "linux")]
    let stack_low = STACK_BASE as usize;

    in_overflow_window(fault_addr as usize, stack_low, STACK_GUARD)
}

/// Whether `addr` is a stack-overflow fault for a stack whose low end is
/// `stack_low` with a `guard`-byte guard below it: anywhere from the guard's
/// low end to [`GUARD_THRESHOLD`] above `stack_low`. A guard smaller than the
/// threshold keeps the old symmetric 64 KiB window (ADR 19.10.26c D2).
fn in_overflow_window(addr: usize, stack_low: usize, guard: usize) -> bool {
    let low_bound = stack_low.wrapping_sub(guard.max(GUARD_THRESHOLD));
    let high_bound = stack_low.wrapping_add(GUARD_THRESHOLD);
    addr >= low_bound && addr <= high_bound
}

// ---- signal handler ----

/// SIGSEGV/SIGBUS handler that detects stack overflow.
///
/// If the fault address is near the stack guard page, prints a diagnostic
/// and exits. Otherwise, re-raises the original signal.
///
/// All operations are async-signal-safe.
#[allow(clippy::borrow_as_ptr)] // Reason: runs only on a real fault; no test can observe it (ADR 30.9.26l D6)
extern "C" fn sigsegv_handler(sig: libc::c_int, si: *mut libc::siginfo_t, _ctx: *mut c_void) {
    unsafe {
        if is_stack_guard_fault((*si).si_addr()) {
            safe_write(b"\n*** Stack overflow detected ***\n");
            safe_write(b"  Fault address:  ");
            safe_write_hex((*si).si_addr() as u64);
            safe_write(b"\n");
            safe_write(b"  Stack size:     ");
            safe_write_dec(STACK_SIZE as u64);
            safe_write(b" bytes (");
            safe_write_dec((STACK_SIZE / (1024 * 1024)) as u64);
            safe_write(b" MB)\n");
            safe_write(b"\n");
            safe_write(b"  Likely cause: unbounded recursion in a function without\n");
            safe_write(b"  tail-call optimization.\n");
            safe_write(b"\n");
            safe_write(b"  Hint: compile with --named-lambdas to see function names\n");
            safe_write(b"  in the backtrace, or run:\n");
            safe_write(b"    tungsten doctor audit-recursion <file>\n");
            safe_write(b"  to identify recursive functions at risk.\n");
            libc::_exit(134); // 128 + SIGSEGV(6) — convention for signal exits
        }

        // Not a stack overflow — re-raise original signal with default handler
        let mut sa: libc::sigaction = core::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = 0;
        libc::sigaction(sig, &sa, ptr::null_mut());
        libc::raise(sig);
    }
}

// ---- public API ----

/// Install signal handlers for stack overflow detection.
///
/// Called from the `main()` prologue of compiled Tungsten programs.
/// Allocates an alternate signal stack (64 KB via mmap) and installs
/// handlers for SIGSEGV and SIGBUS.
///
/// Skipped when:
/// - `ALBITE_NO_SIGNAL_HANDLER=1` environment variable is set
/// - Alternate stack allocation fails (graceful degradation)
///
/// # Safety
///
/// This function modifies global signal handlers. It should be called
/// once, early in program startup, before any threads are spawned.
#[no_mangle]
pub extern "C" fn __albite_install_signal_handlers() {
    unsafe {
        // Check for the opt-out setting. The installer runs from the
        // generated `main` prologue and already `mmap`s, so reading through
        // the settings seam is safe here; the no-allocation contract is the
        // handler's, not the installer's (ADR 20.10.26e D3).
        if crate::settings::read("NO_SIGNAL_HANDLER").is_some_and(|value| value.starts_with('1')) {
            return;
        }

        // Detect stack boundaries
        detect_stack_bounds();

        // Allocate alternate signal stack via mmap
        let alt_stack = libc::mmap(
            ptr::null_mut(),
            ALT_STACK_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        if alt_stack == libc::MAP_FAILED {
            // Graceful degradation — continue without handler
            return;
        }

        let ss = libc::stack_t {
            ss_sp: alt_stack,
            ss_size: ALT_STACK_SIZE,
            ss_flags: 0,
        };
        if libc::sigaltstack(&raw const ss, ptr::null_mut()) != 0 {
            libc::munmap(alt_stack, ALT_STACK_SIZE);
            return;
        }

        // Install handler for SIGSEGV and SIGBUS
        let mut sa: libc::sigaction = core::mem::zeroed();
        sa.sa_sigaction = sigsegv_handler as libc::sighandler_t;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        libc::sigemptyset(&raw mut sa.sa_mask);

        libc::sigaction(libc::SIGSEGV, &raw const sa, ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &raw const sa, ptr::null_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_write_dec_zero() {
        // Smoke test — just verify it doesn't panic.
        // Output goes to stderr which we can't easily capture in unit tests.
        safe_write_dec(0);
    }

    #[test]
    fn test_safe_write_dec_large() {
        safe_write_dec(18_446_744_073_709_551_615); // u64::MAX
    }

    #[test]
    fn test_safe_write_hex() {
        safe_write_hex(0xDEAD_BEEF_CAFE_BABE);
    }

    const LOW: usize = 0x7000_0000_0000;
    const MIB: usize = 1024 * 1024;

    /// ADR 19.10.26c D2: a 1 MiB guard widens the window below the stack's
    /// low end to the guard's low end; above it stays 64 KiB.
    #[test]
    fn a_one_mib_guard_widens_the_window_to_the_guards_low_end() {
        assert!(
            in_overflow_window(LOW - MIB, LOW, MIB),
            "the guard's low end"
        );
        assert!(
            !in_overflow_window(LOW - MIB - 1, LOW, MIB),
            "one below the guard"
        );
        assert!(
            in_overflow_window(LOW - 200 * 1024, LOW, MIB),
            "deep in the guard"
        );
        assert!(in_overflow_window(LOW, LOW, MIB));
        assert!(in_overflow_window(LOW + GUARD_THRESHOLD, LOW, MIB));
        assert!(!in_overflow_window(LOW + GUARD_THRESHOLD + 1, LOW, MIB));
    }

    /// A guard smaller than 64 KiB (the main thread's, or none read) keeps
    /// the symmetric window the handler always had.
    #[test]
    fn a_small_or_unknown_guard_keeps_the_symmetric_window() {
        for guard in [0, 4096, GUARD_THRESHOLD] {
            assert!(in_overflow_window(LOW - GUARD_THRESHOLD, LOW, guard));
            assert!(!in_overflow_window(LOW - GUARD_THRESHOLD - 1, LOW, guard));
            assert!(!in_overflow_window(LOW - 200 * 1024, LOW, guard));
            assert!(in_overflow_window(LOW + GUARD_THRESHOLD, LOW, guard));
        }
    }
}
