//! Environment queries: the running executable's directory and environment
//! variables.
//!
//! Split out of `console.rs` when `tg_exit` gained its allocation-report branch
//! (ADR 30.9.26b) and that file had six lines of headroom left. Both functions
//! answer "what is around this process", not "talk to the terminal".

use std::ffi::{c_char, CStr, CString};
use std::ptr;

use super::{clear_driver_error, set_driver_error};

// ============================================================================
// Current Executable Directory
// ============================================================================

/// Get the directory containing the current executable.
///
/// Returns the directory path as a null-terminated C string (caller must free
/// with `tg_free_string`), or null if the path cannot be determined.
#[no_mangle]
pub extern "C" fn tg_current_exe_dir() -> *mut c_char {
    exe_dir_cstring(std::env::current_exe())
}

/// The pure half of [`tg_current_exe_dir`], over an injected `current_exe()`
/// result, so the no-parent and lookup-failure arms are testable.
pub(super) fn exe_dir_cstring(exe: std::io::Result<std::path::PathBuf>) -> *mut c_char {
    clear_driver_error();

    match exe {
        Ok(exe_path) => {
            if let Some(dir) = exe_path.parent() {
                let dir_str = dir.to_string_lossy();
                CString::new(dir_str.as_ref())
                    .map(std::ffi::CString::into_raw)
                    .unwrap_or(ptr::null_mut())
            } else {
                set_driver_error("current executable has no parent directory");
                ptr::null_mut()
            }
        }
        Err(e) => {
            set_driver_error(format!("failed to get current executable path: {e}"));
            ptr::null_mut()
        }
    }
}

// ============================================================================
// Environment Variables
// ============================================================================

/// Get the value of an environment variable.
///
/// Returns the value as a null-terminated C string (caller must free with
/// `tg_free_string`), or null if the variable is not set.
#[no_mangle]
pub extern "C" fn tg_getenv(name: *const c_char) -> *mut c_char {
    clear_driver_error();

    if name.is_null() {
        return ptr::null_mut();
    }

    let name_str = match unsafe { CStr::from_ptr(name) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    match std::env::var(name_str) {
        Ok(val) => CString::new(val)
            .map(std::ffi::CString::into_raw)
            .unwrap_or(ptr::null_mut()),
        Err(_) => ptr::null_mut(),
    }
}
