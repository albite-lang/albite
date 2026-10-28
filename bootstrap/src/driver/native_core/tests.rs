//! The native core's pure seams (ADR 4.10.26n AC 3, AC 5, AC 6), over
//! injected inputs; one test elaborates the embedded sources themselves.

use std::path::{Path, PathBuf};

use super::rules::linear::buffer_signatures;
use super::rules::{
    check_core_definitions, check_core_rules, core_module_names, source_imports, CoreRuleViolation,
    DefinitionFacts, ModuleImport,
};
use super::switch::{native_core_choice, NativeCoreChoice, NATIVE_CORE_DEFAULT};
use super::{
    elaborate_native_core, is_core_source, refuse_reserved_declarations, reserved_declarations,
    reserved_intrinsic, CoreSource, ReservedDeclaration, ReservedIntrinsic, NATIVE_CORE_SOURCES,
};
use crate::ast::Visibility;
use crate::driver::modules::ParsedModule;
use albite_core::eval::byte_intrinsics::ByteIntrinsic;

// ── The switch (AC 6) ──────────────────────────────────────────────────────

// 4.10.26n AC 6: unset gives the default, which 7.10.26q D5 made `on`.
#[test]
fn an_unset_switch_is_the_default_and_the_default_is_on() {
    assert_eq!(native_core_choice(None), Ok(NATIVE_CORE_DEFAULT));
    assert_eq!(NATIVE_CORE_DEFAULT, NativeCoreChoice::On);
}

// 7.10.26q AC 7 (D5's second branch): the trap met parity and the `main.tg`
// budget (A/B 0.989×, A/A 0.996×) while 7.10.26p left the default `off`, so
// the default is now `on`, the error says so, and an unset variable enables
// exactly the rows that met their rules: the trap, and neither concat
// (7.10.26p halted over the short concat and the owned one). 7.10.26r AC 7:
// the argument init met parity, the startup budget and the `main.tg` budget,
// so it is on by default beside the trap. 9.10.26n AC 6: the concat met both
// micro budgets and the `main.tg` budget, so it is on by default too.
// 16.10.26j AC 5: with the grow amortised the owned concat met its budget
// and `main.tg`'s, so every row is on by default and `on` equals `all`.
#[test]
fn the_default_is_on_and_enables_every_row() {
    assert_eq!(native_core_choice(None), Ok(NativeCoreChoice::On));
    let error = native_core_choice(Some("yes")).expect_err("not a setting");
    assert!(error.contains("(unset means `on`)"), "{error}");
    let default_of = |symbol: &str| {
        albite_core::native_core::NATIVE_CORE_EXPORTS
            .iter()
            .find(|row| row.symbol == symbol)
            .map(|row| row.on_by_default)
    };
    assert_eq!(default_of("tg_native_string_concat"), Some(true));
    assert_eq!(default_of("tg_native_string_concat_owned"), Some(true));
    let enabled_by_unset: Vec<&str> = albite_core::native_core::NATIVE_CORE_EXPORTS
        .iter()
        .filter(|row| albite_core::native_core::EnabledRows::Defaults.enables(row))
        .map(|row| row.symbol)
        .collect();
    assert_eq!(
        enabled_by_unset,
        vec![
            "tg_native_string_concat",
            "tg_native_string_concat_owned",
            "tg_native_int_trap",
            "tg_native_init_args_c"
        ]
    );
    assert_eq!(default_of("tg_native_int_trap"), Some(true));
    assert_eq!(default_of("tg_native_init_args_c"), Some(true));
}

// 4.10.26n AC 6, 6.10.26n AC 4: `on`, `off` and `all` are themselves.
#[test]
fn on_off_and_all_are_named_settings() {
    assert_eq!(native_core_choice(Some("on")), Ok(NativeCoreChoice::On));
    assert_eq!(native_core_choice(Some("off")), Ok(NativeCoreChoice::Off));
    assert_eq!(native_core_choice(Some("all")), Ok(NativeCoreChoice::All));
}

// 4.10.26n AC 6, 6.10.26n AC 4: any other value is an error naming all
// three, never a default.
#[test]
fn any_other_value_is_an_error_naming_all_three_settings() {
    for value in ["", "ON", "yes", "1", " on", "ALL", "every"] {
        let error = native_core_choice(Some(value)).expect_err(value);
        assert!(error.contains("ALBITE_NATIVE_CORE"), "{error}");
        assert!(
            error.contains("`on`") && error.contains("`off`") && error.contains("`all`"),
            "{error}"
        );
        assert!(error.contains(&format!("{value:?}")), "{error}");
    }
}

// ── The embedded sources and the reservation (AC 5) ────────────────────────

#[test]
fn every_embedded_source_is_a_core_source_and_an_edit_is_not() {
    for source in &NATIVE_CORE_SOURCES {
        assert!(is_core_source(source.text), "{}", source.file);
        assert!(
            !is_core_source(&format!("{} ", source.text)),
            "{}",
            source.file
        );
    }
    assert!(!is_core_source(""));
}

#[test]
fn only_the_unchecked_conversion_and_the_adoption_are_reserved_intrinsics() {
    for symbol in [
        "tg_bytes_into_string_unchecked",
        "__c_tg_bytes_into_string_unchecked",
    ] {
        assert_eq!(
            reserved_intrinsic(symbol),
            Some(ReservedIntrinsic::Byte(ByteIntrinsic::IntoStringUnchecked))
        );
    }
    // 6.10.26n AC 2: the eighth is reserved too.
    assert_eq!(
        reserved_intrinsic("__c_tg_bytes_from_string_grown"),
        Some(ReservedIntrinsic::Byte(ByteIntrinsic::FromStringGrown))
    );
    assert_eq!(reserved_intrinsic("tg_bytes_into_string"), None);
    assert_eq!(reserved_intrinsic("tg_println"), None);
}

/// A parsed module at `path` holding `text`, with `submodules`.
fn parsed_module(path: &str, text: &str, submodules: Vec<ParsedModule>) -> ParsedModule {
    ParsedModule {
        path: PathBuf::from(path),
        visibility: Visibility::Private,
        source_file: crate::parser::parse(text).0,
        submodules,
        mount_name: None,
    }
}

const USER_DECLARATION: &str = "extern \"C\" fn tg_bytes_into_string_unchecked(h: Nat) -> String\n";

// 4.10.26n AC 5: a user module declaring the reserved intrinsic is refused,
// a module byte-equal to an embedded core source is accepted, and the walk
// reaches submodules.
#[test]
fn a_user_declaration_is_refused_and_a_core_source_is_accepted() {
    let core_bytes = NATIVE_CORE_SOURCES[1].text;
    let tree = parsed_module(
        "main.tg",
        "fn main() -> Nat { 0 }\n",
        vec![
            parsed_module("core/bytes.tg", core_bytes, vec![]),
            parsed_module("user.tg", USER_DECLARATION, vec![]),
        ],
    );
    let text_of = |path: &Path| match path.to_str() {
        Some("core/bytes.tg") => Some(core_bytes.to_string()),
        Some("user.tg") => Some(USER_DECLARATION.to_string()),
        _ => None,
    };
    assert_eq!(
        reserved_declarations(&tree, &text_of),
        vec![ReservedDeclaration {
            module: PathBuf::from("user.tg"),
            symbol: "tg_bytes_into_string_unchecked".to_string(),
        }]
    );
    let refusal = refuse_reserved_declarations(&tree, &text_of).expect_err("refused");
    assert!(refusal.starts_with("user.tg: `tg_bytes_into_string_unchecked` is reserved"));
}

// 6.10.26n AC 2: each reserved intrinsic's refusal gives its own reason.
#[test]
fn each_reserved_refusal_names_its_own_reason() {
    let refusal = |symbol: &str| {
        ReservedDeclaration {
            module: PathBuf::from("user.tg"),
            symbol: symbol.to_string(),
        }
        .to_string()
    };
    let unchecked = refusal("tg_bytes_into_string_unchecked");
    assert!(unchecked.contains("(ADR 4.10.26n D5)"), "{unchecked}");
    assert!(
        unchecked.contains("checked `tg_bytes_into_string`"),
        "{unchecked}"
    );
    let grown = refusal("__c_tg_bytes_from_string_grown");
    assert!(grown.contains("(ADR 6.10.26n D2)"), "{grown}");
    assert!(grown.contains("realloc read-only data"), "{grown}");
    assert!(!grown.contains("tg_bytes_into_string"), "{grown}");
}

#[test]
fn an_unreadable_module_is_not_a_core_source_and_a_clean_tree_passes() {
    let user = parsed_module("user.tg", USER_DECLARATION, vec![]);
    assert_eq!(reserved_declarations(&user, &|_| None).len(), 1);
    let clean = parsed_module(
        "main.tg",
        "extern \"C\" fn tg_bytes_into_string(h: Nat) -> String\n",
        vec![],
    );
    assert_eq!(refuse_reserved_declarations(&clean, &|_| None), Ok(()));
}

// ── The core's rules (AC 3) ────────────────────────────────────────────────

fn facts(name: &str) -> DefinitionFacts {
    DefinitionFacts {
        name: name.to_string(),
        ..DefinitionFacts::default()
    }
}

// 4.10.26n AC 3: each rule refuses an injected core, naming the definition.
#[test]
fn each_rule_refuses_an_injected_core_and_names_the_definition() {
    let concatenates = DefinitionFacts {
        concatenates: true,
        ..facts("joins")
    };
    let generic = DefinitionFacts {
        generic: true,
        ..facts("ident")
    };
    let foreign = DefinitionFacts {
        extern_symbols: vec!["tg_bytes_new".to_string(), "tg_println".to_string()],
        ..facts("prints")
    };
    let imports = [
        ModuleImport {
            module: "string.tg".to_string(),
            root: "bytes".to_string(),
        },
        ModuleImport {
            module: "string.tg".to_string(),
            root: "driver".to_string(),
        },
    ];
    let violations = check_core_rules(
        &[concatenates, generic, foreign, facts("clean")],
        &imports,
        &["bytes", "string"],
    );
    assert_eq!(
        violations,
        vec![
            CoreRuleViolation::Concatenates {
                definition: "joins".to_string()
            },
            CoreRuleViolation::Generic {
                definition: "ident".to_string()
            },
            CoreRuleViolation::ForeignExtern {
                definition: "prints".to_string(),
                symbol: "tg_println".to_string()
            },
            CoreRuleViolation::ImportsOutside {
                module: "string.tg".to_string(),
                root: "driver".to_string()
            },
        ]
    );
    let rendered: Vec<String> = violations.iter().map(ToString::to_string).collect();
    assert!(rendered[0].starts_with("joins: uses `++`"));
    assert!(rendered[1].starts_with("ident: is generic"));
    assert!(rendered[2].starts_with("prints: calls `tg_println`"));
    assert!(rendered[3].starts_with("string.tg: imports `driver`"));
}

#[test]
fn imports_are_read_by_their_first_segment_in_every_use_shape() {
    let text = "use bytes::{a, b};\nuse driver::ffi::x;\nuse lexer::*;\nuse parser::p as q;\n";
    let roots: Vec<String> = source_imports("m.tg", text)
        .into_iter()
        .map(|import| import.root)
        .collect();
    assert_eq!(roots, vec!["bytes", "driver", "lexer", "parser"]);
    assert_eq!(
        core_module_names(&NATIVE_CORE_SOURCES),
        vec!["args", "bytes", "string", "trap"]
    );
}

// 4.10.26n AC 3: the same check over the EMBEDDED sources accepts them, and
// the facts are real: `string_concat` is in the core and uses no `++`.
#[test]
fn the_embedded_core_elaborates_and_keeps_its_rules() {
    let dir = tempfile::tempdir().expect("temp dir");
    let project = elaborate_native_core(dir.path()).expect("the core elaborates");
    assert_eq!(
        check_core_definitions(&project.defs, &NATIVE_CORE_SOURCES),
        vec![]
    );
    let concat = project
        .defs
        .iter()
        .find(|def| def.name.ends_with("string_concat"))
        .expect("string_concat is defined");
    let texts: Vec<&str> = NATIVE_CORE_SOURCES.iter().map(|s| s.text).collect();
    let signatures = buffer_signatures(&texts);
    assert!(!super::rules::definition_facts(concat, &signatures).concatenates);
    // 6.10.26m AC 3: the at-most-once rule accepts every real definition, and
    // is not inert there: the wrappers the core threads are known to return
    // a buffer, and their parameters to be one.
    assert!(signatures.returns_buffer.contains("bytes_blit_string"));
    assert_eq!(signatures.buffer_params["bytes_set"], vec!["b".to_string()]);
    let unchecked = project
        .defs
        .iter()
        .map(|def| super::rules::definition_facts(def, &signatures))
        .flat_map(|facts| facts.extern_symbols)
        .any(|symbol| symbol == "tg_bytes_into_string_unchecked");
    assert!(unchecked, "the core calls the reserved conversion");
    // 6.10.26n AC 3: the adoption rule accepts the real owned export and
    // is not inert there: it sees one wrapper adoption of its first parameter.
    let owned = project
        .defs
        .iter()
        .find(|def| def.name.ends_with("string_concat_owned"))
        .expect("string_concat_owned is defined");
    let adoptions = super::rules::definition_facts(owned, &signatures).adoptions;
    assert_eq!(adoptions.len(), 1, "{adoptions:?}");
    assert_eq!(
        adoptions[0].argument,
        super::rules::adopt::AdoptedArgument::FirstParameter
    );
    assert!(!adoptions[0].used_after);
    // 6.10.26m AC 3: the signatures come from the SOURCES passed in, so a
    // source declaring `string_concat`'s operands buffers makes each one's two
    // real uses (its `string_len`, then its blit) a reuse.
    let declared = CoreSource {
        file: "string.tg",
        text: "pub fn string_concat(left: Bytes, right: Bytes) -> Bytes { left }\n",
    };
    let reuses: Vec<CoreRuleViolation> = check_core_definitions(&project.defs, &[declared])
        .into_iter()
        .filter(|violation| matches!(violation, CoreRuleViolation::BufferReused { .. }))
        .collect();
    let in_concat = |variable: &str| CoreRuleViolation::BufferReused {
        definition: "string_concat".to_string(),
        variable: variable.to_string(),
    };
    assert_eq!(reuses, vec![in_concat("left"), in_concat("right")]);
}
