//! The integer trap written in Tungsten (ADR 7.10.26q AC 2, AC 3): the fixed
//! runtime-intrinsic list the core's rules admit, its reservation, and the
//! message table `src/native_core/trap.tg` holds, pinned to `albite_core`'s
//! `int_trap_line` by evaluating the embedded core on the evaluator.

use std::path::{Path, PathBuf};

use albite_core::eval::{eval_with_env, EvalEnv};
use albite_core::ffi::int_trap_line;
use albite_core::terms::Term;

use super::rules::{
    check_core_definitions, check_core_rules, definition_facts, is_admitted_extern,
    linear::buffer_signatures, CoreRuleViolation, DefinitionFacts,
};
use super::{
    elaborate_native_core, is_core_source, reserved_declarations, reserved_intrinsic,
    ReservedDeclaration, ReservedIntrinsic, NATIVE_CORE_SOURCES,
};
use crate::ast::Visibility;
use crate::driver::modules::ParsedModule;
use albite_core::native_core::RuntimeIntrinsic;

// ── The list and the rule (AC 2) ───────────────────────────────────────────

// 7.10.26q AC 2: the rule admits both runtime intrinsics, and still refuses
// a libc name and a Rust export, each as `ForeignExtern`.
#[test]
fn the_rule_admits_the_runtime_list_and_refuses_anything_else() {
    let traps = DefinitionFacts {
        name: "int_trap".to_string(),
        extern_symbols: vec![
            "tg_core_write_stderr".to_string(),
            "tg_core_abort".to_string(),
            "tg_bytes_len".to_string(),
        ],
        ..DefinitionFacts::default()
    };
    let leaks = DefinitionFacts {
        name: "leaks".to_string(),
        extern_symbols: vec!["malloc".to_string(), "tg_eprintln".to_string()],
        ..DefinitionFacts::default()
    };
    let violations = check_core_rules(&[traps, leaks], &[], &["trap"]);
    let foreign = |symbol: &str| CoreRuleViolation::ForeignExtern {
        definition: "leaks".to_string(),
        symbol: symbol.to_string(),
    };
    assert_eq!(violations, vec![foreign("malloc"), foreign("tg_eprintln")]);
    assert!(violations[0]
        .to_string()
        .contains("neither a byte intrinsic nor a core runtime intrinsic"));
    assert!(is_admitted_extern("__c_tg_core_abort"));
    assert!(is_admitted_extern("tg_bytes_new"));
    assert!(!is_admitted_extern("abort"));
    assert!(!is_admitted_extern("write"));
}

/// A parsed module at `path` holding `text`.
fn parsed_module(path: &str, text: &str) -> ParsedModule {
    ParsedModule {
        path: PathBuf::from(path),
        visibility: Visibility::Private,
        source_file: crate::parser::parse(text).0,
        submodules: vec![],
        mount_name: None,
    }
}

// 7.10.26q AC 2: both runtime intrinsics are reserved — a user module that
// declares either is refused, naming this ADR — and the embedded `trap.tg`,
// which declares both, is a core source and is accepted.
#[test]
fn a_user_declaration_of_either_runtime_intrinsic_is_refused() {
    for intrinsic in [RuntimeIntrinsic::WriteStderr, RuntimeIntrinsic::Abort] {
        assert_eq!(
            reserved_intrinsic(intrinsic.symbol()),
            Some(ReservedIntrinsic::Runtime(intrinsic))
        );
    }
    let user = "extern \"C\" fn tg_core_write_stderr(s: String) -> Nat\n\
                extern \"C\" fn tg_core_abort() -> Unit\n";
    let tree = parsed_module("user.tg", user);
    let found = reserved_declarations(&tree, &|_: &Path| Some(user.to_string()));
    let symbols: Vec<&str> = found.iter().map(|d| d.symbol.as_str()).collect();
    assert_eq!(symbols, vec!["tg_core_write_stderr", "tg_core_abort"]);
    let refusal = found[1].to_string();
    assert!(
        refusal.starts_with("user.tg: `tg_core_abort` is reserved"),
        "{refusal}"
    );
    assert!(refusal.contains("(ADR 7.10.26q D1)"), "{refusal}");

    let trap = NATIVE_CORE_SOURCES
        .iter()
        .find(|source| source.file == "trap.tg")
        .expect("trap.tg is embedded");
    assert!(is_core_source(trap.text));
    let core = parsed_module("core/trap.tg", trap.text);
    let accepted = reserved_declarations(&core, &|_: &Path| Some(trap.text.to_string()));
    assert_eq!(accepted, Vec::<ReservedDeclaration>::new());
}

// ── The embedded core, elaborated once ─────────────────────────────────────

/// The embedded core's definitions, as `run` sees them.
fn core_env() -> (Vec<crate::elaborate::CoreDef>, EvalEnv) {
    let dir = tempfile::tempdir().expect("temp dir");
    let project = elaborate_native_core(dir.path()).expect("the core elaborates");
    let globals = crate::comparator::eval::defs_as_globals(&project.defs);
    (project.defs, EvalEnv::new(globals))
}

/// `name(arg)` on the evaluator, as a string.
fn evaluate_to_string(env: &EvalEnv, name: &str, arg: u64) -> String {
    let call = Term::App(
        Box::new(Term::Global(name.to_string())),
        Box::new(Term::NatLit(arg)),
    );
    match eval_with_env(&call, env) {
        Ok(Term::StringLit(text)) => text,
        other => panic!("{name}({arg}) did not evaluate to a string: {other:?}"),
    }
}

// 7.10.26q AC 2: the embedded sources, `trap.tg` included, keep every rule,
// and the rule is not inert there: the core really calls the two runtime
// intrinsics. An `extern` elaborates to a stub definition of its own name
// whose body is the `ExternCall` (6.10.26n §6), so the facts are read there.
#[test]
fn the_embedded_trap_keeps_the_rules_and_calls_both_intrinsics() {
    let (defs, _) = core_env();
    assert_eq!(check_core_definitions(&defs, &NATIVE_CORE_SOURCES), vec![]);
    let texts: Vec<&str> = NATIVE_CORE_SOURCES.iter().map(|s| s.text).collect();
    let signatures = buffer_signatures(&texts);
    for intrinsic in [RuntimeIntrinsic::WriteStderr, RuntimeIntrinsic::Abort] {
        let stub = defs
            .iter()
            .find(|def| def.name.ends_with(intrinsic.symbol()))
            .unwrap_or_else(|| panic!("{} has no stub", intrinsic.symbol()));
        assert_eq!(
            definition_facts(stub, &signatures).extern_symbols,
            vec![intrinsic.symbol().to_string()]
        );
    }
}

// ── The two tables (AC 3) ──────────────────────────────────────────────────

// 7.10.26q AC 3: the core's `int_trap_line`, evaluated, is `albite_core`'s
// line byte for byte for every known code; for an unknown code Rust's line
// is the core's prefix, then each digit's `digit_literal`, then `)` — the
// order `int_trap` writes them in. 16.10.26h AC 3 adds code 13, the substring
// cut, so 14 is the first unknown code.
#[test]
fn the_core_trap_lines_equal_rusts_for_every_code() {
    let (_, env) = core_env();
    for code in 0..=13u32 {
        assert_eq!(
            evaluate_to_string(&env, "int_trap_line", u64::from(code)),
            int_trap_line(code),
            "code {code}"
        );
    }
    for code in [14u32, 42, 99, u32::MAX] {
        let mut line = evaluate_to_string(&env, "int_trap_line", u64::from(code));
        for digit in code.to_string().bytes() {
            line.push_str(&evaluate_to_string(
                &env,
                "digit_literal",
                u64::from(digit - b'0'),
            ));
        }
        line.push(')');
        assert_eq!(line, int_trap_line(code), "code {code}");
    }
}

// ── The compile goldens (AC 5) ─────────────────────────────────────────────

// 7.10.26q AC 5: the two native-trap goldens, which `make golden` runs under
// `off`, `on` and `all`, pin exactly the line both tables print for their
// code — `int_trap` code 0, `bytes_trap_index` code 8 — so a golden that
// passes under every setting is parity with the evaluator, not with itself.
#[test]
fn the_native_trap_goldens_pin_the_shared_trap_line() {
    let (_, env) = core_env();
    for (expected, code) in [
        (
            include_str!("../../../../tests/golden/compile/int_trap.expected"),
            0u32,
        ),
        (
            include_str!("../../../../tests/golden/compile/bytes_trap_index.expected"),
            8,
        ),
    ] {
        assert_eq!(
            expected,
            format!("{}\n", int_trap_line(code)),
            "code {code}"
        );
        assert_eq!(
            expected,
            format!(
                "{}\n",
                evaluate_to_string(&env, "int_trap_line", u64::from(code))
            ),
            "code {code}"
        );
    }
}

// ── The argument init (ADR 7.10.26r AC 4) ──────────────────────────────────

// 7.10.26r AC 4: the list has exactly three rows, and the rule admits the
// third — `tg_core_publish_args`, bare or prefixed — while a Rust init it
// replaces is still a foreign extern.
#[test]
fn the_rule_admits_exactly_one_more_name_for_the_argument_init() {
    use albite_core::native_core::CORE_RUNTIME_INTRINSICS;
    assert_eq!(CORE_RUNTIME_INTRINSICS.len(), 3);
    assert!(is_admitted_extern("tg_core_publish_args"));
    assert!(is_admitted_extern("__c_tg_core_publish_args"));
    assert!(!is_admitted_extern("tg_init_args"));
    assert!(!is_admitted_extern("tg_init_args_c"));
    let init = DefinitionFacts {
        name: "init_args_c".to_string(),
        extern_symbols: vec![
            "tg_core_publish_args".to_string(),
            "tg_init_args".to_string(),
        ],
        ..DefinitionFacts::default()
    };
    assert_eq!(
        check_core_rules(&[init], &[], &["args"]),
        vec![CoreRuleViolation::ForeignExtern {
            definition: "init_args_c".to_string(),
            symbol: "tg_init_args".to_string(),
        }]
    );
}

// 7.10.26r AC 4: a user module declaring `tg_core_publish_args` is refused,
// and the embedded `args.tg`, which declares it, is a core source, accepted,
// and its stub really calls the intrinsic.
#[test]
fn the_publish_intrinsic_is_reserved_and_the_embedded_args_tg_declares_it() {
    let symbol = RuntimeIntrinsic::PublishArgs.symbol();
    assert_eq!(
        reserved_intrinsic(symbol),
        Some(ReservedIntrinsic::Runtime(RuntimeIntrinsic::PublishArgs))
    );
    let user = "extern \"C\" fn tg_core_publish_args(argc: Nat, argv: Nat) -> Unit\n";
    let tree = parsed_module("user.tg", user);
    let found = reserved_declarations(&tree, &|_: &Path| Some(user.to_string()));
    let symbols: Vec<&str> = found.iter().map(|d| d.symbol.as_str()).collect();
    assert_eq!(symbols, vec!["tg_core_publish_args"]);
    assert!(
        found[0]
            .to_string()
            .starts_with("user.tg: `tg_core_publish_args` is reserved"),
        "{}",
        found[0]
    );

    let args = NATIVE_CORE_SOURCES
        .iter()
        .find(|source| source.file == "args.tg")
        .expect("args.tg is embedded");
    assert!(is_core_source(args.text));
    let core = parsed_module("core/args.tg", args.text);
    let accepted = reserved_declarations(&core, &|_: &Path| Some(args.text.to_string()));
    assert_eq!(accepted, Vec::<ReservedDeclaration>::new());

    let (defs, _) = core_env();
    let texts: Vec<&str> = NATIVE_CORE_SOURCES.iter().map(|s| s.text).collect();
    let stub = defs
        .iter()
        .find(|def| def.name.ends_with(symbol))
        .expect("the intrinsic has a stub");
    assert_eq!(
        definition_facts(stub, &buffer_signatures(&texts)).extern_symbols,
        vec![symbol.to_string()]
    );
    assert!(defs.iter().any(|def| def.name.ends_with("init_args_c")));
}
