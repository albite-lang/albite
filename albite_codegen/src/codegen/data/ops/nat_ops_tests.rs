//! Tests for `nat_ops.rs`. The division and remainder tests (ADR 9.10.26g
//! AC 2) read the guard back from the function's IR text, so a dropped
//! `icmp`, a swapped `select` arm or a guard on a constant divisor is a failed
//! assertion. A `#[path]` sibling: the guarded emitter pushed the file near
//! its cap.

use super::*;
use inkwell::context::Context;
use inkwell::values::{AnyValue, FunctionValue};

/// A function `i64 test_fn(i64, i64)`. Division tests use the parameters as
/// run-time operands: inkwell's builder constant-folds an instruction whose
/// operands are all constant, which would leave no guard text to assert on.
fn setup_codegen_with_function(context: &Context) -> CodeGen {
    let mut codegen = CodeGen::new(context, "test");
    let i64_type = context.i64_type();
    let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
    let function = codegen.module.add_function("test_fn", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    codegen.compilation.current_fn = Some(function);
    codegen
}

fn function<'ctx>(codegen: &CodeGen<'ctx>) -> FunctionValue<'ctx> {
    codegen.compilation.current_fn.expect("fixture function")
}

/// The fixture's parameters, as run-time `i64` operands.
fn operands<'ctx>(codegen: &CodeGen<'ctx>) -> (IntValue<'ctx>, IntValue<'ctx>) {
    let function = function(codegen);
    let a = function.get_nth_param(0).expect("a").into_int_value();
    let b = function.get_nth_param(1).expect("b").into_int_value();
    (a, b)
}

/// Return `result`, verify the module, and give back the function's IR.
fn finish(codegen: &CodeGen, result: BasicValueEnum) -> String {
    codegen.builder.build_return(Some(&result)).expect("ret");
    if let Err(message) = codegen.module.verify() {
        panic!("module failed verification: {message}");
    }
    function(codegen).print_to_string().to_string()
}

fn count(ir: &str, needle: &str) -> usize {
    ir.matches(needle).count()
}

// ── Division and remainder by zero (ADR 9.10.26g) ─────────────────────────

/// 9.10.26g AC 2: a run-time divisor is tested against zero, replaced by `1`
/// when it is, and a zero divisor's quotient selects `0`.
#[test]
fn nat_div_by_runtime_divisor_guards_and_selects_zero() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let (a, b) = operands(&codegen);
    let result = codegen.compile_nat_div(a, b).unwrap();
    let ir = finish(&codegen, result);

    assert!(
        ir.contains("%nat_divisor_is_zero = icmp eq i64 %1, 0"),
        "{ir}"
    );
    assert!(
        ir.contains("%nat_divisor = select i1 %nat_divisor_is_zero, i64 1, i64 %1"),
        "{ir}"
    );
    assert!(ir.contains("%nat_div = udiv i64 %0, %nat_divisor"), "{ir}");
    assert!(
        ir.contains("%nat_div_guarded = select i1 %nat_divisor_is_zero, i64 0, i64 %nat_div"),
        "{ir}"
    );
    assert!(ir.contains("ret i64 %nat_div_guarded"), "{ir}");
    assert_eq!(count(&ir, "select"), 2, "{ir}");
}

/// 9.10.26g AC 2: `%` guards the divisor the same way and needs no outer
/// `select`, because `a % 1` is already `0`.
#[test]
fn nat_mod_by_runtime_divisor_guards_without_an_outer_select() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let (a, b) = operands(&codegen);
    let result = codegen.compile_nat_mod(a, b).unwrap();
    let ir = finish(&codegen, result);

    assert!(
        ir.contains("%nat_divisor_is_zero = icmp eq i64 %1, 0"),
        "{ir}"
    );
    assert!(
        ir.contains("%nat_divisor = select i1 %nat_divisor_is_zero, i64 1, i64 %1"),
        "{ir}"
    );
    assert!(ir.contains("%nat_mod = urem i64 %0, %nat_divisor"), "{ir}");
    assert!(ir.contains("ret i64 %nat_mod"), "{ir}");
    assert_eq!(count(&ir, "select"), 1, "{ir}");
}

/// 9.10.26g AC 2: a constant non-zero divisor is the bare operation — the
/// native core's `write_decimal` divides by `10` and pays nothing.
#[test]
fn nat_div_and_mod_by_constant_ten_stay_bare() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let (a, _) = operands(&codegen);
    let ten = context.i64_type().const_int(10, false);
    let quotient = codegen.compile_nat_div(a, ten).unwrap();
    let remainder = codegen.compile_nat_mod(a, ten).unwrap();
    let sum = codegen
        .compile_nat_add(quotient.into_int_value(), remainder.into_int_value())
        .unwrap();
    let ir = finish(&codegen, sum);

    assert!(ir.contains("%nat_div = udiv i64 %0, 10"), "{ir}");
    assert!(ir.contains("%nat_mod = urem i64 %0, 10"), "{ir}");
    assert_eq!(count(&ir, "select"), 0, "{ir}");
    assert_eq!(count(&ir, "icmp"), 0, "{ir}");
}

/// 9.10.26g AC 2: a constant zero divisor is the answer `0` at build time,
/// so no `udiv`/`urem` by zero (which the builder would fold to poison) is
/// ever emitted.
#[test]
fn nat_div_and_mod_by_constant_zero_are_constant_zero() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let (a, _) = operands(&codegen);
    let zero = context.i64_type().const_zero();
    for result in [
        codegen.compile_nat_div(a, zero).unwrap(),
        codegen.compile_nat_mod(a, zero).unwrap(),
    ] {
        assert_eq!(
            result.into_int_value().get_zero_extended_constant(),
            Some(0)
        );
    }
    let ir = finish(&codegen, zero.into());
    assert_eq!(count(&ir, "udiv"), 0, "{ir}");
    assert_eq!(count(&ir, "urem"), 0, "{ir}");
}

/// 9.10.26g AC 2: with both operands constant the builder folds the bare
/// operation, so `10 / 3` and `10 % 3` are still computed at build time.
#[test]
fn nat_div_and_mod_of_constants_fold() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let ten = context.i64_type().const_int(10, false);
    let three = context.i64_type().const_int(3, false);
    let quotient = codegen.compile_nat_div(ten, three).unwrap();
    let remainder = codegen.compile_nat_mod(ten, three).unwrap();
    assert_eq!(
        quotient.into_int_value().get_zero_extended_constant(),
        Some(3)
    );
    assert_eq!(
        remainder.into_int_value().get_zero_extended_constant(),
        Some(1)
    );
}

// ── The rest of the Nat operations ────────────────────────────────────────

#[test]
fn test_compile_nat_add() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let a = context.i64_type().const_int(5, false);
    let b = context.i64_type().const_int(3, false);
    let result = codegen.compile_nat_add(a, b).unwrap();
    assert!(result.is_int_value());
}

#[test]
fn test_compile_nat_sub_no_underflow() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let a = context.i64_type().const_int(5, false);
    let b = context.i64_type().const_int(3, false);
    let result = codegen.compile_nat_sub(a, b).unwrap();
    assert!(result.is_int_value());
}

#[test]
fn test_compile_nat_mul() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let a = context.i64_type().const_int(4, false);
    let b = context.i64_type().const_int(3, false);
    let result = codegen.compile_nat_mul(a, b).unwrap();
    assert!(result.is_int_value());
}

#[test]
fn test_compile_nat_eq() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let a = context.i64_type().const_int(5, false);
    let b = context.i64_type().const_int(5, false);
    let result = codegen.compile_nat_eq(a, b).unwrap();
    // Comparison returns i1 (bool)
    assert!(result.is_int_value());
    assert_eq!(result.into_int_value().get_type().get_bit_width(), 1);
}

#[test]
fn test_compile_nat_comparisons() {
    let context = Context::create();
    let codegen = setup_codegen_with_function(&context);
    let a = context.i64_type().const_int(3, false);
    let b = context.i64_type().const_int(5, false);
    let width = |v: BasicValueEnum| v.into_int_value().get_type().get_bit_width();
    // All comparisons should return i1
    assert_eq!(width(codegen.compile_nat_lt(a, b).unwrap()), 1);
    assert_eq!(width(codegen.compile_nat_le(a, b).unwrap()), 1);
    assert_eq!(width(codegen.compile_nat_gt(a, b).unwrap()), 1);
    assert_eq!(width(codegen.compile_nat_ge(a, b).unwrap()), 1);
}
