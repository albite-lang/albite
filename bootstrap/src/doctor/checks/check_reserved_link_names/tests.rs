//! ADR 9.10.26f AC5: the reserved-names gate reads both hosts' `nm` and
//! names what the table lacks.

use super::{parse_nm, run_nm, ReservedNamesCensus, SymbolTable};

// 9.10.26f AC5: a two-field line is an import only when its type letter is
// an undefined one; anything else is neither import nor definition.
#[test]
fn a_two_field_line_is_an_import_only_for_an_undefined_type() {
    let table = parse_nm("T _defined_without_address\nU _write\n", true);
    let undefined: Vec<&str> = table.undefined.iter().map(String::as_str).collect();
    assert_eq!(undefined, ["write"]);
    assert!(table.defined.is_empty());
}

// 9.10.26f AC5: the seam refuses a file it cannot read, rather than handing
// the census an empty table that would read as "nothing imported".
#[test]
fn the_nm_seam_fails_on_a_missing_file() {
    let missing = std::path::Path::new("/nonexistent/9.10.26f/no-such-archive.a");
    assert!(run_nm(missing).is_err());
}

/// Apple `nm` over an archive: member headers, Mach-O's `_` prefix, a
/// `$VARIANT` suffix, a local symbol and a member-to-member call.
const MACOS_ARCHIVE: &str = "
libalbite_core.a(a.o):
0000000000000000 T _tg_int_trap
0000000000000010 t _local_helper
                 U _write
                 U _realpath$DARWIN_EXTSN
                 U _tg_shared

libalbite_core.a(b.o):
0000000000000000 T _tg_shared
                 U _abort
";

/// GNU `nm` over a linked binary: glibc version suffixes, a weak import and
/// the program's own definitions.
const LINUX_BINARY: &str = "
0000000000001139 T main
0000000000001200 T _tg_5write
                 U write@GLIBC_2.2.5
                 U malloc@@GLIBC_2.2.5
                 w __gmon_start__
0000000000004010 B __bss_start
";

#[test]
fn macos_archive_is_parsed_in_c_spelling() {
    let table = parse_nm(MACOS_ARCHIVE, true);
    let undefined: Vec<&str> = table.undefined.iter().map(String::as_str).collect();
    assert_eq!(undefined, ["abort", "realpath", "tg_shared", "write"]);
    let defined: Vec<&str> = table.defined.iter().map(String::as_str).collect();
    assert_eq!(defined, ["tg_int_trap", "tg_shared"]);
}

#[test]
fn linux_binary_keeps_its_spelling_and_drops_versions() {
    let table = parse_nm(LINUX_BINARY, false);
    let undefined: Vec<&str> = table.undefined.iter().map(String::as_str).collect();
    assert_eq!(undefined, ["__gmon_start__", "malloc", "write"]);
    assert!(table.defined.contains("_tg_5write"));
    assert!(table.defined.contains("__bss_start"));
}

#[test]
fn a_bare_name_is_an_import_and_a_header_is_not() {
    let table = parse_nm("x.o:\n_write\n", true);
    assert_eq!(table.undefined.len(), 1);
    assert!(table.undefined.contains("write"));
    assert!(table.defined.is_empty());
}

#[test]
fn an_all_reserved_census_is_clean() {
    let census = ReservedNamesCensus::from_tables(&[
        ("core.a".to_string(), parse_nm(MACOS_ARCHIVE, true)),
        ("probe".to_string(), parse_nm(LINUX_BINARY, false)),
    ]);
    // tg_shared is defined by a sibling member, so it is not an import.
    assert_eq!(census.inputs, 2);
    assert_eq!(census.imported, 5);
    assert!(census.unreserved.is_empty(), "{:?}", census.unreserved);
    assert_eq!(census.exit_status(), 0);
    assert_eq!(
        census.render(),
        "✓ reserved link names: 2 input(s), 5 imported C name(s), all reserved\n"
    );
}

// 9.10.26f AC5: a name the table lacks is reported by name and by file.
#[test]
fn a_missing_name_is_reported_by_name_and_file() {
    let text = "                 U _write\n                 U _frobnicate\n";
    let census = ReservedNamesCensus::from_tables(&[
        ("a.o".to_string(), parse_nm(text, true)),
        ("b.o".to_string(), parse_nm("U _frobnicate\n", true)),
    ]);
    assert_eq!(census.exit_status(), 2);
    assert_eq!(
        census.unreserved.get("frobnicate"),
        Some(&vec!["a.o".to_string(), "b.o".to_string()])
    );
    assert_eq!(census.unreserved.len(), 1);
    let report = census.render();
    assert!(
        report.starts_with(
            "✗ reserved link names: 2 input(s), 2 imported C name(s), 1 not reserved\n"
        ),
        "{report}"
    );
    assert!(
        report.contains("  frobnicate — imported by a.o, b.o\n"),
        "{report}"
    );
    assert!(report.contains("RESERVED_LINK_NAMES"), "{report}");
}

// A census over inputs that import nothing checked nothing: a hard failure,
// never a green.
#[test]
fn nothing_imported_is_a_hard_failure() {
    let census =
        ReservedNamesCensus::from_tables(&[("empty.o".to_string(), SymbolTable::default())]);
    assert_eq!(census.imported, 0);
    assert_eq!(census.exit_status(), 1);
    assert!(
        census.render().starts_with("error: "),
        "{}",
        census.render()
    );
    let none = ReservedNamesCensus::from_tables(&[]);
    assert_eq!(none.exit_status(), 1);
}
