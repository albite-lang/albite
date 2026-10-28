//! IR-shape tests for the adoption lowering, `tg_bytes_from_string_grown`
//! (ADR 6.10.26n AC 2): the bound before the realloc, `old` and `new` equal
//! to the layout's block sizes, the unwritten tail (9.10.26n AC 4), the
//! trailer, the lazy declaration of the amortised grow (ADR 16.10.26j AC 3),
//! and refusal outside the core's run.

use super::grow::GROW_SYMBOL;
use super::tests::{emit_in, position, HANDLE, INDEX, TEXT};
use crate::codegen::CodeGen;
use albite_core::eval::IntTrapKind;
use albite_core::native_core::core_buffer_layout;
use inkwell::context::Context;
use inkwell::AddressSpace;

const GROW: &str = "tg_bytes_from_string_grown";

/// The adoption over a runtime string and a runtime `extra`, in the core's run.
fn emit_grow() -> String {
    emit_in(GROW, &[TEXT, INDEX], true)
}

fn too_large_trap() -> String {
    format!(
        "call void @tg_int_trap(i32 {})",
        IntTrapKind::ByteBufferTooLarge.code()
    )
}

// 6.10.26n AC 2: both halves of the bound, unsigned, precede the realloc,
// and the second compares against `MAX − len`, never `len + extra`.
#[test]
fn the_bound_precedes_the_realloc_and_cannot_wrap() {
    let text = emit_grow();
    let len_check = position(&text, "%grow_len_fits = icmp ule i64");
    let room = position(&text, "%grow_room = sub i64 4294967296,");
    let extra_check = position(&text, "%grow_extra_fits = icmp ule i64");
    let realloc = position(&text, &format!("call ptr @{GROW_SYMBOL}("));
    assert!(
        len_check < room && room < extra_check && extra_check < realloc,
        "{text}"
    );
    assert_eq!(text.matches(&too_large_trap()).count(), 2, "{text}");
    for signed in ["icmp slt", "icmp sle", "icmp sgt", "icmp sge"] {
        assert!(!text.contains(signed), "signed compare `{signed}`:\n{text}");
    }
}

// 6.10.26n AC 2: `old` and `new` are the layout's block sizes, not lengths.
#[test]
fn the_realloc_passes_the_old_and_new_block_sizes() {
    let text = emit_grow();
    assert!(
        text.contains(&format!(
            "@{GROW_SYMBOL}(ptr %grow_src, i64 %grow_old_block_size, i64 %grow_new_block_size)"
        )),
        "{text}"
    );
    assert!(text.contains("%grow_new_len = add nuw i64"), "{text}");
}

/// The adoption over a constant `(null, len)` string and constant `extra`,
/// so the layout arithmetic folds and the realloc's operands are numbers.
fn emit_constant_grow(len: u64, extra: u64) -> String {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "grow_constant");
    codegen.set_native_core_run();
    let function =
        codegen
            .module
            .add_function("probe", context.void_type().fn_type(&[], false), None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    codegen.compilation.current_fn = Some(function);
    let i64_type = context.i64_type();
    let null = context.ptr_type(AddressSpace::default()).const_null();
    let text = context.const_struct(&[null.into(), i64_type.const_int(len, false).into()], false);
    let args = [text.into(), i64_type.const_int(extra, false).into()];
    codegen
        .try_compile_byte_intrinsic(GROW, &args)
        .expect("lowers")
        .expect("is an intrinsic");
    codegen.builder.build_return(None).expect("return");
    assert!(function.verify(true));
    codegen.module.print_to_string().to_string()
}

// 6.10.26n AC 2: a constant length folds to `core_buffer_layout`'s answer for
// both the old block and the new one — the pure function the trailer obeys.
#[test]
fn constant_operands_fold_to_the_pure_layout() {
    for (len, extra) in [(7, 1), (1, 0), (8, 100), (15, 17)] {
        let text = emit_constant_grow(len, extra);
        let (old_block, _) = core_buffer_layout(len).expect("fits");
        let (new_block, new_offset) = core_buffer_layout(len + extra).expect("fits");
        assert!(
            text.contains(&format!(
                "@{GROW_SYMBOL}(ptr null, i64 {old_block}, i64 {new_block})"
            )),
            "len {len} extra {extra}:\n{text}"
        );
        assert!(
            text.contains(&format!("i64 {new_offset}")),
            "the trailer sits at the new layout's offset:\n{text}"
        );
        assert!(!text.contains("llvm.memset"), "no tail fill:\n{text}");
    }
}

// 6.10.26n AC 2, 9.10.26n AC 4: after the realloc the tail is left unwritten
// (no `llvm.memset`), and the two-field trailer `{len, live}` is written over
// the new block — no `data` store.
#[test]
fn the_tail_is_left_unwritten_and_the_trailer_is_written() {
    let text = emit_grow();
    assert!(!text.contains("llvm.memset"), "{text}");
    assert!(!text.contains("grow_tail"), "{text}");
    let realloc = position(&text, &format!("call ptr @{GROW_SYMBOL}("));
    let trailer = position(
        &text,
        "%bytes_trailer = getelementptr i8, ptr %grow_block, i64 %grow_new_header_offset",
    );
    assert!(realloc < trailer, "{text}");
    let after = &text[trailer..];
    assert!(after.contains("store i64 %grow_new_len, ptr"), "{text}");
    assert!(after.contains("store i64 1, ptr"), "{text}");
    assert!(!after.contains("store ptr"), "no data store:\n{text}");
    assert!(
        after.contains("ptrtoint ptr %bytes_trailer to i64"),
        "{text}"
    );
}

// 16.10.26j AC 3: the grow's callee is the runtime's amortised realloc.
#[test]
fn the_grow_symbol_is_the_amortised_realloc() {
    assert_eq!(GROW_SYMBOL, "__albite_realloc_amortised");
}

// 16.10.26j AC 3: the grow calls the amortised function with the layout's
// block sizes, and never `__albite_realloc` itself.
#[test]
fn the_grow_calls_the_amortised_realloc_and_never_the_exact_one() {
    let text = emit_grow();
    assert!(
        text.contains(
            "call ptr @__albite_realloc_amortised(ptr %grow_src, i64 %grow_old_block_size, i64 %grow_new_block_size)"
        ),
        "{text}"
    );
    assert!(!text.contains("@__albite_realloc("), "{text}");
}

// 6.10.26n AC 2: the module declares the grow's callee only if it grows.
#[test]
fn the_realloc_is_declared_only_by_a_module_that_grows() {
    let new_only = emit_in("tg_bytes_new", &[INDEX], true);
    assert!(!new_only.contains(GROW_SYMBOL), "{new_only}");
    let grows = emit_grow();
    assert_eq!(
        grows
            .matches(&format!("declare ptr @{GROW_SYMBOL}(ptr, i64, i64)"))
            .count(),
        1,
        "{grows}"
    );
}

// 6.10.26n AC 2: outside the core's run the code generator refuses it.
#[test]
fn the_adoption_is_refused_outside_the_core_run() {
    use super::convert::{intrinsic_allowed, reserved_refusal};
    use albite_core::eval::byte_intrinsics::ByteIntrinsic;
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "user_unit");
    let i64_type = context.i64_type();
    let null = context.ptr_type(AddressSpace::default()).const_null();
    let text = context.const_struct(&[null.into(), i64_type.const_zero().into()], false);
    let refusal = codegen
        .try_compile_byte_intrinsic(GROW, &[text.into(), i64_type.const_zero().into()])
        .expect_err("refused");
    assert!(
        refusal.to_string().contains("reserved to the native core"),
        "{refusal}"
    );
    assert!(!intrinsic_allowed(ByteIntrinsic::FromStringGrown, false));
    assert!(intrinsic_allowed(ByteIntrinsic::FromStringGrown, true));
    assert!(reserved_refusal(ByteIntrinsic::FromStringGrown).contains("6.10.26n"));
}

// 9.10.26n AC 4: in the core's run `data` is derived from the trailer, never
// loaded, and a `FIELD_DATA` access on the two-field trailer is an error.
#[test]
fn the_core_trailer_derives_data_and_refuses_a_data_field_access() {
    let blit = emit_in("tg_bytes_blit_string", &[HANDLE, INDEX, TEXT], true);
    assert!(blit.contains("%bytes_data_back = sub i64 0,"), "{blit}");
    assert!(!blit.contains("load ptr"), "{blit}");
    let user = emit_in("tg_bytes_blit_string", &[HANDLE, INDEX, TEXT], false);
    assert!(user.contains("load ptr"), "{user}");
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "core_unit");
    codegen.set_native_core_run();
    let function =
        codegen
            .module
            .add_function("probe", context.void_type().fn_type(&[], false), None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    let trailer = context.ptr_type(AddressSpace::default()).const_null();
    assert!(codegen
        .load_header_field_value(trailer, super::FIELD_DATA)
        .is_err());
    assert!(codegen
        .load_header_field_value(trailer, super::FIELD_LIVE)
        .is_ok());
}
