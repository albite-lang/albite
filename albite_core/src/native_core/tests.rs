//! The export table, the runtime-intrinsic list, the switch and the core
//! buffer layout, as plain data (ADR 4.10.26n, 6.10.26n, 7.10.26q, 7.10.26r).

use super::*;
use crate::eval::byte_intrinsics::BYTES_MAX_LEN;

// 6.10.26m AC 2, 9.10.26n AC 4: the one-block layout, exactly, at its edges,
// with the 16-byte `{len, live}` trailer: a 7-byte join asks for 24 bytes.
#[test]
fn the_core_buffer_layout_puts_an_aligned_trailer_after_len_plus_one() {
    assert_eq!(CORE_TRAILER_BYTES, 16);
    assert_eq!(core_buffer_layout(0), Some((24, 8)));
    assert_eq!(core_buffer_layout(7), Some((24, 8)));
    assert_eq!(core_buffer_layout(8), Some((32, 16)));
    assert_eq!(
        core_buffer_layout(BYTES_MAX_LEN),
        Some((BYTES_MAX_LEN + 8 + 16, BYTES_MAX_LEN + 8))
    );
    assert_eq!(core_buffer_layout(BYTES_MAX_LEN + 1), None);
    for len in 0..64 {
        let (block, offset) = core_buffer_layout(len).expect("fits");
        assert_eq!(offset % BUFFER_HEADER_ALIGN, 0, "len {len}");
        assert!(
            offset > len && offset <= len + BUFFER_HEADER_ALIGN,
            "len {len}"
        );
        assert_eq!(block, offset + CORE_TRAILER_BYTES, "len {len}");
    }
}

// 13.10.26p AC2: P0.2 picked no size class, so the layout is unchanged above
// every threshold the replay tried (64, 128): no block is rounded past
// `round_up_8(len + 1) + 16` (D5, D6). 16.10.26j AC 3 keeps the layout too;
// its amortised grow turned the owned row on.
#[test]
fn no_size_class_rounds_a_large_core_block() {
    for len in 64..=1024 {
        let exact = ((len + 1 + 7) & !7) + CORE_TRAILER_BYTES;
        assert_eq!(
            core_buffer_layout(len).map(|(block, _)| block),
            Some(exact),
            "len {len}"
        );
    }
    let owned = NATIVE_CORE_EXPORTS
        .iter()
        .find(|row| row.symbol == "tg_native_string_concat_owned")
        .expect("owned row");
    // ADR 16.10.26j D7: on by default once the grow was amortised.
    assert!(owned.on_by_default);
}

#[test]
fn the_symbol_name_is_prefixed_once_and_rows_are_left_alone() {
    assert_eq!(
        native_core_symbol_name("bytes_new"),
        "__albite_native_core__bytes_new"
    );
    let once = native_core_symbol_name("bytes_new");
    assert_eq!(native_core_symbol_name(&once), once);
    assert_eq!(
        native_core_symbol_name("tg_native_string_concat"),
        "tg_native_string_concat"
    );
}

#[test]
fn only_a_table_row_is_an_export() {
    assert!(is_native_core_export("tg_native_string_concat"));
    assert!(!is_native_core_export("tg_string_concat"));
    assert!(!is_native_core_export(""));
    assert!(is_native_core_export("tg_native_string_concat_owned"));
    assert!(!is_native_core_export("tg_string_concat_owned"));
    assert_eq!(
        NATIVE_CORE_EXPORTS,
        &[
            NativeCoreExport {
                symbol: "tg_native_string_concat",
                definition: "string_concat",
                shadows: "tg_string_concat",
                on_by_default: true,
            },
            NativeCoreExport {
                symbol: "tg_native_string_concat_owned",
                definition: "string_concat_owned",
                shadows: "tg_string_concat_owned",
                on_by_default: true,
            },
            NativeCoreExport {
                symbol: "tg_native_int_trap",
                definition: "int_trap",
                shadows: "tg_int_trap",
                on_by_default: true,
            },
            NativeCoreExport {
                symbol: "tg_native_init_args_c",
                definition: "init_args_c",
                shadows: "tg_init_args_c",
                on_by_default: true,
            },
        ]
    );
    assert!(is_native_core_export("tg_native_int_trap"));
    assert!(!is_native_core_export("tg_int_trap"));
    assert!(is_native_core_export("tg_native_init_args_c"));
    assert!(!is_native_core_export("tg_init_args_c"));
}

// 7.10.26q AC 2, 7.10.26r AC 4: the list is exactly the three rows, each
// found by its own symbol (prefixed or bare), and nothing else is a runtime
// intrinsic.
#[test]
fn the_runtime_intrinsics_are_a_fixed_list_of_three() {
    assert_eq!(
        CORE_RUNTIME_INTRINSICS,
        &[
            RuntimeIntrinsic::WriteStderr,
            RuntimeIntrinsic::Abort,
            RuntimeIntrinsic::PublishArgs
        ]
    );
    for (symbol, intrinsic) in [
        ("tg_core_write_stderr", RuntimeIntrinsic::WriteStderr),
        ("__c_tg_core_write_stderr", RuntimeIntrinsic::WriteStderr),
        ("tg_core_abort", RuntimeIntrinsic::Abort),
        ("__c_tg_core_abort", RuntimeIntrinsic::Abort),
        ("tg_core_publish_args", RuntimeIntrinsic::PublishArgs),
        ("__c_tg_core_publish_args", RuntimeIntrinsic::PublishArgs),
    ] {
        assert_eq!(RuntimeIntrinsic::from_symbol(symbol), Some(intrinsic));
    }
    for symbol in [
        "write",
        "abort",
        "malloc",
        "tg_eprintln",
        "tg_bytes_new",
        "tg_init_args",
        "tg_core_args_count",
        "",
    ] {
        assert_eq!(RuntimeIntrinsic::from_symbol(symbol), None, "{symbol}");
    }
}

fn row(shadows: &'static str, on_by_default: bool) -> NativeCoreExport {
    NativeCoreExport {
        symbol: if shadows == "tg_string_concat" {
            "native_concat"
        } else {
            "native_owned"
        },
        definition: "d",
        shadows,
        on_by_default,
    }
}

// 6.10.26n AC 4: `off` enables nothing, `on` the default rows, `all` every row.
#[test]
fn each_setting_enables_its_rows() {
    let on = row("tg_string_concat", true);
    let off = row("tg_string_concat_owned", false);
    assert!(!EnabledRows::None.enables(&on));
    assert!(EnabledRows::Defaults.enables(&on));
    assert!(!EnabledRows::Defaults.enables(&off));
    assert!(EnabledRows::Every.enables(&off));
}

// 6.10.26n AC 4: the shadowing symbol for each pairing of setting and row default.
#[test]
fn the_shadowing_symbol_follows_the_setting_and_the_row_default() {
    for default in [false, true] {
        let rows = [
            row("tg_string_concat", true),
            row("tg_string_concat_owned", default),
        ];
        let owned = |enabled| shadowing_symbol(&rows, enabled, "tg_string_concat_owned");
        assert_eq!(owned(EnabledRows::None), "tg_string_concat_owned");
        let expected_on = if default {
            "native_owned"
        } else {
            "tg_string_concat_owned"
        };
        assert_eq!(owned(EnabledRows::Defaults), expected_on);
        assert_eq!(owned(EnabledRows::Every), "native_owned");
        assert_eq!(
            shadowing_symbol(&rows, EnabledRows::Defaults, "tg_string_concat"),
            "native_concat"
        );
    }
    assert_eq!(
        shadowing_symbol(&[], EnabledRows::Every, "tg_string_concat"),
        "tg_string_concat"
    );
}

// 6.10.26n AC 4 / D2: the real table holds the owned-implies-concat
// invariant under every setting, and a table that breaks it is caught.
#[test]
fn the_owned_row_is_enabled_only_with_the_concat_row() {
    for enabled in EnabledRows::ALL {
        assert!(
            owned_implies_concat(NATIVE_CORE_EXPORTS, enabled),
            "{enabled:?}"
        );
    }
    let broken = [
        row("tg_string_concat", false),
        row("tg_string_concat_owned", true),
    ];
    assert!(owned_implies_concat(&broken, EnabledRows::None));
    assert!(!owned_implies_concat(&broken, EnabledRows::Defaults));
    assert!(owned_implies_concat(&broken, EnabledRows::Every));
    let owned_only = [row("tg_string_concat_owned", true)];
    assert!(!owned_implies_concat(&owned_only, EnabledRows::Every));
    let concat_only = [row("tg_string_concat", true)];
    assert!(owned_implies_concat(&concat_only, EnabledRows::Every));
}
