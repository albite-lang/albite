//! Test-only view of the error channels before assembly (ADR 9.10.26h AC2).
//!
//! `take_accumulated_errors_ordered` folds the pre-walk list into the module
//! groups and drops duplicates, so the list it returns cannot say which
//! channel a record came from. A test asserting "no module but the defining
//! one re-raises" needs the channels themselves; this captures them on the
//! calling thread, which is the thread that drives the walk's verdict.

use std::cell::RefCell;

use crate::elaborate::ElabError;

/// The two channels, as `take_accumulated_errors_ordered` received them.
#[derive(Default, Clone)]
pub(crate) struct ErrorChannels {
    pub(crate) pre_walk: Vec<ElabError>,
    pub(crate) modules: Vec<(Vec<String>, Vec<ElabError>)>,
}

thread_local! {
    static LAST: RefCell<Option<ErrorChannels>> = const { RefCell::new(None) };
}

pub(super) fn record(pre_walk: &[ElabError], modules: &[(Vec<String>, Vec<ElabError>)]) {
    LAST.with(|last| {
        *last.borrow_mut() = Some(ErrorChannels {
            pre_walk: pre_walk.to_vec(),
            modules: modules.to_vec(),
        });
    });
}

/// The channels the last assembly on this thread saw, clearing them.
pub(crate) fn take_last() -> Option<ErrorChannels> {
    LAST.with(|last| last.borrow_mut().take())
}
