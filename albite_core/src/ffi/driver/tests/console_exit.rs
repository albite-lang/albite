//! `tg_exit`'s report decision (ADR 30.9.26b).
//!
//! `tg_exit` itself ends the process, so only its pure decision is tested here;
//! that the report actually prints is AC 2's profiled self-check.

use crate::ffi::driver::console::report_on_exit;

/// 30.9.26b AC 1: a profiled program prints its allocation report on exit, an
/// unprofiled one does not.
#[test]
fn report_on_exit_prints_only_when_the_profiler_is_active() {
    assert!(report_on_exit(true));
    assert!(!report_on_exit(false));
}
