//! The runtime intrinsics' lowerings and the trap row's shim (ADR 7.10.26q
//! AC 4), over hand-built modules. The core's real emitted IR is asserted in
//! the bootstrap's `compile/native_core/tests.rs`.

use super::runtime::runtime_refusal;
use super::tests::{core_like_module, define_unit_entry};
use super::{NativeCoreExport, NativeCoreSwitch};
use crate::codegen::CodeGen;
use albite_core::native_core::RuntimeIntrinsic;
use inkwell::context::Context;
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;

fn string_type(context: &Context) -> StructType<'_> {
    context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            context.i64_type().into(),
        ],
        false,
    )
}

/// A `{}`-returning function taking one string, positioned at its entry, as
/// the core's extern stub is when it lowers an intrinsic.
fn stub<'ctx>(codegen: &mut CodeGen<'ctx>, context: &'ctx Context) -> FunctionValue<'ctx> {
    let unit = context.struct_type(&[], false);
    let fn_type = unit.fn_type(&[string_type(context).into()], false);
    let function = codegen.module.add_function("stub", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    codegen.compilation.current_fn = Some(function);
    function
}

fn ir(codegen: &CodeGen) -> String {
    codegen.module.print_to_string().to_string()
}

// 7.10.26q AC 4: `tg_core_write_stderr` is `write(2, ptr, len)` over the
// string's own two fields, and its `ssize_t` is the result.
#[test]
fn write_stderr_lowers_to_write_on_fd_two_with_the_strings_parts() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "core_run");
    codegen.set_native_core_run();
    let function = stub(&mut codegen, &context);
    let text = function.get_nth_param(0).expect("s");
    let lowered = codegen
        .try_compile_runtime_intrinsic("__c_tg_core_write_stderr", &[text])
        .expect("lowered")
        .expect("a runtime intrinsic");
    assert_eq!(lowered.into_int_value().get_type().get_bit_width(), 64);
    let unit = context.struct_type(&[], false).const_named_struct(&[]);
    codegen.builder.build_return(Some(&unit)).expect("ret");
    let text = ir(&codegen);
    assert!(text.contains("declare i64 @write(i32, ptr, i64)"), "{text}");
    assert!(
        text.contains("%str_ptr = extractvalue { ptr, i64 } %0, 0"),
        "{text}"
    );
    assert!(
        text.contains("%str_len = extractvalue { ptr, i64 } %0, 1"),
        "{text}"
    );
    assert!(
        text.contains("%core_written = call i64 @write(i32 2, ptr %str_ptr, i64 %str_len)"),
        "{text}"
    );
    assert!(codegen.module.verify().is_ok(), "{text}");
}

// 7.10.26q AC 4: `tg_core_abort` is `abort()` then `unreachable`, and the
// builder is left in a fresh block, so the stub's own `ret` is legal.
#[test]
fn abort_lowers_to_a_noreturn_abort_then_unreachable() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "core_run");
    codegen.set_native_core_run();
    stub(&mut codegen, &context);
    let lowered = codegen
        .try_compile_runtime_intrinsic("tg_core_abort", &[])
        .expect("lowered")
        .expect("a runtime intrinsic");
    codegen.builder.build_return(Some(&lowered)).expect("ret");
    let text = ir(&codegen);
    assert!(text.contains("call void @abort()\n  unreachable"), "{text}");
    assert!(text.contains("after_abort:"), "{text}");
    assert!(text.contains("declare void @abort() #0"), "{text}");
    assert!(text.contains("attributes #0 = { noreturn }"), "{text}");
    assert!(codegen.module.verify().is_ok(), "{text}");
}

/// A `{}`-returning function taking two `Nat`s, positioned at its entry, as
/// `tg_core_publish_args`'s extern stub is.
fn binary_nat_stub<'ctx>(
    codegen: &mut CodeGen<'ctx>,
    context: &'ctx Context,
) -> FunctionValue<'ctx> {
    let unit = context.struct_type(&[], false);
    let i64_type = context.i64_type();
    let fn_type = unit.fn_type(&[i64_type.into(), i64_type.into()], false);
    let function = codegen.module.add_function("publish_stub", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    codegen.compilation.current_fn = Some(function);
    function
}

// 7.10.26r AC 3 (D1, D2): `tg_core_publish_args` is the publish protocol —
// an acquire load of the vector, a branch on null, a release store of the
// count and an `acq_rel` `cmpxchg` of the vector from null, each `align 8` —
// over external globals that keep their C names after the forced prefix.
#[test]
fn publish_args_lowers_to_the_once_only_publish_protocol() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "core_run");
    codegen.set_native_core_run();
    let function = binary_nat_stub(&mut codegen, &context);
    let argc = function.get_nth_param(0).expect("argc");
    let argv = function.get_nth_param(1).expect("argv");
    let lowered = codegen
        .try_compile_runtime_intrinsic("__c_tg_core_publish_args", &[argc, argv])
        .expect("lowered")
        .expect("a runtime intrinsic");
    codegen.builder.build_return(Some(&lowered)).expect("ret");
    codegen.prefix_native_core_symbols();
    let text = ir(&codegen);
    for line in [
        "@tg_core_args_count = external global i64",
        "@tg_core_args_vector = external global ptr",
        "%args_published = load atomic ptr, ptr @tg_core_args_vector acquire, align 8",
        "%args_unpublished = icmp eq ptr %args_published, null",
        "br i1 %args_unpublished, label %publish_args, label %args_ready",
        "store atomic i64 %0, ptr @tg_core_args_count release, align 8",
        "%argv_ptr = inttoptr i64 %1 to ptr",
        "cmpxchg ptr @tg_core_args_vector, ptr null, ptr %argv_ptr acq_rel acquire, align 8",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert!(
        !text.contains("__albite_native_core__tg_core_args"),
        "{text}"
    );
    // A second call in the same module reuses the two declarations.
    let again = codegen.context.append_basic_block(function, "again");
    codegen.builder.position_at_end(again);
    codegen
        .try_compile_runtime_intrinsic("tg_core_publish_args", &[argc, argv])
        .expect("lowered twice");
    assert_eq!(ir(&codegen).matches("external global").count(), 2);
}

// 7.10.26q AC 2/AC 4: both are refused outside the core's run, naming the
// reservation; a wrong arity is an error; anything else is not one of them.
#[test]
fn runtime_intrinsics_are_refused_outside_the_core_run() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "program");
    let function = stub(&mut codegen, &context);
    let text = function.get_nth_param(0).expect("s");
    for (symbol, args) in [
        ("tg_core_write_stderr", vec![text]),
        ("tg_core_abort", Vec::<BasicValueEnum>::new()),
        ("tg_core_publish_args", Vec::<BasicValueEnum>::new()),
    ] {
        let refusal = codegen
            .try_compile_runtime_intrinsic(symbol, &args)
            .expect_err(symbol)
            .to_string();
        assert!(refusal.contains(symbol), "{refusal}");
        assert!(refusal.contains("reserved to the native core"), "{refusal}");
    }
    assert!(runtime_refusal(RuntimeIntrinsic::Abort).contains("ADR 7.10.26q D1"));
    assert!(codegen
        .try_compile_inline_intrinsic("tg_println", &[text])
        .expect("not an intrinsic")
        .is_none());
    codegen.set_native_core_run();
    let arity = codegen
        .try_compile_runtime_intrinsic("tg_core_abort", &[text])
        .expect_err("abort takes nothing")
        .to_string();
    assert!(arity.contains("not the intrinsic's shape"), "{arity}");
    let publish_arity = codegen
        .try_compile_runtime_intrinsic("tg_core_publish_args", &[text])
        .expect_err("publish takes two Nats")
        .to_string();
    assert!(
        publish_arity.contains("not the intrinsic's shape"),
        "{publish_arity}"
    );
    let inline = codegen
        .try_compile_inline_intrinsic("tg_core_write_stderr", &[text])
        .expect("lowered");
    assert!(
        inline.is_some(),
        "the combined dispatch reaches the runtime list"
    );
}

// 7.10.26q AC 4 (D3): the trap shim zero-extends its `i32`, calls the unary
// entry with a null environment, and ends in `unreachable`, because
// `tg_int_trap` is `noreturn`. A trap site that declared the shim first is
// reused, so the export is defined once and never as `.1`.
#[test]
fn the_trap_shim_widens_calls_int_trap_and_never_returns() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    codegen.set_native_core_switch(NativeCoreSwitch::All);
    let declared = codegen.int_trap_function();
    assert_eq!(declared.get_name().to_str(), Ok("tg_native_int_trap"));
    codegen.emit_native_core_exports().expect("shims");
    let shim = codegen
        .module
        .get_function("tg_native_int_trap")
        .expect("defined");
    assert_eq!(shim, declared);
    assert_eq!(shim.count_basic_blocks(), 1);
    let text = ir(&codegen);
    assert!(!text.contains("tg_native_int_trap.1"), "{text}");
    assert!(
        text.contains("%native_core_widened = zext i32 %0 to i64"),
        "{text}"
    );
    assert!(
        text.contains(
            "%native_core_result = call {} @int_trap(ptr null, i64 %native_core_widened)\n  unreachable"
        ),
        "{text}"
    );
    assert!(codegen.module.verify().is_ok(), "{text}");
    let again = codegen
        .emit_native_core_exports()
        .expect_err("defined twice");
    assert!(again.to_string().contains("already defined"), "{again}");
}

// 7.10.26q D3: a `void` export whose Rust declaration may return gets
// `ret void`; a same-width integer passes through unwidened.
#[test]
fn a_returning_void_shim_ends_in_ret_void() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "void_row");
    codegen.set_native_core_run();
    define_unit_entry(&codegen, &context, "note");
    let fn_type = context
        .void_type()
        .fn_type(&[context.i64_type().into()], false);
    codegen.module.add_function("rust_note", fn_type, None);
    let row = NativeCoreExport {
        symbol: "native_note",
        definition: "note",
        shadows: "rust_note",
        on_by_default: false,
    };
    codegen.emit_native_core_shim(&row).expect("shim");
    let text = ir(&codegen);
    assert!(
        text.contains("call {} @note(ptr null, i64 %0)\n  ret void"),
        "{text}"
    );
    assert!(!text.contains("zext"), "{text}");
    assert!(codegen.module.verify().is_ok(), "{text}");
}

// 7.10.26q D3: only a narrower integer is widened. A same-width or wider one
// comes back as the very value passed in — no `zext`, which would be invalid
// IR from a wider type — and so does a non-integer.
#[test]
fn only_a_narrower_integer_is_widened() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "widen");
    let i32_type = context.i32_type();
    let i64_type = context.i64_type();
    let fn_type = context
        .void_type()
        .fn_type(&[i32_type.into(), i64_type.into()], false);
    let function = codegen.module.add_function("probe", fn_type, None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    codegen.compilation.current_fn = Some(function);
    let narrow = function.get_nth_param(0).expect("i32");
    let wide = function.get_nth_param(1).expect("i64");

    let widened = codegen
        .widen_shim_argument(narrow, i64_type.into())
        .expect("widen");
    assert_eq!(widened.into_int_value().get_type().get_bit_width(), 64);
    assert_ne!(widened, narrow);
    for (param, wanted) in [
        (wide, i32_type.into()),
        (wide, i64_type.into()),
        (narrow, string_type(&context).into()),
    ] {
        let passed = codegen
            .widen_shim_argument(param, wanted)
            .expect("pass through");
        assert_eq!(passed, param);
    }
    let text = ir(&codegen);
    assert_eq!(text.matches("zext").count(), 1, "{text}");
}
