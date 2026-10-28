//! `albite info ffi exports`' listing over injected inputs (ADR 4.10.26n AC 11).

use super::{render_exports, ScannedRoot};
use albite_bootstrap::doctor::checks::check_extern_symbols::ExportedSymbol;
use albite_core::native_core::{NativeCoreExport, NATIVE_CORE_EXPORTS};

fn export(symbol: &str, file: &str, cfg: Option<&str>) -> ExportedSymbol {
    ExportedSymbol {
        symbol: symbol.to_string(),
        file: file.to_string(),
        cfg: cfg.map(str::to_string),
    }
}

// 4.10.26n AC 11: every export is listed per root, sorted, with its file and
// gate, and each table row is shown as written in Tungsten with what it shadows.
#[test]
fn the_listing_shows_each_root_sorted_and_the_rows_written_in_tungsten() {
    let scanned = [
        ScannedRoot {
            root: "core".to_string(),
            exports: vec![
                export("tg_string_concat", "core/concat.rs", None),
                export("tg_argc", "core/args.rs", Some("unix")),
            ],
        },
        ScannedRoot {
            root: "runtime".to_string(),
            exports: vec![export("tg_alloc", "runtime/alloc.rs", None)],
        },
    ];
    let text = render_exports(&scanned, NATIVE_CORE_EXPORTS, None);
    assert!(text.contains("core — 2 export(s) in Rust:\n"), "{text}");
    let argc = text
        .find("  tg_argc  core/args.rs  #[cfg(unix)]\n")
        .expect("argc");
    let concat = text
        .find("  tg_string_concat  core/concat.rs\n")
        .expect("concat");
    assert!(argc < concat, "sorted by symbol:\n{text}");
    assert!(text.contains("runtime — 1 export(s) in Rust:\n  tg_alloc  runtime/alloc.rs\n"));
    // 6.10.26n AC 4, 7.10.26q AC 7, 7.10.26r AC 7, 9.10.26n AC 6: every row
    // is listed, and since 16.10.26j AC 5 all four as on by default — the
    // concat, its owned twin, the trap and the init. ADR 9.10.26c AC4: unclassed, each row is
    // byte-identical — no class, no basis, and no `not classed` line after
    // the totals.
    assert!(
        text.contains(
            "Written in Albite (src/native_core/) — 4:\n  tg_native_string_concat  ← \
             string_concat  shadows tg_string_concat (linked under ALBITE_NATIVE_CORE=on)\n  \
             tg_native_string_concat_owned  ← string_concat_owned  shadows \
             tg_string_concat_owned (linked under ALBITE_NATIVE_CORE=on)\n  \
             tg_native_int_trap  ← int_trap  shadows tg_int_trap \
             (linked under ALBITE_NATIVE_CORE=on)\n  \
             tg_native_init_args_c  ← init_args_c  shadows tg_init_args_c \
             (linked under ALBITE_NATIVE_CORE=on)\n"
        ),
        "{text}"
    );
    assert!(
        text.ends_with("3 Rust export(s) in 2 root(s); 4 written in Albite.\n"),
        "{text}"
    );
}

// The command's two exits: a root that is not a directory is exit 2, never an
// empty listing; a readable root lists and exits 0.
#[test]
fn the_command_refuses_a_missing_root_and_lists_a_real_one() {
    use super::{dispatch_ffi_info, InfoFfiCommands};
    use std::process::ExitCode;
    let missing = std::path::PathBuf::from("no/such/ffi/root");
    let refused = dispatch_ffi_info(InfoFfiCommands::Exports {
        roots: vec![missing],
        classify: false,
    });
    assert_eq!(refused, ExitCode::from(2));
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("lib.rs"),
        "#[no_mangle]\npub extern \"C\" fn tg_probe() {}\n",
    )
    .expect("write");
    let listed = dispatch_ffi_info(InfoFfiCommands::Exports {
        roots: vec![dir.path().to_path_buf()],
        classify: false,
    });
    assert_eq!(listed, ExitCode::SUCCESS);
    // ADR 9.10.26c D4: classed, a pure export and a core whose every row
    // walks clean exit 0 too.
    let classed = dispatch_ffi_info(InfoFfiCommands::Exports {
        roots: vec![dir.path().to_path_buf()],
        classify: true,
    });
    assert_eq!(classed, ExitCode::SUCCESS);
    // An unclassifiable Rust export still exits 2 beside a clean core.
    std::fs::write(
        dir.path().join("lib.rs"),
        "#[no_mangle]\npub extern \"C\" fn tg_probe() {}\n#[no_mangle]\npub extern \"C\" fn tg_probe() {}\n",
    )
    .expect("write");
    let ambiguous = dispatch_ffi_info(InfoFfiCommands::Exports {
        roots: vec![dir.path().to_path_buf()],
        classify: true,
    });
    assert_eq!(ambiguous, ExitCode::from(2));
}

#[test]
fn an_empty_root_says_zero_rather_than_nothing() {
    let scanned = [ScannedRoot {
        root: "empty".to_string(),
        exports: vec![],
    }];
    let none: [NativeCoreExport; 0] = [];
    let text = render_exports(&scanned, &none, None);
    assert!(text.starts_with("empty — 0 export(s) in Rust:\n"), "{text}");
    assert!(
        text.contains("Written in Albite (src/native_core/) — 0:\n"),
        "{text}"
    );
    assert!(text.ends_with("0 Rust export(s) in 1 root(s); 0 written in Albite.\n"));
}
