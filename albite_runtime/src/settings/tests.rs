//! ADR 20.10.26e AC1, AC2: the resolution order and the once-per-name notice,
//! over an in-memory environment.

use std::collections::{HashMap, HashSet};

use super::{first_warning, legacy_name, legacy_notice, name, read, resolve, Setting};

fn lookup_in(env: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = env
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

fn current(value: &str) -> Option<Setting> {
    Some(Setting {
        value: value.to_string(),
        legacy: false,
    })
}

fn legacy(value: &str) -> Option<Setting> {
    Some(Setting {
        value: value.to_string(),
        legacy: true,
    })
}

// 20.10.26e AC1 — only the new name: the new value, not legacy.
#[test]
fn only_the_new_name_resolves_to_it() {
    let env = [("ALBITE_ARENA", "bump")];
    assert_eq!(resolve("ARENA", lookup_in(&env)), current("bump"));
}

// 20.10.26e AC1 — only the old name: the old value, legacy.
#[test]
fn only_the_old_name_resolves_to_it_as_legacy() {
    let env = [("TUNGSTEN_ARENA", "bump:2")];
    assert_eq!(resolve("ARENA", lookup_in(&env)), legacy("bump:2"));
}

// 20.10.26e AC1 — both names: the new one wins.
#[test]
fn both_names_resolve_to_the_new_one() {
    let env = [("ALBITE_ARENA", "off"), ("TUNGSTEN_ARENA", "bump")];
    assert_eq!(resolve("ARENA", lookup_in(&env)), current("off"));
}

// 20.10.26e AC1 — neither name: unset.
#[test]
fn neither_name_is_unset() {
    let env = [("ALBITE_OTHER", "1"), ("TUNGSTEN_OTHER", "1")];
    assert_eq!(resolve("ARENA", lookup_in(&env)), None);
}

// 20.10.26e AC1 — an empty new name never shadows a set old one.
#[test]
fn an_empty_new_name_falls_through_to_the_old_one() {
    let env = [("ALBITE_ARENA", ""), ("TUNGSTEN_ARENA", "bump")];
    assert_eq!(resolve("ARENA", lookup_in(&env)), legacy("bump"));
}

// 20.10.26e AC1 — both empty: unset.
#[test]
fn both_names_empty_is_unset() {
    let env = [("ALBITE_ARENA", ""), ("TUNGSTEN_ARENA", "")];
    assert_eq!(resolve("ARENA", lookup_in(&env)), None);
}

// 20.10.26e AC2 — the notice is printed once per name.
#[test]
fn first_warning_fires_once_per_suffix() {
    let mut warned = HashSet::new();
    assert!(first_warning(&mut warned, "ARENA"));
    assert!(!first_warning(&mut warned, "ARENA"));
    assert!(first_warning(&mut warned, "STACK_SIZE"));
}

#[test]
fn names_are_spelled_from_the_suffix() {
    assert_eq!(name("STACK_SIZE"), "ALBITE_STACK_SIZE");
    assert_eq!(legacy_name("STACK_SIZE"), "TUNGSTEN_STACK_SIZE");
}

#[test]
fn the_notice_names_both_spellings_in_the_runtime_voice() {
    assert_eq!(
        legacy_notice("ARENA"),
        "tungsten runtime: TUNGSTEN_ARENA is now ALBITE_ARENA; the old name is removed in 2.1"
    );
}

/// `read` over the real environment, with suffixes no other test or setting
/// uses, so mutating the process environment races nothing.
#[test]
fn read_applies_the_order_to_the_process_environment() {
    // SAFETY: these names are private to this test.
    unsafe {
        std::env::set_var("TUNGSTEN_SETTINGS_TEST_READ_LEGACY", "old");
        std::env::set_var("ALBITE_SETTINGS_TEST_READ_CURRENT", "new");
        std::env::set_var("TUNGSTEN_SETTINGS_TEST_READ_CURRENT", "old");
    }
    assert_eq!(read("SETTINGS_TEST_READ_LEGACY").as_deref(), Some("old"));
    assert_eq!(read("SETTINGS_TEST_READ_CURRENT").as_deref(), Some("new"));
    assert_eq!(read("SETTINGS_TEST_READ_UNSET"), None);
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("TUNGSTEN_SETTINGS_TEST_READ_LEGACY");
        std::env::remove_var("ALBITE_SETTINGS_TEST_READ_CURRENT");
        std::env::remove_var("TUNGSTEN_SETTINGS_TEST_READ_CURRENT");
    }
}
