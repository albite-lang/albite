//! IR tests for the word lowering of small sums on `$direct_mt` entries
//! (ADR 2.10.26h AC 1–3). Host LLVM; each module is verified.

use super::{ParamLowering, ReturnLowering};
use crate::codegen::CodeGen;
use inkwell::context::Context;
use inkwell::types::{BasicType, FunctionType, StructType};
use inkwell::values::{AnyValue, BasicValueEnum};
use inkwell::AddressSpace;

/// The tagged-union blob `{ i32, [N x i8] }` with an `N`-byte payload.
pub(super) fn blob(ctx: &Context, payload: u32) -> StructType<'_> {
    ctx.struct_type(
        &[
            ctx.i32_type().into(),
            ctx.i8_type().array_type(payload).into(),
        ],
        false,
    )
}

/// `ret @f$direct(ptr env, params…)`.
pub(super) fn direct_type<'ctx>(
    ctx: &'ctx Context,
    ret: inkwell::types::BasicTypeEnum<'ctx>,
    params: &[inkwell::types::BasicTypeEnum<'ctx>],
) -> FunctionType<'ctx> {
    let mut all = vec![ctx.ptr_type(AddressSpace::default()).into()];
    all.extend(
        params
            .iter()
            .map(|p| inkwell::types::BasicMetadataTypeEnum::from(*p)),
    );
    ret.fn_type(&all, false)
}

/// The printed `define`/`declare` line of `name`.
pub(super) fn signature_line(cg: &CodeGen<'_>, name: &str) -> String {
    let ir = cg.module.print_to_string().to_string();
    let needle = format!("@\"{name}\"(");
    ir.lines()
        .find(|l| (l.starts_with("define") || l.starts_with("declare")) && l.contains(&needle))
        .unwrap_or_else(|| panic!("no signature for {name} in:\n{ir}"))
        .to_string()
}

/// The text of function `name` in the module.
fn function_ir(cg: &CodeGen<'_>, name: &str) -> String {
    cg.module
        .get_function(name)
        .unwrap_or_else(|| panic!("{name} not in module"))
        .print_to_string()
        .to_string()
}

/// `pick(c: Color, n: Nat) -> Color` with `Color` a three-constructor enum:
/// declared, with its shim compiled and a `$direct_mt` body that binds its
/// params and returns `c`.
fn declare_pick(ctx: &Context) -> CodeGen<'_> {
    let mut cg = CodeGen::new(ctx, "pick");
    let color = blob(ctx, 0);
    let fn_type = direct_type(ctx, color.into(), &[color.into(), ctx.i64_type().into()]);
    cg.module.add_function("pick$direct", fn_type, None);
    let lowerings = cg.declare_decomposed_entry("pick", fn_type).unwrap();
    assert_eq!(
        lowerings,
        vec![ParamLowering::Word(32), ParamLowering::Passthrough]
    );
    cg.direct_calls.set_decompose_map("pick", lowerings);
    cg
}

/// Open `pick$direct_mt`'s entry and unpack its word param into a blob.
fn enter_pick_body<'ctx>(cg: &mut CodeGen<'ctx>, ctx: &'ctx Context) -> BasicValueEnum<'ctx> {
    let mt_fn = cg.module.get_function("pick$direct_mt").unwrap();
    cg.compilation.current_fn = Some(mt_fn);
    cg.direct_calls.current_entry = Some("pick$direct_mt".to_string());
    let entry = ctx.append_basic_block(mt_fn, "entry");
    cg.builder.position_at_end(entry);
    let word = mt_fn.get_nth_param(1).unwrap().into_int_value();
    cg.unpack_word(word, blob(ctx, 0).into()).unwrap()
}

/// ADR 2.10.26h AC1: for `N` from 0 to 4, the target-data store size of the
/// blob is the census's `blob_bytes` — `4 + N` rounded up to 4.
#[test]
fn store_size_agrees_with_blob_bytes() {
    let ctx = Context::create();
    let cg = CodeGen::new(&ctx, "sizes");
    for (payload, blob_bytes) in [(0, 4), (1, 8), (2, 8), (3, 8), (4, 8)] {
        assert_eq!(
            cg.types.type_size(blob(&ctx, payload).into()),
            blob_bytes,
            "N = {payload}"
        );
    }
}

/// ADR 2.10.26h AC2: only blobs of at most 8 bytes are words; a 12-byte blob
/// stays indirect, a flattenable struct decomposes, a `ptr` passes through.
#[test]
fn only_blobs_of_eight_bytes_or_less_are_words() {
    let ctx = Context::create();
    let cg = CodeGen::new(&ctx, "classify");
    let i64_ty = ctx.i64_type();
    let ptr = ctx.ptr_type(AddressSpace::default());
    let pair = ctx.struct_type(&[i64_ty.into(), ptr.into()], false);
    let fn_type = direct_type(
        &ctx,
        i64_ty.into(),
        &[
            blob(&ctx, 0).into(),
            blob(&ctx, 1).into(),
            blob(&ctx, 4).into(),
            blob(&ctx, 8).into(),
            pair.into(),
            ptr.into(),
        ],
    );
    assert_eq!(
        cg.compute_param_lowerings(fn_type),
        vec![
            ParamLowering::Word(32),
            ParamLowering::Word(64),
            ParamLowering::Word(64),
            ParamLowering::Indirect,
            ParamLowering::Decompose(2),
            ParamLowering::Passthrough,
        ]
    );
    // A blob that is not `{ i32, [N x i8] }` is never a word, however small.
    let not_a_blob = ctx.struct_type(&[i64_ty.into()], false);
    assert_eq!(cg.small_sum_word(not_a_blob.into()), None);
    assert_eq!(cg.small_sum_word(i64_ty.into()), None);
}

/// ADR 2.10.26h AC2: a small return is a word with no sret slot — and earns a
/// `$direct_mt` on its own — while a 12-byte return keeps sret.
#[test]
fn a_small_return_is_a_word_not_sret() {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, "returns");
    let i64_ty = ctx.i64_type();
    let small = direct_type(&ctx, blob(&ctx, 0).into(), &[i64_ty.into()]);
    cg.module.add_function("small$direct", small, None);
    let lowered = cg.declare_decomposed_entry("small", small);
    assert_eq!(lowered, Some(vec![ParamLowering::Passthrough]));
    let line = signature_line(&cg, "small$direct_mt");
    assert!(line.starts_with("declare i32 "), "got: {line}");
    assert!(!line.contains("sret"), "got: {line}");
    assert_eq!(
        cg.musttail_decisions().last().map(|d| (d.sret, d.word_ret)),
        Some((false, Some(32)))
    );

    let large = direct_type(&ctx, blob(&ctx, 8).into(), &[i64_ty.into()]);
    cg.module.add_function("large$direct", large, None);
    cg.declare_decomposed_entry("large", large);
    let line = signature_line(&cg, "large$direct_mt");
    assert!(line.starts_with("declare void "), "got: {line}");
    assert!(line.contains("sret({ i32, [8 x i8] })"), "got: {line}");
    let _ = ReturnLowering::Sret;
}

/// ADR 2.10.26h AC3: `pick$direct_mt` takes an `i32` after `env` with no
/// buffer slot, returns `i32`, and its shim packs through `sum_word`.
#[test]
fn pick_mt_takes_and_returns_a_word() {
    let ctx = Context::create();
    let mut cg = declare_pick(&ctx);
    let color = enter_pick_body(&mut cg, &ctx);
    cg.emit_word_return(color, blob(&ctx, 0).into(), ctx.i32_type())
        .unwrap();
    let mt_fn = cg.module.get_function("pick$direct_mt").unwrap();
    let param_map = cg.direct_calls.decompose_map("pick").unwrap().clone();
    cg.compile_decompose_shim("pick", "pick$direct_mt", mt_fn, &param_map)
        .unwrap();
    assert!(
        cg.module.verify().is_ok(),
        "{}",
        cg.module.print_to_string()
    );

    let line = signature_line(&cg, "pick$direct_mt");
    assert!(
        line.starts_with("define i32 @\"pick$direct_mt\"(ptr"),
        "got: {line}"
    );
    assert!(line.contains("i32 %"), "word slot after env, got: {line}");
    assert!(!line.contains("sret("), "got: {line}");
    assert!(!line.contains("dereferenceable(4)"), "got: {line}");

    let shim = function_ir(&cg, "pick$direct");
    assert!(shim.contains("%sum_word = alloca i32, align 8"), "{shim}");
    // D2: both stores into the slot carry its 8-byte alignment, not the
    // `i32`'s or the blob's natural 4.
    assert!(
        shim.contains("store i32 0, ptr %sum_word, align 8"),
        "{shim}"
    );
    assert!(
        shim.contains("store { i32, [0 x i8] } %1, ptr %sum_word, align 8"),
        "{shim}"
    );
    assert!(!shim.contains("indirect_buf"), "{shim}");
    assert!(!shim.contains("sret_buf"), "{shim}");
    assert!(shim.contains("call i32 @\"pick$direct_mt\"(ptr"), "{shim}");
}

/// ADR 2.10.26h AC3: a word param with a scalar return — the shim packs the
/// param and hands the `i64` result back as it came, with no unpack.
#[test]
fn a_scalar_result_is_not_unpacked() {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, "scalar");
    let fn_type = direct_type(&ctx, ctx.i64_type().into(), &[blob(&ctx, 0).into()]);
    cg.module.add_function("scalar$direct", fn_type, None);
    let lowerings = cg.declare_decomposed_entry("scalar", fn_type).unwrap();
    let mt_fn = cg.module.get_function("scalar$direct_mt").unwrap();
    cg.builder
        .position_at_end(ctx.append_basic_block(mt_fn, "entry"));
    cg.builder
        .build_return(Some(&ctx.i64_type().const_zero()))
        .unwrap();
    cg.compile_decompose_shim("scalar", "scalar$direct_mt", mt_fn, &lowerings)
        .unwrap();
    assert!(
        cg.module.verify().is_ok(),
        "{}",
        cg.module.print_to_string()
    );
    let shim = function_ir(&cg, "scalar$direct");
    assert!(shim.contains("sum_word.pack"), "{shim}");
    assert!(!shim.contains("sum_word.unpack"), "{shim}");
    assert!(shim.contains("ret i64 %shim_call"), "{shim}");
}

/// ADR 2.10.26h AC3: the self-edge packs a fresh word and musttails it, then
/// returns the callee's word as it came.
#[test]
fn the_self_edge_musttails_a_word() {
    let ctx = Context::create();
    let mut cg = declare_pick(&ctx);
    let color = enter_pick_body(&mut cg, &ctx);
    let n = ctx.i64_type().const_int(3, false).into();
    let placeholder = cg
        .try_emit_decomposed_musttail("pick", &[color, n])
        .unwrap()
        .expect("a word self-edge emits");
    // D4: the epilogue's dead-block placeholder is `i32 0`, never a blob.
    assert_eq!(placeholder.get_type(), ctx.i32_type().into());
    cg.emit_word_return(placeholder, blob(&ctx, 0).into(), ctx.i32_type())
        .unwrap();
    assert!(
        cg.module.verify().is_ok(),
        "{}",
        cg.module.print_to_string()
    );

    let body = function_ir(&cg, "pick$direct_mt");
    let call = body
        .lines()
        .position(|l| l.contains("musttail call i32 @\"pick$direct_mt\"(ptr null, i32"))
        .unwrap_or_else(|| panic!("no word musttail in:\n{body}"));
    let next = body.lines().nth(call + 1).unwrap_or_default();
    assert!(next.trim_start().starts_with("ret i32 "), "got: {next}");
}

/// ADR 2.10.26h AC3/D4: an early `return` of a 1-byte-payload sum (whose
/// blob has padding) zeroes the `sum_word` slot before storing the blob, and
/// returns `i64` — never `ret void`, never a struct.
#[test]
fn an_early_return_packs_the_word() {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, "flag");
    let flag = blob(&ctx, 1);
    let fn_type = direct_type(&ctx, flag.into(), &[ctx.i64_type().into()]);
    cg.module.add_function("flag$direct", fn_type, None);
    cg.declare_decomposed_entry("flag", fn_type);
    let mt_fn = cg.module.get_function("flag$direct_mt").unwrap();
    let entry = ctx.append_basic_block(mt_fn, "entry");
    cg.builder.position_at_end(entry);
    let on_true = flag.const_named_struct(&[
        ctx.i32_type().const_int(1, false).into(),
        ctx.i8_type()
            .const_array(&[ctx.i8_type().const_int(1, false)])
            .into(),
    ]);
    let ret_ty = mt_fn.get_type().get_return_type().unwrap();
    let word = cg
        .early_return_value(mt_fn, ret_ty, on_true.into())
        .unwrap();
    cg.builder.build_return(Some(&word)).unwrap();
    assert!(
        cg.module.verify().is_ok(),
        "{}",
        cg.module.print_to_string()
    );

    let body = function_ir(&cg, "flag$direct_mt");
    let zero = body
        .find("store i64 0, ptr %sum_word")
        .unwrap_or_else(|| panic!("{body}"));
    let store = body
        .find("store { i32, [1 x i8] }")
        .unwrap_or_else(|| panic!("{body}"));
    assert!(
        zero < store,
        "the slot is zeroed before the blob lands:\n{body}"
    );
    assert!(body.contains("ret i64 "), "{body}");
    assert!(!body.contains("ret void"), "{body}");

    // Outside a `$direct_mt`, an integer-returning function keeps cast_to_type.
    let plain = cg
        .module
        .add_function("plain", ctx.i64_type().fn_type(&[], false), None);
    cg.builder
        .position_at_end(ctx.append_basic_block(plain, "entry"));
    let kept = cg
        .early_return_value(plain, ctx.i64_type().into(), on_true.into())
        .unwrap();
    cg.builder.build_return(Some(&kept)).unwrap();
    assert!(!function_ir(&cg, "plain").contains("sum_word"));
}

/// ADR 2.10.26h AC3/D4: one arm tail-calls (leaving the epilogue's dead
/// block) and the other returns a constructed blob; both returns are words
/// and the module verifies.
#[test]
fn a_tail_called_arm_merges_with_a_word() {
    let ctx = Context::create();
    let mut cg = declare_pick(&ctx);
    let color = enter_pick_body(&mut cg, &ctx);
    let mt_fn = cg.module.get_function("pick$direct_mt").unwrap();
    let (tail_arm, build_arm) = (
        ctx.append_basic_block(mt_fn, "tail_arm"),
        ctx.append_basic_block(mt_fn, "build_arm"),
    );
    let n = mt_fn.get_nth_param(2).unwrap().into_int_value();
    let is_zero = cg
        .builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            n,
            ctx.i64_type().const_zero(),
            "z",
        )
        .unwrap();
    cg.builder
        .build_conditional_branch(is_zero, build_arm, tail_arm)
        .unwrap();

    cg.builder.position_at_end(tail_arm);
    let next = ctx.i64_type().const_int(1, false).into();
    let placeholder = cg
        .try_emit_decomposed_musttail("pick", &[color, next])
        .unwrap()
        .unwrap();
    cg.emit_word_return(placeholder, blob(&ctx, 0).into(), ctx.i32_type())
        .unwrap();

    cg.builder.position_at_end(build_arm);
    let blue = blob(&ctx, 0).const_named_struct(&[
        ctx.i32_type().const_int(2, false).into(),
        ctx.i8_type().const_array(&[]).into(),
    ]);
    cg.emit_word_return(blue.into(), blob(&ctx, 0).into(), ctx.i32_type())
        .unwrap();
    assert!(
        cg.module.verify().is_ok(),
        "{}",
        cg.module.print_to_string()
    );
    let body = function_ir(&cg, "pick$direct_mt");
    assert!(!body.contains("ret {"), "{body}");
    assert!(!body.contains("ret void"), "{body}");
}
