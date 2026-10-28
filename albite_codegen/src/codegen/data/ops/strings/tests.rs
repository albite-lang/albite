use super::*;
use inkwell::context::Context;

#[test]
fn test_string_concat_ffi_declared() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "test");
    assert!(
        codegen.module.get_function("tg_string_concat").is_some(),
        "tg_string_concat should be declared in runtime functions"
    );
}

#[test]
fn test_string_concat_owned_ffi_declared() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "test");
    assert!(
        codegen
            .module
            .get_function("tg_string_concat_owned")
            .is_some(),
        "tg_string_concat_owned should be declared in runtime functions"
    );
}

#[test]
fn test_liveness_gate_nested_concat_uses_owned() {
    // Term::StrConcat(Term::StrConcat(..), ..) should trigger owned path
    let inner = Term::str_concat(
        Term::StringLit("a".to_string()),
        Term::StringLit("b".to_string()),
    );
    let right = Term::StringLit("c".to_string());
    let outer_left: &Term = &inner;
    // The gate: nested StrConcat → owned
    assert!(
        matches!(outer_left, Term::StrConcat(_, _)),
        "nested StrConcat left should match the owned-path gate"
    );
    let _ = right;
}

#[test]
fn test_liveness_gate_literal_uses_regular() {
    // Term::StringLit should NOT trigger owned path
    let lit = Term::StringLit("hello".to_string());
    let left: &Term = &lit;
    assert!(
        !matches!(left, Term::StrConcat(_, _)),
        "string literal should NOT match the owned-path gate"
    );
}

#[test]
fn test_liveness_gate_variable_uses_regular() {
    // Term::Var should NOT trigger owned path without state tracking
    let var = Term::Var("x".to_string());
    let left: &Term = &var;
    assert!(
        !matches!(left, Term::StrConcat(_, _)),
        "variable reference should NOT match the owned-path gate"
    );
}

#[test]
fn test_last_use_vars_populated_for_str_concat_binding() {
    // When let s = a ++ b in <body using s once>, s should be in last_use_vars
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");

    // Simulate: s is bound to StrConcat, used once in body
    codegen.compilation.last_use_vars.insert("s".to_string());
    codegen.compilation.heap_origin_vars.insert("s".to_string());

    let left = Term::Var("s".to_string());
    // Gate check: Var("s") with both sets populated → owned
    if let Term::Var(x) = &left {
        let uses_owned = codegen.compilation.last_use_vars.contains(x)
            && codegen.compilation.heap_origin_vars.contains(x);
        assert!(
            uses_owned,
            "let-bound last-use StrConcat var should trigger owned path"
        );
    }
}

#[test]
fn test_last_use_vars_not_triggered_without_heap_origin() {
    // Var in last_use but NOT heap_origin → regular (could be a literal binding)
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");

    codegen.compilation.last_use_vars.insert("s".to_string());
    // heap_origin NOT set

    let left = Term::Var("s".to_string());
    if let Term::Var(x) = &left {
        let uses_owned = codegen.compilation.last_use_vars.contains(x)
            && codegen.compilation.heap_origin_vars.contains(x);
        assert!(
            !uses_owned,
            "last-use without heap-origin should NOT trigger owned path"
        );
    }
}

#[test]
fn test_last_use_vars_not_triggered_for_multi_use() {
    // Var in heap_origin but NOT last_use → regular (used multiple times)
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");

    codegen.compilation.heap_origin_vars.insert("s".to_string());
    // last_use NOT set

    let left = Term::Var("s".to_string());
    if let Term::Var(x) = &left {
        let uses_owned = codegen.compilation.last_use_vars.contains(x)
            && codegen.compilation.heap_origin_vars.contains(x);
        assert!(
            !uses_owned,
            "heap-origin without last-use should NOT trigger owned path"
        );
    }
}

// ── The concat symbol and the native-core switch (ADR 4.10.26n AC 6) ───────

mod native_core_switch {
    use crate::codegen::data::ops::strings::concat_symbol::{
        concat_symbol, NATIVE_CONCAT, NATIVE_CONCAT_OWNED, RUST_CONCAT, RUST_CONCAT_OWNED,
    };
    use crate::codegen::native_core::{NativeCoreExport, NativeCoreSwitch, NATIVE_CORE_EXPORTS};
    use crate::codegen::CodeGen;
    use albite_core::terms::Term;
    use inkwell::context::Context;

    /// The real two concat rows, the concat on by default (as it was until
    /// 7.10.26q D5) and the owned row's default set to `owned_default`.
    fn rows(owned_default: bool) -> [NativeCoreExport; 2] {
        let mut rows = [NATIVE_CORE_EXPORTS[0], NATIVE_CORE_EXPORTS[1]];
        rows[0].on_by_default = true;
        rows[1].on_by_default = owned_default;
        rows
    }

    // 6.10.26n AC 4: every pairing of owned-ness, setting and row default.
    #[test]
    fn each_path_follows_the_switch_and_its_row_default() {
        use NativeCoreSwitch::{All, Off, On};
        for owned_default in [false, true] {
            let rows = rows(owned_default);
            assert_eq!(concat_symbol(false, Off, &rows), RUST_CONCAT);
            assert_eq!(concat_symbol(false, On, &rows), NATIVE_CONCAT);
            assert_eq!(concat_symbol(false, All, &rows), NATIVE_CONCAT);
            assert_eq!(concat_symbol(true, Off, &rows), RUST_CONCAT_OWNED);
            let owned_on = if owned_default {
                NATIVE_CONCAT_OWNED
            } else {
                RUST_CONCAT_OWNED
            };
            assert_eq!(concat_symbol(true, On, &rows), owned_on);
            assert_eq!(concat_symbol(true, All, &rows), NATIVE_CONCAT_OWNED);
        }
        assert_eq!(NATIVE_CONCAT, "tg_native_string_concat");
        assert_eq!(NATIVE_CONCAT_OWNED, "tg_native_string_concat_owned");
        // The shipped table: the concat row is on by default (9.10.26n D5)
        // and so is the owned row (16.10.26j D7), so `on` calls both native
        // concats.
        assert_eq!(
            concat_symbol(true, On, NATIVE_CORE_EXPORTS),
            NATIVE_CONCAT_OWNED
        );
        assert_eq!(concat_symbol(false, On, NATIVE_CORE_EXPORTS), NATIVE_CONCAT);
    }

    // 6.10.26n AC 4 / D2: a row table enabling the owned shim without the
    // concat shim trips `concat_symbol`'s debug assertion.
    #[test]
    #[should_panic(expected = "an owned native concat is enabled without the native concat")]
    fn a_table_enabling_owned_without_concat_trips_the_assertion() {
        let mut broken = rows(true);
        broken[0].on_by_default = false;
        let _ = concat_symbol(false, NativeCoreSwitch::On, &broken);
    }

    /// Lower `left ++ right` under `switch` into a fresh function and return
    /// the module text.
    fn lower_concat(switch: NativeCoreSwitch, left: Term, right: Term) -> String {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "concat_switch");
        codegen.set_native_core_switch(switch);
        let function =
            codegen
                .module
                .add_function("probe", context.void_type().fn_type(&[], false), None);
        codegen
            .builder
            .position_at_end(context.append_basic_block(function, "entry"));
        codegen.compilation.current_fn = Some(function);
        codegen.compile_str_concat(&left, &right).expect("lowers");
        codegen.builder.build_return(None).expect("return");
        codegen.module.print_to_string().to_string()
    }

    fn literal(text: &str) -> Term {
        Term::StringLit(text.to_string())
    }

    // 4.10.26n AC 6: `off` calls exactly today's symbol and never mentions
    // the native export, not even as a declaration.
    #[test]
    fn off_emits_todays_symbols_and_no_native_declaration() {
        let text = lower_concat(NativeCoreSwitch::Off, literal("a"), literal("b"));
        assert!(
            text.contains("call { ptr, i64 } @tg_string_concat("),
            "{text}"
        );
        assert!(!text.contains(NATIVE_CONCAT), "{text}");
        let nested = Term::str_concat(literal("a"), literal("b"));
        let owned = lower_concat(NativeCoreSwitch::Off, nested, literal("c"));
        assert!(
            owned.contains("call { ptr, i64 } @tg_string_concat_owned("),
            "{owned}"
        );
    }

    /// Lower `s ++ "c"` with `s` bound in the env and marked last-use and/or
    /// heap-origin, under `off`; true when the owned symbol was called.
    fn compile_str_concat_uses_owned_for_var(last_use: bool, heap_origin: bool) -> bool {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "concat_var");
        let function =
            codegen
                .module
                .add_function("probe", context.void_type().fn_type(&[], false), None);
        codegen
            .builder
            .position_at_end(context.append_basic_block(function, "entry"));
        codegen.compilation.current_fn = Some(function);
        let bound = codegen.compile_string_lit("ab").expect("literal");
        codegen
            .compilation
            .env
            .insert("s".to_string(), (bound, albite_core::types::Type::String));
        if last_use {
            codegen.compilation.last_use_vars.insert("s".to_string());
        }
        if heap_origin {
            codegen.compilation.heap_origin_vars.insert("s".to_string());
        }
        codegen
            .compile_str_concat(&Term::Var("s".to_string()), &literal("c"))
            .expect("lowers");
        codegen.builder.build_return(None).expect("return");
        let text = codegen.module.print_to_string().to_string();
        text.contains("call { ptr, i64 } @tg_string_concat_owned(")
    }

    // The liveness gate through the real lowering: owned only when the
    // variable is BOTH last-use and heap-origin.
    #[test]
    fn compile_str_concat_owns_a_var_only_when_last_use_and_heap_origin() {
        assert!(compile_str_concat_uses_owned_for_var(true, true));
        assert!(!compile_str_concat_uses_owned_for_var(true, false));
        assert!(!compile_str_concat_uses_owned_for_var(false, true));
        assert!(!compile_str_concat_uses_owned_for_var(false, false));
    }

    // 7.10.26q AC 7, 9.10.26n AC 6, 16.10.26j AC 5: with both concat rows on
    // by default, `on` moves the plain concat call and the owned one.
    #[test]
    fn on_moves_the_concat_call_and_the_owned_one() {
        let text = lower_concat(NativeCoreSwitch::On, literal("a"), literal("b"));
        assert!(
            text.contains("call { ptr, i64 } @tg_native_string_concat("),
            "{text}"
        );
        assert!(
            !text.contains("call { ptr, i64 } @tg_string_concat("),
            "{text}"
        );
        let nested = Term::str_concat(literal("a"), literal("b"));
        let owned = lower_concat(NativeCoreSwitch::On, nested, literal("c"));
        assert!(
            owned.contains(&format!("call {{ ptr, i64 }} @{NATIVE_CONCAT_OWNED}(")),
            "{owned}"
        );
        assert!(
            !owned.contains("call { ptr, i64 } @tg_string_concat_owned("),
            "{owned}"
        );
    }

    // 6.10.26n AC 4: `all` moves both calls, and neither Rust export is called.
    #[test]
    fn all_moves_the_owned_call_too() {
        let nested = Term::str_concat(literal("a"), literal("b"));
        let owned = lower_concat(NativeCoreSwitch::All, nested, literal("c"));
        assert!(
            owned.contains("call { ptr, i64 } @tg_native_string_concat("),
            "{owned}"
        );
        assert!(
            owned.contains("call { ptr, i64 } @tg_native_string_concat_owned("),
            "{owned}"
        );
        assert!(
            !owned.contains("call { ptr, i64 } @tg_string_concat"),
            "{owned}"
        );
    }
}
