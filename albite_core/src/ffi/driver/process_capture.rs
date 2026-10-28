//! Subprocess execution with stdout captured to a file (ADR 1.10.26i D2).
//!
//! `tg_exec_process` inherits both streams, so a generated test harness's
//! output goes straight to the terminal and nothing comes back but an exit
//! code. In a machine format the driver needs the harness's frames, so this
//! twin sends the child's stdout to a file the driver then reads, and leaves
//! stderr inherited. Its own file because `console.rs` is near its size cap.

use std::ffi::{c_char, CStr};
use std::fs::File;
use std::process::{Command, Stdio};

use super::{clear_driver_error, set_driver_error};

/// Execute `program` with its stdout written to `stdout_path`.
///
/// `args` is newline-delimited, as for `tg_exec_process`. Returns the exit
/// code, or -1 if the file cannot be created or the process cannot be spawned
/// (call `tg_driver_get_last_error` for the reason). A child killed by a
/// signal also reads -1, as there.
#[no_mangle]
pub extern "C" fn tg_exec_process_capture(
    program: *const c_char,
    args: *const c_char,
    stdout_path: *const c_char,
) -> i64 {
    clear_driver_error();
    let Some(program) = c_str_arg(program, "program path") else {
        return -1;
    };
    let Some(stdout_path) = c_str_arg(stdout_path, "stdout path") else {
        return -1;
    };
    let file = match File::create(stdout_path) {
        Ok(file) => file,
        Err(e) => {
            set_driver_error(format!("failed to create '{stdout_path}': {e}"));
            return -1;
        }
    };
    let mut cmd = Command::new(program);
    cmd.args(split_args(args)).stdout(Stdio::from(file));
    match cmd.status() {
        Ok(status) => i64::from(status.code().unwrap_or(-1)),
        Err(e) => {
            set_driver_error(format!("failed to execute '{program}': {e}"));
            -1
        }
    }
}

/// A required C-string argument, or the driver error naming it.
fn c_str_arg<'a>(ptr: *const c_char, what: &str) -> Option<&'a str> {
    if ptr.is_null() {
        set_driver_error(format!("{what} is null"));
        return None;
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Some(s),
        Err(e) => {
            set_driver_error(format!("invalid {what}: {e}"));
            None
        }
    }
}

/// The newline-delimited argument list, empty lines dropped.
fn split_args(args: *const c_char) -> Vec<String> {
    if args.is_null() {
        return Vec::new();
    }
    unsafe { CStr::from_ptr(args) }.to_str().map_or_else(
        |_| Vec::new(),
        |text| {
            text.split('\n')
                .filter(|arg| !arg.is_empty())
                .map(str::to_string)
                .collect()
        },
    )
}
