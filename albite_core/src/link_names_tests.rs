//! ADR 9.10.26f AC2: the reserved link-name table is well-formed.

use super::{is_reserved_link_name, RESERVED_LINK_NAMES, RESERVED_LINK_PREFIXES};

// 9.10.26f AC2: sorted with no duplicates, so `binary_search` is sound.
#[test]
fn the_table_is_sorted_without_duplicates() {
    for pair in RESERVED_LINK_NAMES.windows(2) {
        assert!(
            pair[0] < pair[1],
            "{} must sort before {}",
            pair[0],
            pair[1]
        );
    }
}

// 9.10.26f AC2: an entry a prefix already covers is dead weight, and the
// usual way to write one is to forget to strip macOS's leading `_`.
#[test]
fn no_entry_is_already_covered_by_a_prefix_rule() {
    for name in RESERVED_LINK_NAMES {
        let covered = RESERVED_LINK_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
            || super::reserved_by_c_underscore_rule(name);
        assert!(!covered, "{name} is covered by a prefix rule");
        assert!(
            !name.contains('$') && !name.contains('@'),
            "{name} keeps a variant or version suffix"
        );
    }
}

// 9.10.26f AC2: the code generator's and native core's own libc names.
#[test]
fn the_codegen_and_native_core_names_are_reserved() {
    for name in [
        "write",
        "abort",
        "malloc",
        "memcpy",
        "printf",
        "albite_main",
    ] {
        assert!(is_reserved_link_name(name), "{name}");
    }
}

// 9.10.26f AC2: an ordinary program name is not reserved, nor is `main`,
// which `def_llvm_name` already renames.
#[test]
fn an_ordinary_name_is_not_reserved() {
    for name in [
        "helper",
        "main",
        "writer",
        "write_decimal",
        "_harness_run_tests",
        "_",
        "tg",
    ] {
        assert!(!is_reserved_link_name(name), "{name}");
    }
}

// 9.10.26f AC2: the toolchain's prefixes and C's `_` + uppercase rule.
#[test]
fn the_prefix_rules_match() {
    for name in ["tg_x", "__albite_x", "__wrap_x", "_Exit", "_Unwind_Resume"] {
        assert!(is_reserved_link_name(name), "{name}");
    }
    assert!(!super::reserved_by_c_underscore_rule("_exit"));
    assert!(!super::reserved_by_c_underscore_rule("Exit"));
}

// 20.10.26g AC4: the rename keeps both spellings of the entry symbol and of
// the runtime's hook prefix reserved through 2.0.x, so a program cannot
// define either the new name or the one an older archive still exports.
#[test]
fn both_spellings_of_the_renamed_runtime_names_are_reserved() {
    for name in ["albite_main", "tungsten_main", "__albite_x", "__tungsten_x"] {
        assert!(is_reserved_link_name(name), "{name}");
    }
    assert!(RESERVED_LINK_NAMES.contains(&"albite_main"));
    assert!(RESERVED_LINK_NAMES.contains(&"tungsten_main"));
}
