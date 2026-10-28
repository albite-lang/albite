//! ADR 19.10.26c AC1: `ALBITE_STACK_SIZE` parsing, bounds and rounding,
//! the guard the entry asks for, the body's exit code through the join, and
//! the fallback when the spawn fails.

use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

const PAGE_4K: usize = 4096;

#[test]
fn unset_or_blank_is_the_default() {
    assert_eq!(stack_plan(None, PAGE_4K), Ok(DEFAULT_STACK_BYTES));
    assert_eq!(stack_plan(Some(""), PAGE_4K), Ok(DEFAULT_STACK_BYTES));
    assert_eq!(stack_plan(Some("  "), PAGE_4K), Ok(DEFAULT_STACK_BYTES));
}

// 19.10.26b: the self-host's largest passing -O0 limit is 10 MiB; D3's
// default is at least four times it.
const _: () = assert!(DEFAULT_STACK_BYTES >= 4 * 10 * 1024 * 1024);

#[test]
fn the_default_is_512_mib() {
    assert_eq!(DEFAULT_STACK_BYTES, 512 * 1024 * 1024);
}

#[test]
fn each_suffix_is_a_power_of_1024() {
    assert_eq!(stack_plan(Some("2097152"), PAGE_4K), Ok(2 * 1024 * 1024));
    assert_eq!(stack_plan(Some("2048K"), PAGE_4K), Ok(2 * 1024 * 1024));
    assert_eq!(stack_plan(Some("8M"), PAGE_4K), Ok(8 * 1024 * 1024));
    assert_eq!(stack_plan(Some("2G"), PAGE_4K), Ok(2 * 1024 * 1024 * 1024));
    assert_eq!(stack_plan(Some("12m"), PAGE_4K), Ok(12 * 1024 * 1024));
    assert_eq!(stack_plan(Some("1g"), PAGE_4K), Ok(1024 * 1024 * 1024));
    assert_eq!(stack_plan(Some("4096k"), PAGE_4K), Ok(4 * 1024 * 1024));
    assert_eq!(stack_plan(Some(" 8M "), PAGE_4K), Ok(8 * 1024 * 1024));
}

#[test]
fn the_lower_bound_is_one_mib_inclusive() {
    assert_eq!(stack_plan(Some("1M"), PAGE_4K), Ok(1024 * 1024));
    assert_eq!(stack_plan(Some("1048576"), PAGE_4K), Ok(1024 * 1024));
    assert_eq!(
        stack_plan(Some("1048575"), PAGE_4K),
        Err(StackSizeError::OutOfRange("1048575".to_string()))
    );
    assert!(matches!(
        stack_plan(Some("0"), PAGE_4K),
        Err(StackSizeError::OutOfRange(_))
    ));
}

#[test]
fn the_upper_bound_is_sixteen_gib_inclusive() {
    let sixteen_gib = 16_u64 * 1024 * 1024 * 1024;
    assert_eq!(
        stack_plan(Some("16G"), PAGE_4K),
        Ok(usize::try_from(sixteen_gib).unwrap())
    );
    let one_past = (sixteen_gib + 1).to_string();
    assert_eq!(
        stack_plan(Some(&one_past), PAGE_4K),
        Err(StackSizeError::OutOfRange(one_past.clone()))
    );
    assert!(matches!(
        stack_plan(Some("17G"), PAGE_4K),
        Err(StackSizeError::OutOfRange(_))
    ));
}

#[test]
fn a_count_that_overflows_is_out_of_range_not_wrapped() {
    assert!(matches!(
        stack_plan(Some("18446744073709551615G"), PAGE_4K),
        Err(StackSizeError::OutOfRange(_))
    ));
    assert!(matches!(
        stack_plan(Some("99999999999999999999999"), PAGE_4K),
        Err(StackSizeError::OutOfRange(_))
    ));
}

#[test]
fn a_size_rounds_up_to_the_page() {
    let page_16k = 16 * 1024;
    // 1 MiB + 1 byte rounds to the next 16 KiB multiple.
    assert_eq!(
        stack_plan(Some("1048577"), page_16k),
        Ok(1024 * 1024 + page_16k)
    );
    assert_eq!(
        stack_plan(Some("1048577"), PAGE_4K),
        Ok(1024 * 1024 + PAGE_4K)
    );
    // Already a multiple: unchanged.
    assert_eq!(stack_plan(Some("8M"), page_16k), Ok(8 * 1024 * 1024));
    // An unknown page size leaves the count as written.
    assert_eq!(stack_plan(Some("1048577"), 0), Ok(1_048_577));
}

#[test]
fn a_malformed_value_is_an_error() {
    for bad in [
        "banana", "8MB", "M", "-8M", "8.5M", "8 M", "0x800000", "8T", "+8M",
    ] {
        assert_eq!(
            stack_plan(Some(bad), PAGE_4K),
            Err(StackSizeError::Malformed(bad.to_string())),
            "{bad}"
        );
    }
}

#[test]
fn the_refusal_line_names_the_variable_and_the_accepted_forms() {
    let line = StackSizeError::Malformed("banana".to_string()).to_string();
    assert!(line.contains("ALBITE_STACK_SIZE=\"banana\""), "{line}");
    assert!(line.contains("not recognised"), "{line}");
    assert!(line.contains("K, M or G"), "{line}");
    let line = StackSizeError::OutOfRange("17G".to_string()).to_string();
    assert!(line.contains("out of range"), "{line}");
    assert!(line.contains("from 1M to 16G"), "{line}");
}

#[test]
fn the_entry_asks_for_a_guard_of_at_least_one_mib() {
    assert_eq!(STACK_GUARD_BYTES, 1024 * 1024);
    let request = StackRequest::with_default_guard(8 * 1024 * 1024);
    assert_eq!(request.stack_bytes, 8 * 1024 * 1024);
    assert_eq!(request.guard_bytes, STACK_GUARD_BYTES);

    let mut seen = None;
    let exit_code = run_on_stack_with(request, body_returns_seven, |req, body| {
        seen = Some(req);
        Ok(body())
    });
    assert_eq!(exit_code, 7);
    assert_eq!(
        seen,
        Some(request),
        "the spawn receives the request unchanged"
    );
}

extern "C" fn body_returns_seven() -> i32 {
    7
}

static FALLBACK_BODY_RUNS: AtomicU64 = AtomicU64::new(0);

extern "C" fn fallback_body() -> i32 {
    FALLBACK_BODY_RUNS.fetch_add(1, Ordering::SeqCst);
    11
}

#[test]
fn a_failed_spawn_runs_the_body_on_the_current_thread() {
    let spawn_calls = Cell::new(0);
    let exit_code = run_on_stack_with(
        StackRequest::with_default_guard(DEFAULT_STACK_BYTES),
        fallback_body,
        |_, _| {
            spawn_calls.set(spawn_calls.get() + 1);
            Err("pthread_create: Resource temporarily unavailable".to_string())
        },
    );
    assert_eq!(exit_code, 11, "the fallback returns the body's own code");
    assert_eq!(spawn_calls.get(), 1);
    assert_eq!(
        FALLBACK_BODY_RUNS.load(Ordering::SeqCst),
        1,
        "the body ran exactly once"
    );
}

#[test]
fn the_fallback_line_names_the_size_and_the_error() {
    let line = fallback_line(StackRequest::with_default_guard(536_870_912), "EAGAIN");
    assert!(line.contains("536870912-byte stack"), "{line}");
    assert!(line.contains("(EAGAIN)"), "{line}");
    assert!(line.contains("current thread"), "{line}");
}

/// The thread the real-spawn body ran on, recorded by the body itself.
#[cfg(unix)]
static SPAWNED_BODY_THREAD: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
extern "C" fn body_records_its_thread() -> i32 {
    // SAFETY: pthread_self has no preconditions.
    let body_thread = unsafe { libc::pthread_self() } as u64;
    SPAWNED_BODY_THREAD.store(body_thread, Ordering::SeqCst);
    42
}

#[cfg(unix)]
#[test]
fn the_body_runs_on_a_new_thread_and_its_code_comes_back_through_the_join() {
    let request =
        StackRequest::with_default_guard(stack_plan(Some("8M"), platform::page_size()).unwrap());
    let exit_code = run_on_stack_with(request, body_records_its_thread, platform::spawn_and_join);
    assert_eq!(exit_code, 42);
    // SAFETY: as above.
    let caller = unsafe { libc::pthread_self() } as u64;
    let body_thread = SPAWNED_BODY_THREAD.load(Ordering::SeqCst);
    assert_ne!(body_thread, 0, "the body ran");
    assert_ne!(body_thread, caller, "the body ran on a spawned thread");
}

/// The non-unix platform arm (wasm, Windows until R6): a nominal 4 KiB page,
/// and the body runs where it is called, its exit code returned as the
/// spawn's.
#[test]
fn the_threadless_arm_assumes_a_4k_page_and_runs_the_body_in_place() {
    assert_eq!(nominal_page_size(), 4096);
    let request = StackRequest::with_default_guard(DEFAULT_STACK_BYTES);
    assert_eq!(run_on_current_thread(request, body_returns_seven), Ok(7));
}

/// ADR 19.10.26c D2: the `#[no_mangle]` entry itself, as the generated
/// `main` calls it — unset override, default size, the body's code back.
/// The harness never sets `ALBITE_STACK_SIZE`; a developer shell that does
/// still exercises the same path with a different size.
#[test]
fn the_runtime_entry_returns_the_bodys_exit_code() {
    if stack_plan(
        crate::settings::read(STACK_SIZE_ENV).as_deref(),
        platform::page_size(),
    )
    .is_err()
    {
        return; // a malformed override would exit 2 — not this test's subject
    }
    assert_eq!(__albite_run_on_stack(body_returns_seven), 7);
}

#[cfg(unix)]
#[test]
fn an_impossible_stack_size_is_a_spawn_error_not_a_crash() {
    // Below PTHREAD_STACK_MIN: pthread_attr_setstacksize refuses it.
    let request = StackRequest {
        stack_bytes: 1,
        guard_bytes: STACK_GUARD_BYTES,
    };
    let error = platform::spawn_and_join(request, body_returns_seven).unwrap_err();
    assert!(error.starts_with("pthread_attr_setstacksize"), "{error}");
}

#[cfg(unix)]
#[test]
fn the_page_size_is_known() {
    let page = platform::page_size();
    assert!(page >= 4096 && page.is_power_of_two(), "{page}");
}
