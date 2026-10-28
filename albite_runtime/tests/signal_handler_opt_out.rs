//! `ALBITE_NO_SIGNAL_HANDLER=1` opts out (ADR 30.9.26l D8, AC5).
//!
//! Its own file — and so its own process — because `sigaction` is
//! process-wide, and the only test in it, because it sets an environment
//! variable before any thread it spawns reads one. The comparison is relative,
//! so it holds whatever handler Rust's std installed first.
#![cfg(unix)]
#![allow(unsafe_code)] // Reason: querying sigaction/sigaltstack is a libc call; workspace lints apply to integration tests (ADR 30.9.26l D8)

use std::ptr;

/// The fields of an action that say what runs on a fault. The mask is read by
/// membership, not by bytes: glibc's `sigset_t` is 128 bytes of which the
/// kernel fills 8, and the rest comes back as whatever was on glibc's stack.
fn current_action(signal: libc::c_int) -> (libc::sighandler_t, libc::c_int, Vec<bool>) {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        assert_eq!(libc::sigaction(signal, ptr::null(), &raw mut action), 0);
        let mask = (1..32)
            .map(|member| libc::sigismember(&raw const action.sa_mask, member) == 1)
            .collect();
        (action.sa_sigaction, action.sa_flags, mask)
    }
}

fn current_alt_stack() -> (usize, usize, libc::c_int) {
    unsafe {
        let mut stack: libc::stack_t = std::mem::zeroed();
        assert_eq!(libc::sigaltstack(ptr::null(), &raw mut stack), 0);
        (stack.ss_sp as usize, stack.ss_size, stack.ss_flags)
    }
}

// ADR 30.9.26l AC5
#[test]
fn the_opt_out_variable_leaves_both_fault_actions_untouched() {
    std::env::set_var("ALBITE_NO_SIGNAL_HANDLER", "1");
    let segv_before = current_action(libc::SIGSEGV);
    let bus_before = current_action(libc::SIGBUS);
    let stack_before = current_alt_stack();

    albite_runtime::__albite_install_signal_handlers();

    assert_eq!(
        current_action(libc::SIGSEGV),
        segv_before,
        "SIGSEGV's action moved"
    );
    assert_eq!(
        current_action(libc::SIGBUS),
        bus_before,
        "SIGBUS's action moved"
    );
    assert_eq!(
        current_alt_stack(),
        stack_before,
        "the thread's alternate stack moved"
    );
}
