//! The export table, the shim and the forced prefix (ADR 4.10.26n AC 2, AC 3),
//! over a hand-built module standing in for the core's run.

use super::{is_native_core_export, NATIVE_CORE_EXPORTS, NATIVE_CORE_PREFIX};
use crate::codegen::CodeGen;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::types::StructType;
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

/// Add `name` with a body that returns its parameter `returned`.
fn define_passthrough<'ctx>(
    codegen: &CodeGen<'ctx>,
    context: &'ctx Context,
    name: &str,
    params: usize,
    returned: u32,
) {
    let string = string_type(context);
    let ptr = context.ptr_type(AddressSpace::default());
    let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![ptr.into()];
    for _ in 0..params {
        param_types.push(string.into());
    }
    let function = codegen
        .module
        .add_function(name, string.fn_type(&param_types, false), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let value = function.get_nth_param(returned).expect("param");
    builder.build_return(Some(&value)).expect("ret");
}

/// A module holding each row's `$direct` the shims can call.
pub(super) fn core_like_module(context: &Context) -> CodeGen<'_> {
    let mut codegen = CodeGen::new(context, "core_like");
    codegen.set_native_core_run();
    define_passthrough(&codegen, context, "string_concat$direct", 2, 1);
    define_passthrough(&codegen, context, "string_concat_owned$direct", 2, 1);
    define_unit_entry(&codegen, context, "int_trap");
    define_binary_nat_entry(&codegen, context, "init_args_c$direct");
    codegen
}

/// Add `name(ptr, i64, i64) -> {}` returning unit: a binary definition over
/// two `Nat`s, as `init_args_c$direct` is (7.10.26r D2).
fn define_binary_nat_entry<'ctx>(codegen: &CodeGen<'ctx>, context: &'ctx Context, name: &str) {
    let unit = context.struct_type(&[], false);
    let i64_type = context.i64_type();
    let fn_type = unit.fn_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            i64_type.into(),
            i64_type.into(),
        ],
        false,
    );
    let function = codegen.module.add_function(name, fn_type, None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    builder
        .build_return(Some(&unit.const_named_struct(&[])))
        .expect("ret");
}

// 7.10.26r AC 3 (D3): the init shim takes `tg_init_args_c`'s `void (i32,
// ptr)`, zero-extends `argc`, converts `argv` with `ptrtoint`, calls
// `init_args_c$direct` with a null environment and returns `void`.
#[test]
fn the_init_args_shim_widens_both_arguments_and_returns_void() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    codegen.emit_native_core_exports().expect("shims");
    let shim = codegen
        .module
        .get_function("tg_native_init_args_c")
        .expect("defined");
    let rust = codegen
        .module
        .get_function(super::INIT_ARGS_SYMBOL)
        .expect("declared before the shims");
    assert_eq!(shim.get_type(), rust.get_type());
    let ptr = context.ptr_type(AddressSpace::default());
    let expected = context
        .void_type()
        .fn_type(&[context.i32_type().into(), ptr.into()], false);
    assert_eq!(rust.get_type(), expected);
    let text = codegen.module.print_to_string().to_string();
    let body = text
        .split("define void @tg_native_init_args_c(i32 %0, ptr %1)")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .unwrap_or_else(|| panic!("no shim body:\n{text}"));
    for line in [
        "%native_core_widened = zext i32 %0 to i64",
        "%native_core_address = ptrtoint ptr %1 to i64",
        "call {} @\"init_args_c$direct\"(ptr null, i64 %native_core_widened, i64 %native_core_address)\n  ret void",
    ] {
        assert!(body.contains(line), "{line}\n{body}");
    }
    assert!(codegen.module.verify().is_ok(), "{text}");
}

/// Add `name(ptr, i64) -> {}` returning unit: a unary definition's entry,
/// which has no `$direct` (7.10.26q D3).
pub(super) fn define_unit_entry<'ctx>(codegen: &CodeGen<'ctx>, context: &'ctx Context, name: &str) {
    let unit = context.struct_type(&[], false);
    let fn_type = unit.fn_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            context.i64_type().into(),
        ],
        false,
    );
    let function = codegen.module.add_function(name, fn_type, None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    builder
        .build_return(Some(&unit.const_named_struct(&[])))
        .expect("ret");
}

// 4.10.26n AC 2, 6.10.26n AC 4: each row's signature is the one
// `codegen/mod.rs` declares for the Rust export it shadows, so the two cannot
// drift — `{ptr, i64} ({ptr, i64}, {ptr, i64})` for both concats.
#[test]
fn the_export_signature_is_the_shadowed_rust_declaration() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    codegen.emit_native_core_exports().expect("shim");
    for row in NATIVE_CORE_EXPORTS {
        let shim = codegen.module.get_function(row.symbol).expect("defined");
        let rust = codegen.module.get_function(row.shadows).expect("declared");
        assert_eq!(shim.get_type(), rust.get_type(), "{}", row.symbol);
    }
    let string = string_type(&context);
    let expected = string.fn_type(&[string.into(), string.into()], false);
    for shadowed in ["tg_string_concat", "tg_string_concat_owned"] {
        let rust = codegen.module.get_function(shadowed).expect("declared");
        assert_eq!(rust.get_type(), expected, "{shadowed}");
    }
    let owned = codegen
        .module
        .get_function("tg_native_string_concat_owned")
        .expect("the owned shim is defined");
    assert_eq!(owned.get_type(), expected);
    let text = codegen.module.print_to_string().to_string();
    assert!(
        text.contains("call { ptr, i64 } @\"string_concat_owned$direct\"(ptr null"),
        "{text}"
    );
}

// 4.10.26n AC 2: the shim's body is one call to `$direct` with a null
// environment, then `ret`.
#[test]
fn the_shim_calls_direct_with_a_null_environment_then_returns() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    codegen.emit_native_core_exports().expect("shim");
    let shim = codegen
        .module
        .get_function("tg_native_string_concat")
        .expect("defined");
    assert_eq!(shim.get_linkage(), Linkage::External);
    assert!(shim.verify(true));
    let text = codegen.module.print_to_string().to_string();
    assert!(
        text.contains(
            "call { ptr, i64 } @\"string_concat$direct\"(ptr null, { ptr, i64 } %0, { ptr, i64 } %1)"
        ),
        "{text}"
    );
    assert!(
        text.contains("ret { ptr, i64 } %native_core_result"),
        "{text}"
    );
    assert_eq!(shim.count_basic_blocks(), 1, "{text}");
}

#[test]
fn a_missing_or_misshapen_direct_is_an_error() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "no_core");
    let missing = codegen.emit_native_core_exports().expect_err("no $direct");
    assert!(missing
        .to_string()
        .contains("defines no string_concat$direct"));

    let mut codegen = CodeGen::new(&context, "wrong_arity");
    define_passthrough(&codegen, &context, "string_concat$direct", 1, 1);
    let arity = codegen.emit_native_core_exports().expect_err("one string");
    assert!(
        arity.to_string().contains("takes 2 parameter(s)"),
        "{arity}"
    );
}

// 4.10.26n AC 3: every externally linked definition is prefixed or a table
// row; declarations and private definitions keep their names.
#[test]
fn prefix_native_core_symbols_reaches_every_external_definition_but_the_rows() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    define_passthrough(&codegen, &context, "string_concat", 1, 1);
    define_passthrough(&codegen, &context, "__core_lambda_1", 1, 1);
    define_passthrough(&codegen, &context, "kept_private", 1, 1);
    codegen
        .module
        .get_function("kept_private")
        .expect("defined")
        .set_linkage(Linkage::Private);
    let data = codegen
        .module
        .add_global(context.i64_type(), None, "core_counter");
    data.set_initializer(&context.i64_type().const_zero());
    // A declared global the core only references (as the arena mode is) and
    // a private constant (as the UTF-8 table is) both keep their names.
    codegen
        .module
        .add_global(context.i32_type(), None, "runtime_declared_global");
    let private = codegen
        .module
        .add_global(context.i64_type(), None, "core_private_constant");
    private.set_initializer(&context.i64_type().const_zero());
    private.set_linkage(Linkage::Private);
    codegen.emit_native_core_exports().expect("shim");
    codegen.prefix_native_core_symbols();

    let defined: Vec<String> = codegen
        .module
        .get_functions()
        .filter(|f| f.count_basic_blocks() > 0 && f.get_linkage() != Linkage::Private)
        .map(|f| f.get_name().to_string_lossy().into_owned())
        .collect();
    // Four definitions, the trap's entry, the init's `$direct` and four shims.
    assert_eq!(defined.len(), 10, "{defined:?}");
    for name in &defined {
        assert!(
            name.starts_with(NATIVE_CORE_PREFIX) || is_native_core_export(name),
            "{name} escaped the prefix"
        );
    }
    assert!(codegen.module.get_function("kept_private").is_some());
    assert!(codegen.module.get_function("tg_string_concat").is_some());
    let renamed_data = format!("{NATIVE_CORE_PREFIX}core_counter");
    assert!(codegen.module.get_global(&renamed_data).is_some());
    assert!(codegen
        .module
        .get_global("runtime_declared_global")
        .is_some());
    assert!(codegen.module.get_global("core_private_constant").is_some());
    // The shim still calls the renamed `$direct`.
    assert!(codegen
        .module
        .get_function("tg_native_string_concat")
        .is_some());
    let text = codegen.module.print_to_string().to_string();
    assert!(
        text.contains(&format!("@\"{NATIVE_CORE_PREFIX}string_concat$direct\"")),
        "{text}"
    );
}

// 4.10.26n D6, 6.10.26n AC 4: each row's shim is declared on first use, with
// its Rust export's type, and only once; a symbol that is no row is `None`.
#[test]
fn each_native_export_is_declared_lazily_with_the_rust_type() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "lazy");
    // Only a trap site declares `tg_int_trap`; the trap row needs it.
    assert!(codegen
        .native_export_function("tg_native_int_trap")
        .is_none());
    codegen.rust_int_trap_function();
    // Likewise only the main wrapper declares `tg_init_args_c` (7.10.26r).
    assert!(codegen
        .native_export_function("tg_native_init_args_c")
        .is_none());
    codegen.rust_init_args_function();
    for row in NATIVE_CORE_EXPORTS {
        assert!(codegen.module.get_function(row.symbol).is_none());
        let first = codegen.native_export_function(row.symbol).expect("a row");
        let second = codegen.native_export_function(row.symbol).expect("a row");
        assert_eq!(first, second);
        assert_eq!(first.get_name().to_str(), Ok(row.symbol));
        let rust = codegen.module.get_function(row.shadows).expect("declared");
        assert_eq!(first.get_type(), rust.get_type());
        assert_eq!(first.count_basic_blocks(), 0);
    }
    assert!(codegen.native_export_function("tg_string_concat").is_none());
}

// 6.10.26n AC 4: the switch's three settings and what each enables and links.
#[test]
fn the_switch_maps_onto_the_rows_it_enables() {
    use super::{EnabledRows, NativeCoreSwitch};
    assert_eq!(NativeCoreSwitch::Off.enabled_rows(), EnabledRows::None);
    assert_eq!(NativeCoreSwitch::On.enabled_rows(), EnabledRows::Defaults);
    assert_eq!(NativeCoreSwitch::All.enabled_rows(), EnabledRows::Every);
    assert!(!NativeCoreSwitch::Off.links_core());
    assert!(NativeCoreSwitch::On.links_core());
    assert!(NativeCoreSwitch::All.links_core());
}

// 7.10.26p AC 2 (D2): the core's pipeline folds each shim's `$direct` call
// into the shim, and every shim keeps its name, its external linkage and the
// shadowed Rust export's type.
#[test]
fn the_pipeline_inlines_the_core_and_keeps_every_shim() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    codegen.emit_native_core_exports().expect("shim");
    // The pipeline drops the now-unused Rust declarations, so their types
    // are read first.
    let shadowed: Vec<_> = NATIVE_CORE_EXPORTS
        .iter()
        .map(|row| {
            codegen
                .module
                .get_function(row.shadows)
                .expect("declared")
                .get_type()
        })
        .collect();
    codegen.optimize_native_core().expect("pipeline");
    for (row, rust_type) in NATIVE_CORE_EXPORTS.iter().zip(shadowed) {
        let shim = codegen.module.get_function(row.symbol).expect("kept");
        assert_eq!(shim.get_type(), rust_type, "{}", row.symbol);
        assert_eq!(shim.get_linkage(), Linkage::External, "{}", row.symbol);
    }
    let text = codegen.module.print_to_string().to_string();
    assert!(
        !text.contains("call { ptr, i64 } @\"string_concat"),
        "{text}"
    );
}

// 7.10.26p D2: the module's data layout and the pipeline's target machine
// come from one constructor, so the pipeline never runs against a layout the
// module was not given.
#[test]
fn the_module_layout_is_the_pipelines_target_machine() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "layout");
    let machine =
        crate::codegen::backend::native_target_machine(inkwell::OptimizationLevel::Default)
            .expect("host target");
    assert_eq!(
        codegen.module.get_data_layout().as_str(),
        machine.get_target_data().get_data_layout().as_str()
    );
    assert_eq!(codegen.module.get_triple(), machine.get_triple());
    assert_eq!(super::NATIVE_CORE_PIPELINE, "default<O2>");
}

// 7.10.26p D2: the object writer takes the factored target machine and still
// writes an object — the core's optimised module included.
#[test]
fn the_optimised_core_writes_an_object_through_the_shared_machine() {
    let context = Context::create();
    let mut codegen = core_like_module(&context);
    codegen.emit_native_core_exports().expect("shim");
    codegen.optimize_native_core().expect("pipeline");
    let path = std::env::temp_dir().join(format!(
        "albite_codegen_native_core_{}.o",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    codegen
        .write_object_file_with_opt(&path, inkwell::OptimizationLevel::None)
        .expect("object");
    let written = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let _ = std::fs::remove_file(&path);
    assert!(written > 0, "no object written");
}
