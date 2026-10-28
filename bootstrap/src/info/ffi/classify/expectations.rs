//! The reviewed disagreements between `--classify` and 1.10.26k's record
//! (ADR 5.10.26f D3).
//!
//! The oracle test fails on any disagreement not listed here, and on any row
//! listed here that now agrees, so the table only shrinks by being edited.
//! Its length is pinned by [`PINNED_DISAGREEMENTS`]: a new row must raise the
//! constant in the same commit, which makes a new exemption a visible
//! decision. Neither the record nor the rule is edited to make them agree.

use super::Class;

/// One reviewed disagreement.
pub(crate) struct Expectation {
    pub(crate) symbol: &'static str,
    /// The record's class.
    pub(crate) record: Class,
    /// The rule's class; `None` when the symbol no longer resolves to an
    /// item. Equal to `record` when only the basis differs.
    pub(crate) rule: Option<Class>,
    /// Why, in one line.
    pub(crate) cause: &'static str,
}

/// The table's length, raised in the same commit as any new row.
pub(crate) const PINNED_DISAGREEMENTS: usize = 11;

pub(crate) const EXPECTED_DISAGREEMENTS: [Expectation; PINNED_DISAGREEMENTS] = [
    Expectation {
        symbol: "tg_type_prop",
        record: Class::Kernel,
        rule: None,
        cause: "removed with the `Prop` type by 4.10.26i (FFI tag 5 retired)",
    },
    Expectation {
        symbol: "__albite_alloc_profile_malloc",
        record: Class::Runtime,
        rule: Some(Class::Pure),
        cause: "its body only calls `__albite_alloc`, the shared allocator D2 does not follow",
    },
    Expectation {
        symbol: "__albite_alloc_profile_malloc_class",
        record: Class::Runtime,
        rule: Some(Class::Pure),
        cause: "its body only calls `__albite_alloc`, the shared allocator D2 does not follow",
    },
    Expectation {
        symbol: "tg_alloc_profile_marker",
        record: Class::Runtime,
        rule: Some(Class::Kernel),
        cause: "reads arena statistics under the profiler flag (`with_arena_ref`); the record read only the marker",
    },
    Expectation {
        symbol: "tg_get_last_error",
        record: Class::Runtime,
        rule: Some(Class::Kernel),
        cause: "the last-error slot lives in the kernel arena, opened with `with_arena_ref`",
    },
    Expectation {
        symbol: "tg_get_last_error_len",
        record: Class::Runtime,
        rule: Some(Class::Kernel),
        cause: "the last-error slot lives in the kernel arena, opened with `with_arena_ref`",
    },
    Expectation {
        symbol: "tg_int_trap",
        record: Class::Runtime,
        rule: Some(Class::Kernel),
        cause: "calls `crate::eval::IntTrapKind::from_code`, a code-to-text table the record flagged as not an evaluation",
    },
    Expectation {
        symbol: "tg_cstring_to_string",
        record: Class::Pure,
        rule: Some(Class::Runtime),
        cause: "`process::abort` on allocation failure; the record read the success path",
    },
    Expectation {
        symbol: "tg_string_append_char",
        record: Class::Pure,
        rule: Some(Class::Runtime),
        cause: "`process::abort` on allocation failure; the record read the success path",
    },
    Expectation {
        symbol: "tg_string_concat",
        record: Class::Pure,
        rule: Some(Class::Runtime),
        cause: "`process::abort` on allocation failure; the record read the success path",
    },
    Expectation {
        symbol: "tg_string_substring",
        record: Class::Pure,
        rule: Some(Class::Kernel),
        cause: "a cut inside a character calls `tg_int_trap` (ADR 16.10.26h D4), which the rule reads as kernel",
    },
];
