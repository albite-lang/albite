//! Tests for the `main` / `__albite_inner_main` wrapper: what the prologue
//! emits unconditionally.

use super::*;
use inkwell::context::Context;

/// Verify that `compile_main_wrapper` unconditionally emits the CLI args
/// init sequence — `tg_init_args_c` call + `__albite_argc`/`__albite_argv`
/// globals — even when `tg_argc`/`tg_argv` are not present in the module.
///
/// ADR 10.5.26c: in per-module codegen, `tg_argc`/`tg_argv` live in separate
/// codegen units, so a visibility check would incorrectly skip init.
#[test]
fn test_emit_cli_args_init_unconditional() {
    let ctx = Context::create();
    let mut codegen = CodeGen::new(&ctx, "test_main_wrapper");

    // Register a simple main type: Unit -> Unit
    let main_ty = albite_core::types::Type::Arrow(
        Box::new(albite_core::types::Type::Unit),
        Box::new(albite_core::types::Type::Unit),
    );

    // Declare albite_main so the wrapper can call it
    codegen
        .declare_def("albite_main", &main_ty)
        .expect("declare albite_main");

    // Compile the main wrapper — should succeed without tg_argc/tg_argv
    codegen
        .compile_main_wrapper(&main_ty)
        .expect("compile_main_wrapper should succeed");

    let ir = codegen.get_ir_string();

    // Verify tg_init_args_c is declared and called
    assert!(
        ir.contains("tg_init_args_c"),
        "IR should contain tg_init_args_c declaration/call"
    );
    // Verify argc/argv globals are emitted
    assert!(
        ir.contains("__albite_argc"),
        "IR should contain __albite_argc global"
    );
    assert!(
        ir.contains("__albite_argv"),
        "IR should contain __albite_argv global"
    );
}

/// ADR 14.9.26b: the prologue reads `ALBITE_ARENA` through
/// `__albite_arena_init()` unconditionally — including under
/// `--debug-info`, where the signal-handler install is skipped and a
/// mode read placed beside it would leave a debug build silently `off`.
/// The call must precede the `albite_main` call, so the first
/// allocation sees the mode.
#[test]
fn arena_init_is_emitted_before_main_even_with_debug_info() {
    let ctx = Context::create();
    let mut codegen = CodeGen::new(&ctx, "test_arena_init");
    codegen.enable_debug_info("arena_init.tg", "fn main() -> Unit { () }\n");

    let main_ty = albite_core::types::Type::Arrow(
        Box::new(albite_core::types::Type::Unit),
        Box::new(albite_core::types::Type::Unit),
    );
    codegen
        .declare_def("albite_main", &main_ty)
        .expect("declare albite_main");
    codegen
        .compile_main_wrapper(&main_ty)
        .expect("compile_main_wrapper should succeed");

    let ir = codegen.get_ir_string();
    assert!(
        !ir.contains("__albite_install_signal_handlers"),
        "precondition: debug-info skips the signal handlers"
    );
    let init_at = ir
        .find("call void @__albite_arena_init()")
        .expect("IR should call __albite_arena_init()");
    // The call site passes the null env; the declaration has no operand.
    let main_at = ir
        .find("@albite_main(ptr null)")
        .expect("IR should call albite_main");
    assert!(
        init_at < main_at,
        "arena init must run before the first allocation (albite_main)"
    );
}

/// The main wrapper's IR for a `Unit -> Unit` main with the region marker
/// set to `regions_used`.
fn main_wrapper_ir_with_regions(regions_used: bool) -> String {
    let ctx = Context::create();
    let mut codegen = CodeGen::new(&ctx, "test_init_regions");
    codegen.regions.regions_used = regions_used;
    let main_ty = albite_core::types::Type::Arrow(
        Box::new(albite_core::types::Type::Unit),
        Box::new(albite_core::types::Type::Unit),
    );
    codegen
        .declare_def("albite_main", &main_ty)
        .expect("declare albite_main");
    codegen
        .compile_main_wrapper(&main_ty)
        .expect("compile_main_wrapper should succeed");
    codegen.get_ir_string()
}

/// ADR 7.10.26s AC 5 (D4): with the marker unset the prologue calls
/// `__albite_arena_init()` and never names the regions init — the default
/// marker is unset, byte for byte. With it set the prologue calls
/// `__albite_arena_init_regions()` instead, before `albite_main`.
#[test]
fn the_prologue_calls_the_init_the_region_marker_selects() {
    let unset = main_wrapper_ir_with_regions(false);
    assert!(
        unset.contains("call void @__albite_arena_init()"),
        "{unset}"
    );
    assert!(!unset.contains("__albite_arena_init_regions"), "{unset}");
    let default_ir = main_wrapper_ir(crate::codegen::native_core::NativeCoreSwitch::default());
    assert_eq!(
        unset.replace("test_init_regions", "test_init_args"),
        default_ir,
        "an unset marker changes nothing"
    );

    let set = main_wrapper_ir_with_regions(true);
    let init_at = set
        .find("call void @__albite_arena_init_regions()")
        .expect("the marked prologue calls the regions init");
    assert!(!set.contains("call void @__albite_arena_init()"), "{set}");
    assert!(
        set.contains("declare void @__albite_arena_init_regions()"),
        "{set}"
    );
    let main_at = set
        .find("@albite_main(ptr null)")
        .expect("IR should call albite_main");
    assert!(init_at < main_at, "{set}");
}

/// The main wrapper's IR for a `Unit -> Unit` main under `switch`.
fn main_wrapper_ir(switch: crate::codegen::native_core::NativeCoreSwitch) -> String {
    let ctx = Context::create();
    let mut codegen = CodeGen::new(&ctx, "test_init_args");
    codegen.set_native_core_switch(switch);
    let main_ty = albite_core::types::Type::Arrow(
        Box::new(albite_core::types::Type::Unit),
        Box::new(albite_core::types::Type::Unit),
    );
    codegen
        .declare_def("albite_main", &main_ty)
        .expect("declare albite_main");
    codegen
        .compile_main_wrapper(&main_ty)
        .expect("compile_main_wrapper should succeed");
    codegen.get_ir_string()
}

/// ADR 19.10.26c AC 4 (D2): `main` hands `__albite_inner_main` to
/// `__albite_run_on_stack` and returns what it returns; it no longer calls
/// the body directly, so the body runs on the runtime-sized stack.
#[test]
fn main_runs_the_body_through_the_runtime_stack_entry() {
    let ir = main_wrapper_ir(crate::codegen::native_core::NativeCoreSwitch::default());
    assert!(
        ir.contains("declare i32 @__albite_run_on_stack(ptr)"),
        "{ir}"
    );
    let main_def = &ir[ir.find("define i32 @main(").expect("main is defined")..];
    let main_body = &main_def[..main_def.find("\n}\n").expect("main has a body")];
    assert!(
        main_body
            .contains("%exit_code = call i32 @__albite_run_on_stack(ptr @__albite_inner_main)"),
        "{main_body}"
    );
    assert!(
        !main_body.contains("call i32 @__albite_inner_main("),
        "main must not call the body directly: {main_body}"
    );
    assert!(main_body.contains("ret i32 %exit_code"), "{main_body}");
}

/// ADR 7.10.26r AC 3 (D3): under `off` the prologue calls Rust's
/// `tg_init_args_c` and never mentions the core's init, exactly as before;
/// the default switch is `off`, byte for byte. Under `all` it calls
/// `tg_native_init_args_c` with the same two loads. The row is off by
/// default, so `on` keeps Rust's init too.
#[test]
fn the_prologue_calls_the_init_the_switch_selects() {
    use crate::codegen::native_core::NativeCoreSwitch;
    let off = main_wrapper_ir(NativeCoreSwitch::Off);
    assert!(
        off.contains("call void @tg_init_args_c(i32 %argc, ptr %argv)"),
        "{off}"
    );
    assert!(
        off.contains("declare void @tg_init_args_c(i32, ptr)"),
        "{off}"
    );
    assert!(!off.contains("tg_native_init_args_c"), "{off}");
    assert_eq!(off, main_wrapper_ir(NativeCoreSwitch::default()));

    let all = main_wrapper_ir(NativeCoreSwitch::All);
    assert!(
        all.contains("call void @tg_native_init_args_c(i32 %argc, ptr %argv)"),
        "{all}"
    );
    assert!(
        all.contains("declare void @tg_native_init_args_c(i32, ptr)"),
        "{all}"
    );
    assert!(!all.contains("call void @tg_init_args_c("), "{all}");

    let on = main_wrapper_ir(NativeCoreSwitch::On);
    let row_on = albite_core::native_core::NATIVE_CORE_EXPORTS
        .iter()
        .find(|row| row.symbol == "tg_native_init_args_c")
        .expect("the init row")
        .on_by_default;
    assert_eq!(
        on.contains("call void @tg_native_init_args_c("),
        row_on,
        "{on}"
    );
}
