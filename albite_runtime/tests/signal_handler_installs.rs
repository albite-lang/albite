//! `__albite_install_signal_handlers` installs (ADR 30.9.26l D8, AC5).
//!
//! Its own file — and so its own process — because `sigaction` and the
//! calling thread's `sigaltstack` are process/thread state and libtest runs a
//! file's tests in one process. Nothing raises a signal: the check is a pure
//! before/after query. Flags are not a discriminator, because Rust's std
//! already installs its own `SA_SIGINFO | SA_ONSTACK` stack-overflow handler.
#![cfg(unix)]
#![allow(unsafe_code)] // Reason: querying sigaction/sigaltstack is a libc call; workspace lints apply to integration tests (ADR 30.9.26l D8)

use std::ptr;

/// The size `unix.rs` maps for its alternate signal stack.
const ALT_STACK_SIZE: usize = 64 * 1024;

fn current_action(signal: libc::c_int) -> libc::sigaction {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        assert_eq!(libc::sigaction(signal, ptr::null(), &raw mut action), 0);
        action
    }
}

fn current_alt_stack() -> libc::stack_t {
    unsafe {
        let mut stack: libc::stack_t = std::mem::zeroed();
        assert_eq!(libc::sigaltstack(ptr::null(), &raw mut stack), 0);
        stack
    }
}

// ADR 30.9.26l AC5
#[test]
fn installing_replaces_both_fault_actions_with_an_on_stack_handler() {
    assert!(
        std::env::var_os("ALBITE_NO_SIGNAL_HANDLER").is_none(),
        "the opt-out variable must be unset for this test to mean anything"
    );
    let segv_before = current_action(libc::SIGSEGV);
    let bus_before = current_action(libc::SIGBUS);

    albite_runtime::__albite_install_signal_handlers();

    let segv_after = current_action(libc::SIGSEGV);
    let bus_after = current_action(libc::SIGBUS);
    assert_ne!(
        segv_after.sa_sigaction, segv_before.sa_sigaction,
        "SIGSEGV's handler did not change"
    );
    assert_ne!(
        bus_after.sa_sigaction, bus_before.sa_sigaction,
        "SIGBUS's handler did not change"
    );
    assert_eq!(
        segv_after.sa_sigaction, bus_after.sa_sigaction,
        "SIGSEGV and SIGBUS share one handler"
    );
    let wanted = libc::SA_SIGINFO | libc::SA_ONSTACK;
    assert_eq!(segv_after.sa_flags & wanted, wanted);

    let stack = current_alt_stack();
    assert_eq!(
        stack.ss_size, ALT_STACK_SIZE,
        "the 64 KB alternate stack is installed"
    );
    assert_eq!(
        stack.ss_flags & libc::SS_DISABLE,
        0,
        "the alternate stack is enabled"
    );
}
