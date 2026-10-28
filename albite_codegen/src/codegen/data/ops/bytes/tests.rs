//! IR-shape tests for the inline byte-buffer lowering (ADR 3.10.26n AC 4).
//!
//! Each emits one intrinsic into a fresh function whose operands are its
//! parameters (constants would fold away the compares under test), verifies
//! the function, and reads the module text back.

use super::utf8::UTF8_LEAD_TABLE;
use crate::codegen::CodeGen;
use inkwell::context::Context;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

/// Emit `symbol` over `(handle, index, value, text)` parameters picked by
/// `pick`, return void, verify, and hand back the module text.
fn emit(symbol: &str, pick: &[usize]) -> String {
    emit_in(symbol, pick, false)
}

/// [`emit`], in the native core's own run when `core_run` (ADR 4.10.26n D5).
pub(super) fn emit_in(symbol: &str, pick: &[usize], core_run: bool) -> String {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "bytes_shape");
    if core_run {
        codegen.set_native_core_run();
    }
    let i64_type = context.i64_type();
    let string_type = context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            i64_type.into(),
        ],
        false,
    );
    let fn_type = context.void_type().fn_type(
        &[
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
            string_type.into(),
        ],
        false,
    );
    let function = codegen.module.add_function("probe", fn_type, None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    codegen.compilation.current_fn = Some(function);
    let args: Vec<BasicValueEnum> = pick
        .iter()
        .map(|&n| function.get_nth_param(n as u32).expect("param"))
        .collect();
    let lowered = codegen
        .try_compile_byte_intrinsic(symbol, &args)
        .expect("lowers")
        .expect("is an intrinsic");
    let _ = lowered;
    codegen.builder.build_return(None).expect("return");
    assert!(function.verify(true), "{symbol} must verify");
    codegen.module.print_to_string().to_string()
}

pub(super) const HANDLE: usize = 0;
pub(super) const INDEX: usize = 1;
const VALUE: usize = 2;
pub(super) const TEXT: usize = 3;

pub(super) fn position(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in:\n{text}"))
}

/// A macro rather than a helper fn: the mutation engine empties a helper's
/// body, which would leave the test calling it asserting nothing.
macro_rules! assert_no_signed_compare {
    ($text:expr) => {
        for signed in ["icmp slt", "icmp sle", "icmp sgt", "icmp sge"] {
            assert!(
                !$text.contains(signed),
                "signed compare `{signed}`:\n{}",
                $text
            );
        }
    };
}

#[test]
fn no_intrinsic_is_ever_declared_to_the_linker() {
    for (symbol, pick) in [
        ("tg_bytes_new", &[INDEX][..]),
        ("tg_bytes_len", &[HANDLE]),
        ("tg_bytes_get", &[HANDLE, INDEX]),
        ("__c_tg_bytes_set", &[HANDLE, INDEX, VALUE]),
        ("tg_bytes_blit_string", &[HANDLE, INDEX, TEXT]),
        ("tg_bytes_into_string", &[HANDLE]),
    ] {
        let text = emit(symbol, pick);
        assert!(!text.contains("@tg_bytes_"), "{symbol} declared:\n{text}");
        assert_no_signed_compare!(text);
    }
}

#[test]
fn get_checks_liveness_then_an_unsigned_index_before_the_load_and_zero_extends() {
    let text = emit("tg_bytes_get", &[HANDLE, INDEX]);
    let live = position(&text, "call void @tg_int_trap(i32 10)");
    let compare = position(&text, "icmp ult i64");
    let branch = position(&text, "br i1 %bytes_index_in_range, label %bytes_cont");
    let load = position(&text, "load i8, ptr");
    assert!(compare < branch && branch < load, "{text}");
    assert!(live < load, "{text}");
    assert!(text.contains("call void @tg_int_trap(i32 8)"), "{text}");
    assert!(text[load..].contains("zext i8"), "{text}");
}

#[test]
fn set_checks_the_index_before_the_value_and_both_before_the_store() {
    let text = emit("tg_bytes_set", &[HANDLE, INDEX, VALUE]);
    let index_check = position(&text, "icmp ult i64");
    let value_check = position(&text, ", 255");
    let store = position(&text, "store i8");
    assert!(index_check < value_check && value_check < store, "{text}");
    assert!(text.contains("icmp ule i64"), "{text}");
    assert!(text.contains("call void @tg_int_trap(i32 9)"), "{text}");
    assert!(text.contains("call void @tg_int_trap(i32 8)"), "{text}");
}

#[test]
fn blit_bounds_at_then_the_room_left_then_copies_with_memcpy() {
    let text = emit("tg_bytes_blit_string", &[HANDLE, INDEX, TEXT]);
    let at = position(&text, "%bytes_at_fits = icmp ule i64");
    let room = position(&text, "%bytes_room = sub i64");
    let fits = position(&text, "%bytes_text_fits = icmp ule i64");
    let copy = position(&text, "call void @llvm.memcpy");
    assert!(at < room && room < fits && fits < copy, "{text}");
    assert!(
        !text.contains("add i64 %1"),
        "the bound must not add `at`:\n{text}"
    );
}

#[test]
fn new_refuses_above_the_ceiling_then_allocates_header_and_data_apart() {
    let text = emit("tg_bytes_new", &[INDEX]);
    let ceiling = position(&text, "icmp ule i64 %1, 4294967296");
    let header = position(&text, "bytes_header");
    let data = position(&text, "bytes_data");
    assert!(ceiling < header && header < data, "{text}");
    assert!(text.contains("call void @tg_int_trap(i32 11)"), "{text}");
    assert!(text.contains("llvm.memset"), "{text}");
    assert!(text.contains("ptrtoint ptr"), "{text}");
    // Two allocations under each arm of the arena-mode branch: header, data.
    // ADR 7.10.26s AC 5: a user-run buffer is a handle's memory, so both arena
    // arms name the region-bypassing symbol and neither the region-aware one.
    assert_eq!(text.matches("call ptr @malloc").count(), 2, "{text}");
    assert_eq!(
        text.matches("call ptr @__albite_alloc_parent(").count(),
        2,
        "{text}"
    );
    assert!(!text.contains("call ptr @__albite_alloc("), "{text}");
    assert!(!text.contains("bytes_trailer"), "{text}");
}

// 6.10.26m AC 2: in the core's run, ONE allocation on each arm of the mode
// branch, the header a trailer at `core_buffer_layout`'s offset, and the
// zero-fill over the data region only.
#[test]
fn new_in_the_core_run_makes_one_block_with_the_header_as_its_trailer() {
    let text = emit_in("tg_bytes_new", &[INDEX], true);
    assert_eq!(text.matches("call ptr @malloc").count(), 1, "{text}");
    assert_eq!(
        text.matches("call ptr @__albite_alloc(").count(),
        1,
        "{text}"
    );
    // ADR 7.10.26s AC 5: the core's single block stays region-aware.
    assert!(!text.contains("__albite_alloc_parent"), "{text}");
    let ceiling = position(&text, "icmp ule i64 %1, 4294967296");
    let padded = position(&text, "%bytes_padded = add nuw i64 %1, 8");
    let offset = position(&text, "%bytes_header_offset = and i64 %bytes_padded, -8");
    // 9.10.26n D1: the trailer is `{len, live}`, 16 bytes, with no `data`.
    let size = position(
        &text,
        "%bytes_block_size = add nuw i64 %bytes_header_offset, 16",
    );
    assert!(!text.contains("store ptr"), "{text}");
    let trailer = position(&text, "%bytes_trailer = getelementptr i8, ptr");
    assert!(
        ceiling < padded && padded < offset && offset < size && size < trailer,
        "{text}"
    );
    assert!(
        text.contains(", i8 0, i64 %bytes_header_offset, i1 false)"),
        "{text}"
    );
    assert!(
        text.contains("ptrtoint ptr %bytes_trailer to i64"),
        "{text}"
    );
}

// 6.10.26m AC 2: a constant length folds to `core_buffer_layout`'s answer, so
// the emitter and the pure function cannot drift apart.
#[test]
fn a_constant_length_folds_to_the_pure_layout() {
    use albite_core::native_core::core_buffer_layout;
    for len in [0_u64, 7, 8, 100] {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "bytes_layout");
        codegen.set_native_core_run();
        let function =
            codegen
                .module
                .add_function("probe", context.void_type().fn_type(&[], false), None);
        codegen
            .builder
            .position_at_end(context.append_basic_block(function, "entry"));
        codegen.compilation.current_fn = Some(function);
        let len_value = context.i64_type().const_int(len, false).into();
        codegen
            .try_compile_byte_intrinsic("tg_bytes_new", &[len_value])
            .expect("lowers")
            .expect("is an intrinsic");
        codegen.builder.build_return(None).expect("return");
        let text = codegen.module.print_to_string().to_string();
        let (block, offset) = core_buffer_layout(len).expect("fits");
        assert!(
            text.contains(&format!("@malloc(i64 {block})")),
            "len {len}:\n{text}"
        );
        assert!(
            text.contains(&format!("i64 {offset}")) && text.contains("bytes_trailer"),
            "len {len}:\n{text}"
        );
    }
}

#[test]
fn into_string_validates_utf8_from_a_private_table_then_clears_live() {
    let text = emit("tg_bytes_into_string", &[HANDLE]);
    assert!(
        text.contains(&format!(
            "@{UTF8_LEAD_TABLE} = private constant [256 x i32]"
        )),
        "{text}"
    );
    assert!(text.contains("call void @tg_int_trap(i32 12)"), "{text}");
    assert!(text.contains("call void @tg_int_trap(i32 10)"), "{text}");
    let validated = position(&text, "utf8_done:");
    let cleared = position(&text, "store i64 0, ptr");
    assert!(validated < cleared, "{text}");
}

#[test]
fn a_non_intrinsic_is_declined_and_a_wrong_arity_is_an_error() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "decline");
    assert!(matches!(
        codegen.try_compile_byte_intrinsic("tg_string_concat", &[]),
        Ok(None)
    ));
    assert!(codegen
        .try_compile_byte_intrinsic("tg_bytes_get", &[])
        .is_err());
}

/// The UTF-8 walk's shape, pinned by constant: it leaves for `utf8_step` after
/// zero, one and two continuations (never three — the fourth byte falls
/// through), checks the FIRST continuation against the lead's narrowed range
/// and the other two against `80..=BF`. A walk that compared the count to the
/// wrong constant, or swapped which byte gets the narrowed range, still emits
/// every instruction kind the test above looks for.
#[test]
fn the_utf8_walk_leaves_at_counts_zero_one_two_and_narrows_only_the_first_continuation() {
    let text = emit("tg_bytes_into_string", &[HANDLE]);
    for count in 0..3 {
        let compare = format!("icmp eq i64 %utf8_continuations, {count}");
        assert_eq!(text.matches(&compare).count(), 1, "`{compare}`:\n{text}");
    }
    assert!(
        !text.contains("icmp eq i64 %utf8_continuations, 3"),
        "{text}"
    );
    assert_eq!(
        text.matches(", label %utf8_step, label %utf8_more").count(),
        3,
        "{text}"
    );
    let narrowed_lo = text.matches(", %utf8_first_lo").count();
    let narrowed_hi = text.matches(", %utf8_first_hi").count();
    assert_eq!((narrowed_lo, narrowed_hi), (1, 1), "{text}");
    assert_eq!(
        text.matches("icmp uge i32 %utf8_byte32").count(),
        3,
        "{text}"
    );
    assert_eq!(text.matches(", 128\n").count(), 2, "{text}");
    assert_eq!(text.matches(", 191\n").count(), 2, "{text}");
    let first = position(&text, ", %utf8_first_lo");
    let generic = position(&text, ", 128\n");
    assert!(
        first < generic,
        "the narrowed range belongs to the first continuation:\n{text}"
    );
}

// ── The reserved, unchecked conversion (ADR 4.10.26n AC 5) ─────────────────

// 4.10.26n AC 5: natively the unchecked conversion has no UTF-8 walk, and it
// still tests and clears `live`.
#[test]
fn the_unchecked_conversion_skips_only_the_utf8_walk() {
    let text = emit_in("tg_bytes_into_string_unchecked", &[HANDLE], true);
    assert!(!text.contains("utf8_"), "{text}");
    assert!(!text.contains(UTF8_LEAD_TABLE), "{text}");
    assert!(!text.contains("call void @tg_int_trap(i32 12)"), "{text}");
    let live = position(&text, "call void @tg_int_trap(i32 10)");
    let cleared = position(&text, "store i64 0, ptr");
    assert!(live < cleared, "{text}");
}

// 4.10.26n AC 5: a unit outside the core's run that uses it is refused.
#[test]
fn the_unchecked_conversion_is_refused_outside_the_core_run() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "user_unit");
    let handle = context.i64_type().const_zero().into();
    let refusal = codegen
        .try_compile_byte_intrinsic("__c_tg_bytes_into_string_unchecked", &[handle])
        .expect_err("refused");
    assert!(
        refusal.to_string().contains("reserved to the native core"),
        "{refusal}"
    );
}

#[test]
fn only_a_reserved_intrinsic_outside_the_core_run_is_disallowed() {
    use super::convert::{intrinsic_allowed, reserved_refusal};
    use albite_core::eval::byte_intrinsics::ByteIntrinsic;
    assert!(intrinsic_allowed(ByteIntrinsic::IntoString, false));
    assert!(intrinsic_allowed(ByteIntrinsic::IntoString, true));
    assert!(intrinsic_allowed(ByteIntrinsic::IntoStringUnchecked, true));
    assert!(!intrinsic_allowed(
        ByteIntrinsic::IntoStringUnchecked,
        false
    ));
    let message = reserved_refusal(ByteIntrinsic::IntoStringUnchecked);
    assert!(message.starts_with("tg_bytes_into_string_unchecked is reserved"));
}

// 9.10.26n AC 4: the unfilled new emits no `llvm.memset` in the core's run,
// the filled one still does, and a user program's call gets D2's refusal.
#[test]
fn the_unfilled_new_skips_only_the_fill_and_is_refused_outside_the_core_run() {
    use super::convert::reserved_refusal;
    use albite_core::eval::byte_intrinsics::ByteIntrinsic;
    let unfilled = emit_in("__c_tg_bytes_new_uninit", &[INDEX], true);
    assert!(!unfilled.contains("llvm.memset"), "{unfilled}");
    assert!(
        unfilled.contains("call void @tg_int_trap(i32 11)"),
        "{unfilled}"
    );
    assert!(emit_in("tg_bytes_new", &[INDEX], true).contains("llvm.memset"));
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "user_unit");
    let refusal = codegen
        .try_compile_byte_intrinsic(
            "tg_bytes_new_uninit",
            &[context.i64_type().const_zero().into()],
        )
        .expect_err("refused");
    assert_eq!(
        refusal.to_string(),
        format!("LLVM error: {}", reserved_refusal(ByteIntrinsic::NewUninit))
    );
    assert_eq!(
        reserved_refusal(ByteIntrinsic::NewUninit),
        "tg_bytes_new_uninit is reserved to the native core (ADR 9.10.26n D2): \
         an unfilled buffer is admitted only under the core's write-cover rule"
    );
}
