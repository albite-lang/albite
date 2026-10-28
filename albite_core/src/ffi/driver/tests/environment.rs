//! Environment FFI: the executable's directory and environment variables.

use std::ffi::{c_char, CStr};
use std::path::PathBuf;

use crate::ffi::driver::environment::{exe_dir_cstring, tg_current_exe_dir, tg_getenv};
use crate::ffi::driver::{tg_driver_get_last_error, tg_free_string};

/// Take ownership of an FFI string and return it as a Rust `String`.
fn take_ffi_string(raw: *mut c_char) -> String {
    assert!(!raw.is_null(), "expected a string, got null");
    let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_owned();
    tg_free_string(raw);
    text
}

#[test]
fn exe_dir_is_the_parent_of_the_executable_path() {
    let dir = exe_dir_cstring(Ok(PathBuf::from("/opt/tungsten/bin/tungsten1")));
    assert_eq!(take_ffi_string(dir), "/opt/tungsten/bin");
    assert!(
        tg_driver_get_last_error().is_null(),
        "success leaves no error"
    );
}

#[test]
fn exe_dir_without_a_parent_is_null_with_an_error() {
    assert!(exe_dir_cstring(Ok(PathBuf::from("/"))).is_null());
    assert_eq!(
        take_ffi_string(tg_driver_get_last_error()),
        "current executable has no parent directory"
    );
}

#[test]
fn exe_dir_lookup_failure_is_null_with_the_cause() {
    let failure = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
    assert!(exe_dir_cstring(Err(failure)).is_null());
    assert_eq!(
        take_ffi_string(tg_driver_get_last_error()),
        "failed to get current executable path: gone"
    );
}

#[test]
fn current_exe_dir_names_the_test_binarys_directory() {
    let expected = std::env::current_exe().unwrap();
    let expected = expected.parent().unwrap().to_string_lossy().into_owned();
    assert_eq!(take_ffi_string(tg_current_exe_dir()), expected);
}

#[test]
fn getenv_refuses_a_null_name_and_a_non_utf8_name() {
    assert!(tg_getenv(std::ptr::null()).is_null());
    let not_utf8 = [0xff_u8, 0xfe, 0];
    assert!(tg_getenv(not_utf8.as_ptr().cast::<c_char>()).is_null());
}
