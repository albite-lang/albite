//! `--classify`'s rule over injected facts and its extraction over inline
//! snippets (ADR 5.10.26f AC 1, AC 2). The oracle test against 1.10.26k's
//! record is in `oracle_tests.rs` (AC 3).

use super::extract::{CrateIndex, SourceFile};
use super::reach::Reach;
use super::{classify_export, Class, Fact, Facts, Verdict};
use crate::info::ffi::native_reach::{row_verdicts, RowVerdict};
use crate::info::ffi::NATIVE_CORE_BASIS;

fn facts(kernel: &[&str], runtime: &[&str], unreadable: &[&str]) -> Facts {
    let all = |tokens: &[&str]| tokens.iter().map(|token| Fact::direct(*token)).collect();
    Facts {
        kernel: all(kernel),
        runtime: all(runtime),
        unreadable: all(unreadable),
    }
}

// 5.10.26f AC1: kernel facts win over runtime facts, and runtime over pure.
#[test]
fn kernel_wins_over_runtime_and_runtime_over_unreadable() {
    let verdict = classify_export(&facts(&["with_arena"], &["CLI_ARGS"], &["sha2::x"]));
    assert_eq!(verdict.class, Class::Kernel);
    assert_eq!(verdict.deciding, vec![Fact::direct("with_arena")]);
    let verdict = classify_export(&facts(&[], &["CLI_ARGS"], &["sha2::x"]));
    assert_eq!(verdict.class, Class::Runtime);
    assert_eq!(verdict.basis(), "`CLI_ARGS`");
}

// 5.10.26f AC1: no facts is pure; an unreadable part alone is unknown.
#[test]
fn no_facts_is_pure_and_an_unreadable_part_is_unknown_never_pure() {
    let pure = classify_export(&Facts::default());
    assert_eq!(pure.class, Class::Pure);
    assert_eq!(pure.basis(), "no kernel or runtime fact");
    let unknown = classify_export(&facts(&[], &[], &["sha2::Sha256::new", "matches!"]));
    assert_eq!(unknown.class, Class::Unknown);
    assert_eq!(unknown.basis(), "`sha2::Sha256::new` (+1 more)");
}

// 5.10.26f AC1: each verdict carries the basis that decided it, with the
// helper it went through.
#[test]
fn the_basis_names_the_deciding_fact_and_its_helper() {
    let mut caller = Facts::default();
    caller.absorb_through(&facts(&["with_arena"], &[], &[]), "binary_term");
    let verdict = classify_export(&caller);
    assert_eq!(verdict.basis(), "`with_arena` via `binary_term`");
    let labels: Vec<&str> = [Class::Kernel, Class::Runtime, Class::Pure, Class::Unknown]
        .iter()
        .map(|class| class.label())
        .collect();
    assert_eq!(labels, ["kernel", "runtime", "pure", "unknown"]);
    assert_eq!(Class::from_label("runtime"), Some(Class::Runtime));
    assert_eq!(Class::from_label("Kernel"), None);
}

/// Classify `symbol` in a one-file `albite_core` crate made of `source`.
fn classify_snippet(source: &str, symbol: &str) -> Result<Verdict, String> {
    let files = [SourceFile::parse("lib.rs", &[], source).expect("snippet parses")];
    let index = CrateIndex::build(&[("albite_core", &files)]);
    Reach::new(&index)
        .export_facts(symbol)
        .map(|facts| classify_export(&facts))
}

fn class_of(source: &str) -> (Class, String) {
    let verdict = classify_snippet(source, "tg_probe").expect("tg_probe resolves");
    (verdict.class, verdict.basis())
}

// 5.10.26f AC2: a term or type handle call — the kernel arena macro, or a
// call into the checker — is kernel.
#[test]
fn an_arena_macro_or_a_checker_call_is_kernel() {
    let arena = "#[no_mangle] pub extern \"C\" fn tg_probe() -> u32 { with_arena!(|arena| arena.alloc(1)) }";
    assert_eq!(class_of(arena), (Class::Kernel, "`with_arena`".to_string()));
    let files = [
        SourceFile::parse(
            "lib.rs",
            &[],
            "#[no_mangle] pub extern \"C\" fn tg_probe() { crate::typecheck::type_of(); }",
        )
        .expect("parses"),
        SourceFile::parse("typecheck.rs", &["typecheck"], "pub fn type_of() {}").expect("parses"),
    ];
    let index = CrateIndex::build(&[("albite_core", &files)]);
    let facts = Reach::new(&index)
        .export_facts("tg_probe")
        .expect("resolves");
    assert_eq!(classify_export(&facts).basis(), "`type_of`");
}

// 5.10.26f AC2: an OS call and a direct global are runtime; an allocation
// alone is pure, as 1.10.26k's D2 rules.
#[test]
fn an_os_call_or_a_global_is_runtime_and_an_allocation_alone_is_pure() {
    let os = "#[no_mangle] pub extern \"C\" fn tg_probe() { let _ = std::fs::read(\"x\"); }";
    assert_eq!(class_of(os), (Class::Runtime, "`fs::read`".to_string()));
    let global =
        "static CLI_ARGS: u8 = 0;\n#[no_mangle] pub extern \"C\" fn tg_probe() -> u8 { CLI_ARGS }";
    assert_eq!(class_of(global), (Class::Runtime, "`CLI_ARGS`".to_string()));
    let console = "#[no_mangle] pub extern \"C\" fn tg_probe() { eprintln!(\"{}\", 1); }";
    assert_eq!(class_of(console).0, Class::Runtime);
    let alloc =
        "mod c_allocator { static ARENA_MODE: u8 = 0; pub fn malloc() -> u8 { ARENA_MODE } }\n\
                 #[no_mangle] pub extern \"C\" fn tg_probe() -> u8 { c_allocator::malloc() }";
    assert_eq!(class_of(alloc).0, Class::Pure);
}

// 5.10.26f AC2: braces and calls inside comments and string literals, and a
// multi-line signature, are never facts.
#[test]
fn comments_literals_and_signatures_are_not_facts() {
    let source = "#[no_mangle]\npub extern \"C\" fn tg_probe(\n    a: u64, // std::fs::read(\n    b: u64,\n) -> u64 {\n\
                  /* with_arena!(|a| { a }) */ let _ = \"std::process::exit(1) }\"; a + b\n}";
    assert_eq!(
        class_of(source),
        (Class::Pure, "no kernel or runtime fact".to_string())
    );
}

// 5.10.26f AC2: a macro whose arguments parse is read through; one whose
// arguments do not is unknown, never pure.
#[test]
fn a_readable_macro_is_read_and_an_unreadable_one_is_unknown() {
    let readable =
        "#[no_mangle] pub extern \"C\" fn tg_probe() { let _ = vec![std::env::args(); 2]; }";
    assert_eq!(
        class_of(readable),
        (Class::Runtime, "`env::args`".to_string())
    );
    let unreadable =
        "#[no_mangle] pub extern \"C\" fn tg_probe(x: u8) -> bool { matches!(x, 1..=3 if x > 1) }";
    assert_eq!(
        class_of(unreadable),
        (Class::Unknown, "`matches!`".to_string())
    );
}

// 5.10.26f AC2: a helper call is followed to its definition and named in the
// basis; a recursive helper terminates.
#[test]
fn a_helper_is_followed_and_a_recursive_one_terminates() {
    let helper = "static TEST_FAILED: u8 = 0;\nfn record() -> u8 { TEST_FAILED }\n\
                  #[no_mangle] pub extern \"C\" fn tg_probe() -> u8 { record() }";
    assert_eq!(
        class_of(helper),
        (Class::Runtime, "`TEST_FAILED` via `record`".to_string())
    );
    let recursive =
        "fn walk(n: u64) -> u64 { if n == 0 { std::process::abort() } else { walk(n - 1) } }\n\
                     #[no_mangle] pub extern \"C\" fn tg_probe(n: u64) -> u64 { walk(n) }";
    assert_eq!(
        class_of(recursive),
        (Class::Runtime, "`process::abort` via `walk`".to_string())
    );
}

// 5.10.26f AC2: a `thread_local!` declares exactly the names after `static`,
// and those, like a plain `static`, are the state a body can touch.
#[test]
fn a_thread_local_declares_only_its_static_names() {
    let source = "thread_local! { pub static FLAG: Cell<bool> = const { Cell::new(false) }; }\n\
                  static PLAIN: u8 = 0;\n\
                  #[no_mangle] pub extern \"C\" fn tg_probe() -> bool { FLAG.with(Cell::get) }";
    let files = [SourceFile::parse("lib.rs", &[], source).expect("parses")];
    let index = CrateIndex::build(&[("albite_core", &files)]);
    let mut statics: Vec<&str> = index.statics.iter().map(String::as_str).collect();
    statics.sort_unstable();
    assert_eq!(statics, ["FLAG", "PLAIN"]);
    assert_eq!(class_of(source), (Class::Runtime, "`FLAG`".to_string()));
}

// 5.10.26f AC2 / D5: `Type::name` follows that type's `impl` function, and
// `<T as Trait>::name` follows the trait's — both helpers in the same crate.
#[test]
fn an_associated_or_trait_call_is_followed_to_its_impl() {
    let associated =
        "static CLI_ARGS: u8 = 0;\nstruct Probe;\nimpl Probe { fn touch() -> u8 { CLI_ARGS } }\n\
                      #[no_mangle] pub extern \"C\" fn tg_probe() -> u8 { crate::Probe::touch() }";
    assert_eq!(
        class_of(associated),
        (Class::Runtime, "`CLI_ARGS` via `touch`".to_string())
    );
    let other_type = "static CLI_ARGS: u8 = 0;\nstruct Probe;\nstruct Other;\n\
                      impl Other { fn touch() -> u8 { CLI_ARGS } }\n\
                      #[no_mangle] pub extern \"C\" fn tg_probe() -> u8 { crate::Probe::touch() }";
    assert_eq!(
        class_of(other_type).0,
        Class::Pure,
        "another type's impl is not followed"
    );
    let through_trait = "static CLI_ARGS: u8 = 0;\nstruct Probe;\n\
                         trait Tracker { fn record() -> u8 { CLI_ARGS } }\nimpl Tracker for Probe {}\n\
                         #[no_mangle] pub extern \"C\" fn tg_probe() -> u8 { <Probe as crate::Tracker>::record() }";
    assert_eq!(
        class_of(through_trait),
        (Class::Runtime, "`CLI_ARGS` via `record`".to_string())
    );
}

// 5.10.26f AC2 / D5: a call into a crate outside the workspace, or an
// ambiguous one, is unknown; an OS crate is runtime.
#[test]
fn an_outside_or_ambiguous_call_is_unknown_and_an_os_crate_is_runtime() {
    let outside = "#[no_mangle] pub extern \"C\" fn tg_probe() { sha2::digest(); }";
    assert_eq!(
        class_of(outside),
        (Class::Unknown, "`sha2::digest`".to_string())
    );
    let libc = "#[no_mangle] pub extern \"C\" fn tg_probe() { libc::write(); }";
    assert_eq!(
        class_of(libc),
        (Class::Runtime, "`libc::write`".to_string())
    );
    let ambiguous = "mod a { pub fn f() {} }\nmod b { pub fn f() {} }\n\
                     #[no_mangle] pub extern \"C\" fn tg_probe() { crate::f(); }";
    assert_eq!(class_of(ambiguous).0, Class::Unknown);
}

// 5.10.26f AC2 / D2: a symbol with no item is an error naming it; a
// complementary `cfg` pair is one export, classed by its worst arm.
#[test]
fn a_missing_symbol_is_an_error_and_a_cfg_pair_is_one_export() {
    let error = classify_snippet("fn unrelated() {}", "tg_missing").expect_err("no item");
    assert!(error.contains("`tg_missing`"), "{error}");
    let pair = "#[cfg(unix)] #[no_mangle] pub extern \"C\" fn tg_probe() {}\n\
                #[cfg(not(unix))] #[no_mangle] pub extern \"C\" fn tg_probe() { std::process::abort() }";
    assert_eq!(class_of(pair).0, Class::Runtime);
    let twins = "#[no_mangle] pub extern \"C\" fn tg_probe() {}\n#[no_mangle] pub extern \"C\" fn tg_probe() {}";
    let error = classify_snippet(twins, "tg_probe").expect_err("ambiguous");
    assert!(error.contains("2 items"), "{error}");
}

/// The classed listing of one Rust export beside the real table's rows, the
/// Tungsten ones classed by `native`.
fn classed_listing(
    native: &std::collections::HashMap<String, Result<RowVerdict, String>>,
) -> String {
    use crate::info::ffi::{render_exports, Classified, ScannedRoot};
    use albite_bootstrap::doctor::checks::check_extern_symbols::ExportedSymbol;
    use albite_core::native_core::NATIVE_CORE_EXPORTS;
    let scanned = [ScannedRoot {
        root: "core".to_string(),
        exports: vec![ExportedSymbol {
            symbol: "tg_argc".to_string(),
            file: "core/args.rs".to_string(),
            cfg: None,
        }],
    }];
    let rust_verdicts = std::collections::HashMap::from([(
        "tg_argc".to_string(),
        classify_export(&facts(&[], &["CLI_ARGS"], &[])),
    )]);
    let classified = Classified {
        rust: &rust_verdicts,
        native,
    };
    render_exports(&scanned, NATIVE_CORE_EXPORTS, Some(&classified))
}

/// The embedded core's row verdicts, as `--classify` reads them (ADR 9.10.26c D2).
fn real_core_verdicts() -> std::collections::HashMap<String, Result<RowVerdict, String>> {
    use albite_bootstrap::driver::native_core::elaborate_native_core;
    use albite_core::native_core::NATIVE_CORE_EXPORTS;
    let dir = tempfile::tempdir().expect("scratch dir");
    let project = elaborate_native_core(dir.path()).expect("the core elaborates");
    row_verdicts(
        NATIVE_CORE_EXPORTS.iter().map(|row| row.definition),
        Ok(&project.defs),
    )
}

const TRAP_BASIS: &str = "native core: runtime intrinsics tg_core_abort, tg_core_write_stderr";

/// The listing line of the Tungsten row whose definition is `definition`.
fn row_line<'t>(text: &'t str, definition: &str) -> &'t str {
    let marker = format!("← {definition}  ");
    text.lines()
        .find(|line| line.contains(&marker))
        .expect(definition)
}

// ADR 9.10.26c AC3: over the embedded core the trap reaches both runtime
// intrinsics and the concats reach byte intrinsics only.
#[test]
fn the_real_core_classes_the_trap_runtime_and_the_concats_pure() {
    let verdicts = real_core_verdicts();
    let class_of = |row: &str| verdicts[row].clone().map(|v| (v.class, v.basis));
    assert_eq!(
        class_of("int_trap"),
        Ok((Class::Runtime, TRAP_BASIS.to_string()))
    );
    for concat in ["string_concat", "string_concat_owned"] {
        assert_eq!(
            class_of(concat),
            Ok((Class::Pure, NATIVE_CORE_BASIS.to_string()))
        );
    }
}

// ADR 9.10.26c AC4 (and 5.10.26f D6): the classed listing gives every row a
// class and a basis — the trap row runtime, the concat rows pure.
#[test]
fn the_listing_classes_every_row_the_tungsten_ones_included() {
    let text = classed_listing(&real_core_verdicts());
    assert!(
        text.contains("  tg_argc  core/args.rs  runtime  `CLI_ARGS`\n"),
        "{text}"
    );
    for (row, class, basis) in [
        ("int_trap", "runtime", TRAP_BASIS),
        ("string_concat", "pure", NATIVE_CORE_BASIS),
        ("string_concat_owned", "pure", NATIVE_CORE_BASIS),
    ] {
        let line = row_line(&text, row);
        assert!(line.ends_with(&format!(")  {class}  {basis}")), "{line}");
    }
    assert!(!text.contains("not classed"), "{text}");
}

// ADR 9.10.26c AC2: an unclassed row prints as in the plain listing, and
// each distinct reason follows the listing once.
#[test]
fn an_unclassed_row_prints_bare_and_its_reason_follows_once() {
    use albite_core::native_core::NATIVE_CORE_EXPORTS;
    let definitions = NATIVE_CORE_EXPORTS.iter().map(|row| row.definition);
    let text = classed_listing(&row_verdicts(definitions, Err("boom")));
    for row in NATIVE_CORE_EXPORTS {
        assert!(row_line(&text, row.definition).ends_with(')'), "{text}");
    }
    assert_eq!(text.matches("not classed").count(), 1, "{text}");
    assert!(
        text.ends_with(".\nnative core: not classed: boom\n"),
        "{text}"
    );
    // A row the walk never reached is unclassed, never pure.
    let text = classed_listing(&std::collections::HashMap::new());
    assert!(
        text.contains("native core: not classed: `int_trap` was not walked\n"),
        "{text}"
    );
    assert!(!text.contains("  pure  native core"), "{text}");
}

// 5.10.26f D4: the walk follows `mod` declarations — flat, nested, `#[path]`
// and inside an inline module — inherits a module's `cfg`, skips a test
// module, and names a declaration with no file.
#[test]
fn the_walk_follows_mod_declarations_and_skips_test_modules() {
    use super::walk::parse_crate;
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let write = |relative: &str, text: &str| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    };
    write(
        "lib.rs",
        "mod flat;\nmod nested;\n#[path = \"elsewhere.rs\"] mod moved;\n\
         mod inline { mod deep; }\n#[cfg(unix)] mod gated;\n#[cfg(test)] mod tests;\n",
    );
    write("flat.rs", "mod child;\n");
    write("flat/child.rs", "");
    write("nested/mod.rs", "");
    write("elsewhere.rs", "");
    write("inline/deep.rs", "");
    write("gated.rs", "");
    let parsed = parse_crate(root).expect("the crate parses");
    let mut modules: Vec<(String, Option<String>)> = parsed
        .iter()
        .map(|file| (file.module.join("::"), file.cfg.clone()))
        .collect();
    modules.sort();
    let expected = [
        ("", None),
        ("flat", None),
        ("flat::child", None),
        ("gated", Some("unix")),
        ("inline::deep", None),
        ("moved", None),
        ("nested", None),
    ];
    let expected: Vec<(String, Option<String>)> = expected
        .iter()
        .map(|&(module, cfg)| (module.to_owned(), cfg.map(str::to_owned)))
        .collect();
    assert_eq!(modules, expected);
    write("lib.rs", "mod absent;\n");
    let error = parse_crate(root).err().expect("a missing module file");
    assert!(error.contains("module `absent` has no file"), "{error}");
}
