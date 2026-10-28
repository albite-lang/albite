//! The stack a compiled program's body runs on (ADR 19.10.26c).
//!
//! The generated `main` stores `argc`/`argv` and calls
//! [`__albite_run_on_stack`] with `__albite_inner_main`. The entry runs
//! that body on a fresh thread whose stack size the runtime chooses —
//! [`DEFAULT_STACK_BYTES`], or `ALBITE_STACK_SIZE` — so how deep a program
//! can recurse no longer depends on the `RLIMIT_STACK` of the shell that
//! started it. On Linux glibc sizes the main thread from that rlimit and never
//! reads the `PT_GNU_STACK` size a `-z stack-size` link flag writes, which is
//! why a link flag could not do this (D1).
//!
//! Raw `pthread` rather than `std::thread::Builder`: std cannot set the guard
//! size, and the guard is the point — `-O0` frames run to tens of KB and the
//! code emits no stack probes, so a one-page guard could be jumped clean over
//! (D2). The decision logic — [`stack_plan`] and [`run_on_stack_with`] — is
//! pure over injected data; only [`__albite_run_on_stack`] reads the
//! environment and spawns.

use std::fmt;

/// The stack a body runs on when `ALBITE_STACK_SIZE` is unset: 512 MiB.
///
/// 19.10.26b measured 10 MiB as the self-host's largest passing `-O0` limit;
/// four times that is under 512 MiB, so D3's floor applies. Under the default
/// overcommit policy this is reserved address space, committed only as it is
/// touched, so it costs no RSS.
pub const DEFAULT_STACK_BYTES: usize = 512 * MIB;

/// The guard below the spawned thread's stack: 1 MiB, matching the main
/// thread's kernel guard gap, so a large `-O0` frame faults in it rather than
/// landing in whatever is mapped beneath (D2).
pub const STACK_GUARD_BYTES: usize = MIB;

/// The smallest `ALBITE_STACK_SIZE` accepted: above glibc's aarch64
/// `PTHREAD_STACK_MIN` (128 KiB) and the overflow handler's 64 KiB window.
pub const MIN_STACK_BYTES: u64 = MIB as u64;

/// The largest `ALBITE_STACK_SIZE` accepted: 16 GiB.
pub const MAX_STACK_BYTES: u64 = 16 * GIB;

/// The setting that overrides [`DEFAULT_STACK_BYTES`], as a suffix read
/// through [`crate::settings`] (ADR 20.10.26e D1): `ALBITE_STACK_SIZE`.
pub const STACK_SIZE_ENV: &str = "STACK_SIZE";

const MIB: usize = 1024 * 1024;
const GIB: u64 = 1024 * 1024 * 1024;

/// Why a `ALBITE_STACK_SIZE` value was refused. Its `Display` is the one
/// line the program prints before exiting 2 (D3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StackSizeError {
    /// Not a byte count with an optional `K`, `M` or `G` suffix.
    Malformed(String),
    /// A well-formed count outside [`MIN_STACK_BYTES`]..=[`MAX_STACK_BYTES`].
    OutOfRange(String),
}

impl fmt::Display for StackSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, value) = match self {
            Self::Malformed(value) => ("not recognised", value),
            Self::OutOfRange(value) => ("out of range", value),
        };
        write!(
            f,
            "tungsten runtime: {}{STACK_SIZE_ENV}={value:?} {what} \
             (want a byte count with an optional K, M or G suffix, from 1M to 16G, e.g. 8M)",
            crate::settings::PREFIX
        )
    }
}

/// The stack size a `ALBITE_STACK_SIZE` value asks for, rounded up to
/// `page` (D3). Unset or blank is [`DEFAULT_STACK_BYTES`]; otherwise a byte
/// count with an optional `K`, `M` or `G` suffix (powers of 1024, either case)
/// inside [`MIN_STACK_BYTES`]..=[`MAX_STACK_BYTES`]. The bounds apply to the
/// value as written, before rounding.
///
/// # Errors
///
/// [`StackSizeError::Malformed`] for anything that is not such a count, and
/// [`StackSizeError::OutOfRange`] for a count outside the bounds.
pub fn stack_plan(value: Option<&str>, page: usize) -> Result<usize, StackSizeError> {
    let trimmed = value.map_or("", str::trim);
    if trimmed.is_empty() {
        return Ok(DEFAULT_STACK_BYTES);
    }
    let malformed = || StackSizeError::Malformed(trimmed.to_string());
    let (digits, multiplier) = match trimmed.as_bytes()[trimmed.len() - 1] {
        b'K' | b'k' => (&trimmed[..trimmed.len() - 1], 1024),
        b'M' | b'm' => (&trimmed[..trimmed.len() - 1], 1024 * 1024),
        b'G' | b'g' => (&trimmed[..trimmed.len() - 1], GIB),
        _ => (trimmed, 1),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    let out_of_range = || StackSizeError::OutOfRange(trimmed.to_string());
    let count: u64 = digits.parse().map_err(|_| out_of_range())?;
    let bytes = count.checked_mul(multiplier).ok_or_else(out_of_range)?;
    if !(MIN_STACK_BYTES..=MAX_STACK_BYTES).contains(&bytes) {
        return Err(out_of_range());
    }
    let bytes = usize::try_from(bytes).map_err(|_| out_of_range())?;
    Ok(round_up_to_page(bytes, page))
}

/// `bytes` rounded up to a multiple of `page`; a zero page leaves it as is.
/// macOS arm64 refuses a stack size that is not a 16 KiB multiple.
fn round_up_to_page(bytes: usize, page: usize) -> usize {
    if page == 0 {
        return bytes;
    }
    bytes.div_ceil(page) * page
}

/// What the entry asks the spawn for: a stack size and a guard size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackRequest {
    pub stack_bytes: usize,
    pub guard_bytes: usize,
}

impl StackRequest {
    /// A request for `stack_bytes` with the [`STACK_GUARD_BYTES`] guard.
    #[must_use]
    pub fn with_default_guard(stack_bytes: usize) -> Self {
        Self {
            stack_bytes,
            guard_bytes: STACK_GUARD_BYTES,
        }
    }
}

/// The body the generated `main` hands over: `__albite_inner_main`.
pub type ProgramBody = extern "C" fn() -> i32;

/// Run `body` on a thread `spawn` creates for `request`, and return its exit
/// code. If the spawn fails, print one line naming the size and the error,
/// then run `body` on the current thread — at the shell's limit, as before
/// this entry existed (D4). `spawn` is injected so tests can fail it.
pub fn run_on_stack_with(
    request: StackRequest,
    body: ProgramBody,
    spawn: impl FnOnce(StackRequest, ProgramBody) -> Result<i32, String>,
) -> i32 {
    match spawn(request, body) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{}", fallback_line(request, &error));
            body()
        }
    }
}

/// The line a failed spawn prints before the body runs on the current thread.
fn fallback_line(request: StackRequest, error: &str) -> String {
    format!(
        "tungsten runtime: could not start the program on a {}-byte stack ({error}); \
         running on the current thread's stack instead",
        request.stack_bytes
    )
}

/// The runtime entry the generated `main` calls (ADR 19.10.26c D2): read
/// `ALBITE_STACK_SIZE`, then run `body` on a thread of that size. A refused
/// value prints [`StackSizeError`]'s line and exits 2 before the body runs.
#[no_mangle]
pub extern "C" fn __albite_run_on_stack(body: ProgramBody) -> i32 {
    let setting = crate::settings::read(STACK_SIZE_ENV);
    match stack_plan(setting.as_deref(), platform::page_size()) {
        Ok(stack_bytes) => run_on_stack_with(
            StackRequest::with_default_guard(stack_bytes),
            body,
            platform::spawn_and_join,
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2)
        }
    }
}

/// The `pthread` half: page size, and spawn-then-join with an explicit stack
/// and guard. Every pthread name here is already in the link's
/// `RESERVED_LINK_NAMES`, and both link lines pass `-lpthread`.
#[cfg(all(unix, target_os = "linux"))]
pub(crate) use platform::pthread_attr_getguardsize;

#[cfg(unix)]
mod platform {
    use super::{ProgramBody, StackRequest};
    use core::ffi::c_void;

    // POSIX, in glibc and libSystem alike, but the `libc` crate declares the
    // pair for neither Linux nor Apple targets.
    extern "C" {
        fn pthread_attr_setguardsize(attr: *mut libc::pthread_attr_t, guard: usize) -> i32;
        // Read back only where `pthread_getattr_np` exists (the handler, Linux).
        #[cfg(target_os = "linux")]
        pub(crate) fn pthread_attr_getguardsize(
            attr: *const libc::pthread_attr_t,
            guard: *mut usize,
        ) -> i32;
    }

    pub(super) fn page_size() -> usize {
        // SAFETY: sysconf reads a constant; -1 (unknown) maps to "no rounding".
        usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(0)
    }

    /// What the trampoline reads and writes across the thread boundary.
    struct Handoff {
        body: ProgramBody,
        exit_code: i32,
    }

    extern "C" fn trampoline(handoff: *mut c_void) -> *mut c_void {
        // SAFETY: `handoff` is the `&mut Handoff` spawn_and_join passed, which
        // outlives this thread because spawn_and_join joins it before returning.
        let handoff = unsafe { &mut *handoff.cast::<Handoff>() };
        handoff.exit_code = (handoff.body)();
        core::ptr::null_mut()
    }

    /// `name` failed with `code`, as one phrase for the fallback line.
    fn failed(name: &str, code: i32) -> String {
        format!("{name}: {}", std::io::Error::from_raw_os_error(code))
    }

    pub(super) fn spawn_and_join(request: StackRequest, body: ProgramBody) -> Result<i32, String> {
        let mut handoff = Handoff { body, exit_code: 0 };
        // SAFETY: the attr is initialised before use and destroyed on every
        // path out; the thread is joined before `handoff` goes out of scope.
        unsafe {
            let mut attr: libc::pthread_attr_t = core::mem::zeroed();
            let code = libc::pthread_attr_init(&raw mut attr);
            if code != 0 {
                return Err(failed("pthread_attr_init", code));
            }
            let spawned = spawn_with_attr(&raw mut attr, request, &raw mut handoff);
            libc::pthread_attr_destroy(&raw mut attr);
            let thread = spawned?;
            let code = libc::pthread_join(thread, core::ptr::null_mut());
            if code != 0 {
                // The body has run or is running; running it again would
                // repeat its effects, so this is fatal rather than a fallback.
                eprintln!("tungsten runtime: {}", failed("pthread_join", code));
                std::process::abort();
            }
        }
        Ok(handoff.exit_code)
    }

    unsafe fn spawn_with_attr(
        attr: *mut libc::pthread_attr_t,
        request: StackRequest,
        handoff: *mut Handoff,
    ) -> Result<libc::pthread_t, String> {
        let code = libc::pthread_attr_setstacksize(attr, request.stack_bytes);
        if code != 0 {
            return Err(failed("pthread_attr_setstacksize", code));
        }
        let code = pthread_attr_setguardsize(attr, request.guard_bytes);
        if code != 0 {
            return Err(failed("pthread_attr_setguardsize", code));
        }
        let mut thread: libc::pthread_t = core::mem::zeroed();
        let code = libc::pthread_create(&raw mut thread, attr, trampoline, handoff.cast());
        if code != 0 {
            return Err(failed("pthread_create", code));
        }
        Ok(thread)
    }
}

/// The page size where the runtime cannot ask the OS: a nominal 4 KiB. The
/// arm that uses it never spawns, so the rounding it drives only tidies the
/// size a refusal or fallback line would name. Defined outside the
/// `not(unix)` arm that uses it, so a host test reaches (and a mutation sweep
/// on a unix host can kill) its body.
#[cfg_attr(unix, allow(dead_code))] // Reason: only the not(unix) platform arm calls it; its test runs on every host
fn nominal_page_size() -> usize {
    4096
}

/// Run the body on the current thread and report its exit code: the spawn
/// where the runtime targets no thread API. Outside the `not(unix)` arm for
/// the same reason as [`nominal_page_size`].
#[cfg_attr(unix, allow(dead_code))] // Reason: only the not(unix) platform arm calls it; its test runs on every host
#[allow(clippy::unnecessary_wraps)] // Reason: it stands in for spawn_and_join, whose signature the seam fixes
fn run_on_current_thread(_request: StackRequest, body: ProgramBody) -> Result<i32, String> {
    Ok(body())
}

/// No thread API the runtime targets here (wasm, and Windows until R6 ports
/// it — Non-Goals): the body runs on the current thread, as the fallback does.
#[cfg(not(unix))]
mod platform {
    pub(super) use super::nominal_page_size as page_size;
    pub(super) use super::run_on_current_thread as spawn_and_join;
}

#[cfg(test)]
mod tests;
