//! Settings read from the environment: `ALBITE_<suffix>` first, then the
//! legacy `TUNGSTEN_<suffix>` (ADR 20.10.26e D1, D2).
//!
//! The one place a setting's spelling is decided. It lives in the runtime
//! because compiled programs read settings too, and the runtime is the one
//! crate every compiler crate already reaches; it depends only on `libc`.
//!
//! [`resolve`] is pure over an injected lookup, so the order is tested
//! against an in-memory map. [`read`] is the only function here that touches
//! the process environment or stderr: on a legacy hit it prints
//! [`legacy_notice`] once per suffix per process.
//!
//! A caller names a setting by its **suffix** (`"ARENA"`, `"STACK_SIZE"`);
//! [`name`] spells it for a setter, so no `TUNGSTEN_` literal lives outside
//! this module.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::sync::{Mutex, OnceLock};

/// The prefix a setting is read under first, and the one setters write.
pub const PREFIX: &str = "ALBITE_";

/// The prefix still accepted this release, with a one-time notice.
pub const LEGACY_PREFIX: &str = "TUNGSTEN_";

/// A resolved setting: its value, and whether it came from the legacy name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub value: String,
    pub legacy: bool,
}

/// The current spelling of a setting: `ALBITE_<suffix>`.
#[must_use]
pub fn name(suffix: &str) -> String {
    format!("{PREFIX}{suffix}")
}

/// The legacy spelling of a setting: `TUNGSTEN_<suffix>`.
#[must_use]
pub fn legacy_name(suffix: &str) -> String {
    format!("{LEGACY_PREFIX}{suffix}")
}

/// Resolve a setting through `lookup`: the current name, then the legacy one.
///
/// An empty value counts as unset for both names, so an empty current name
/// never shadows a set legacy one — the order the make and shell idioms
/// (`$(or …)`, `${…:-…}`) produce too.
pub fn resolve(suffix: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<Setting> {
    let non_empty = |key: &str| lookup(key).filter(|value| !value.is_empty());
    if let Some(value) = non_empty(&name(suffix)) {
        return Some(Setting {
            value,
            legacy: false,
        });
    }
    non_empty(&legacy_name(suffix)).map(|value| Setting {
        value,
        legacy: true,
    })
}

/// Whether this is the first legacy use of `suffix` recorded in `warned`
/// (D2). Records it, so a second call with the same suffix answers `false`.
pub fn first_warning<S: BuildHasher>(warned: &mut HashSet<String, S>, suffix: &str) -> bool {
    warned.insert(suffix.to_owned())
}

/// The one line printed for a legacy name, in the runtime's message voice.
#[must_use]
pub fn legacy_notice(suffix: &str) -> String {
    format!(
        "tungsten runtime: {} is now {}; the old name is removed in 2.1",
        legacy_name(suffix),
        name(suffix)
    )
}

/// Read a setting from the process environment, current name first.
///
/// Values are read with `var_os` and converted lossily, so a non-UTF-8 value
/// is read as it always was rather than falling through to the legacy name.
/// A legacy hit prints [`legacy_notice`] to stderr once per suffix.
#[must_use]
pub fn read(suffix: &str) -> Option<String> {
    let setting = resolve(suffix, |key| {
        std::env::var_os(key).map(|value| value.to_string_lossy().into_owned())
    })?;
    if setting.legacy {
        warn_once(suffix);
    }
    Some(setting.value)
}

/// Print the legacy notice for `suffix` unless this process already has.
fn warn_once(suffix: &str) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut warned = WARNED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if first_warning(&mut warned, suffix) {
        eprintln!("{}", legacy_notice(suffix));
    }
}

#[cfg(test)]
mod tests;
