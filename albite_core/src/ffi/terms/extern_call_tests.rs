//! ADR 3.10.26q AC 1: `tg_term_extern_call` and `tg_term_extern_call_arg`
//! build the bootstrap's extern-wrapper body, one appended argument at a time.

use std::ffi::CString;

use super::core::tg_term_var_named;
use super::core_data::tg_term_zero;
use super::ext::{tg_term_extern_call, tg_term_extern_call_arg};
use super::nodes::materialize_term;
use crate::ffi::{tg_init, TermHandle, INVALID_HANDLE};
use crate::terms::Term;

fn extern_call(symbol: &str) -> TermHandle {
    let symbol = CString::new(symbol).expect("no interior NUL");
    // SAFETY: `symbol` is a valid NUL-terminated string that outlives the call.
    unsafe { tg_term_extern_call(symbol.as_ptr()) }
}

fn var(name: &str) -> TermHandle {
    let name = CString::new(name).expect("no interior NUL");
    // SAFETY: as above.
    unsafe { tg_term_var_named(name.as_ptr()) }
}

fn materialized(handle: TermHandle) -> Term {
    crate::ffi::with_arena_ref!(|arena| materialize_term(arena, handle).expect("valid handle"))
}

// 3.10.26q AC1: two appended arguments materialise as the bootstrap's term.
#[test]
fn appended_arguments_build_the_bootstraps_extern_call() {
    tg_init();
    let empty = extern_call("__c_g");
    let one = tg_term_extern_call_arg(empty, var("x"));
    let two = tg_term_extern_call_arg(one, var("y"));
    assert_eq!(
        materialized(two),
        Term::extern_call("__c_g", vec![Term::var("x"), Term::var("y")])
    );
    assert_eq!(
        materialized(one),
        Term::extern_call("__c_g", vec![Term::var("x")])
    );
}

// 3.10.26q AC1 / D2: appending allocates a new node; the first handle still
// names the argument-less call.
#[test]
fn appending_never_changes_an_earlier_handle() {
    tg_init();
    let empty = extern_call("__c_g");
    let extended = tg_term_extern_call_arg(empty, var("x"));
    assert_ne!(extended, empty);
    assert_eq!(materialized(empty), Term::extern_call("__c_g", Vec::new()));
}

// 3.10.26q AC1: a null symbol, an invalid argument and a call that is not an
// `ExternCall` each refuse.
#[test]
fn malformed_inputs_return_the_invalid_handle() {
    tg_init();
    // SAFETY: a null pointer is the case under test; it is never dereferenced.
    assert_eq!(
        unsafe { tg_term_extern_call(std::ptr::null()) },
        INVALID_HANDLE
    );
    let non_utf8 = [0xffu8, 0x00];
    // SAFETY: NUL-terminated, deliberately not UTF-8.
    assert_eq!(
        unsafe { tg_term_extern_call(non_utf8.as_ptr().cast()) },
        INVALID_HANDLE
    );
    let call = extern_call("__c_g");
    assert_eq!(
        tg_term_extern_call_arg(call, INVALID_HANDLE),
        INVALID_HANDLE
    );
    assert_eq!(
        tg_term_extern_call_arg(INVALID_HANDLE, var("x")),
        INVALID_HANDLE
    );
    assert_eq!(
        tg_term_extern_call_arg(tg_term_zero(), var("x")),
        INVALID_HANDLE,
        "only a foreign call takes an argument"
    );
}
