//! IR-shape tests for `substring`'s cut check (ADR 16.10.26h AC 4) and
//! `char_at`'s guarded read (ADR 16.10.26i AC 3, D5).
//!
//! The lowering is emitted into a fresh function whose operands are its
//! parameters — constants would fold the guards under test away — and the
//! verified module text is read back. The assertions match instructions and
//! operands, never block labels.

use crate::codegen::backend::native_target_machine;
use crate::codegen::CodeGen;
use inkwell::context::Context;
use inkwell::passes::PassBuilderOptions;
use inkwell::{AddressSpace, OptimizationLevel};

/// Emit `substring(text, start, len)` over the probe's three parameters,
/// return void, verify, and hand back the module text.
fn emit_substring() -> String {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "substring_shape");
    let i64_type = context.i64_type();
    let string_type = context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            i64_type.into(),
        ],
        false,
    );
    let fn_type = context.void_type().fn_type(
        &[string_type.into(), i64_type.into(), i64_type.into()],
        false,
    );
    let function = codegen.module.add_function("probe", fn_type, None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    codegen.compilation.current_fn = Some(function);
    let param = |n: u32| function.get_nth_param(n).expect("param");
    codegen
        .build_str_substring(
            param(0).into_struct_value(),
            param(1).into_int_value(),
            param(2).into_int_value(),
        )
        .expect("lowers");
    codegen.builder.build_return(None).expect("return");
    assert!(function.verify(true), "substring must verify");
    codegen.module.print_to_string().to_string()
}

/// The text of the `probe` function's body only, so declarations of the
/// runtime functions cannot satisfy an assertion.
fn probe_body(module: &str) -> &str {
    let name = module.find(" @probe(").expect("probe defined");
    let start = module[..name].rfind("define ").expect("probe defined");
    let body = &module[start..];
    &body[..body.find("\n}\n").expect("probe closes")]
}

fn position(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in:\n{text}"))
}

// 16.10.26h AC 4: exactly two byte loads (one per cut point), and each is
// preceded by its own unsigned in-range compare — no load outside a guard.
#[test]
fn each_cut_point_load_sits_behind_an_unsigned_guard() {
    let module = emit_substring();
    let body = probe_body(&module);
    let loads: Vec<usize> = body.match_indices("load i8").map(|(at, _)| at).collect();
    assert_eq!(loads.len(), 2, "one load per cut point:\n{body}");
    let guards: Vec<usize> = body.match_indices("icmp ult").map(|(at, _)| at).collect();
    // The two clamps are `icmp ult` too, so the guards are the last two
    // before each load: the n-th load must have at least n + 2 compares
    // ahead of it (two clamps, then one guard per load so far).
    for (n, load) in loads.iter().enumerate() {
        let ahead = guards.iter().filter(|&&guard| guard < *load).count();
        assert!(
            ahead >= n + 3,
            "load {n} is not behind its own guard:\n{body}"
        );
    }
    assert!(!body.contains("icmp slt"), "signed compare:\n{body}");
}

// 16.10.26h AC 4: the trap is called with code 13, and its branch comes
// before the copy's allocation, so a refused cut allocates nothing.
#[test]
fn the_substring_trap_is_code_13_and_precedes_the_allocation() {
    let module = emit_substring();
    let body = probe_body(&module);
    let trap = position(body, "@tg_int_trap(i32 13)");
    let allocation = position(body, "@malloc(");
    assert!(
        trap < allocation,
        "the trap must precede the allocation:\n{body}"
    );
    let copy = position(body, "llvm.memcpy");
    assert!(trap < copy, "the trap must precede the copy:\n{body}");
    let last_cut_test = body.rfind("and i8").expect("a cut test");
    assert!(
        last_cut_test < trap,
        "both cut tests precede the trap:\n{body}"
    );
    assert_eq!(
        body.matches("and i8").count(),
        2,
        "one test per cut point:\n{body}"
    );
    assert!(body.contains("-64"), "the continuation mask 0xC0:\n{body}");
    assert!(body.contains("-128"), "the continuation tag 0x80:\n{body}");
}

/// Emit `char_at(text, index)` over the probe's two parameters, return the
/// `Nat`, verify, optionally run LLVM's `default<O2>` over the module, and
/// hand back the module text.
fn emit_char_at(optimize: bool) -> String {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "char_at_shape");
    let i64_type = context.i64_type();
    let string_type = context.struct_type(
        &[
            context.ptr_type(AddressSpace::default()).into(),
            i64_type.into(),
        ],
        false,
    );
    let fn_type = i64_type.fn_type(&[string_type.into(), i64_type.into()], false);
    let function = codegen.module.add_function("probe", fn_type, None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    codegen.compilation.current_fn = Some(function);
    let param = |n: u32| function.get_nth_param(n).expect("param");
    let nat = codegen
        .build_str_char_at(param(0).into_struct_value(), param(1).into_int_value())
        .expect("lowers");
    codegen.builder.build_return(Some(&nat)).expect("return");
    assert!(function.verify(true), "char_at must verify");
    if optimize {
        let machine =
            native_target_machine(OptimizationLevel::Default).expect("a host target machine");
        codegen
            .module
            .run_passes("default<O2>", &machine, PassBuilderOptions::create())
            .expect("default<O2> runs");
    }
    codegen.module.print_to_string().to_string()
}

// 16.10.26i AC 3: one byte load, behind an unsigned compare of the index with
// the length and a conditional branch, joined with `0` by a `phi`.
#[test]
fn char_at_load_sits_behind_an_unsigned_guard_and_joins_zero() {
    let module = emit_char_at(false);
    let body = probe_body(&module);
    assert_eq!(body.matches("load i8").count(), 1, "one load:\n{body}");
    let guard = position(body, "icmp ult i64 %1, ");
    let branch = position(body, "br i1");
    let load = position(body, "load i8");
    let join = position(body, "phi i8");
    assert!(guard < branch, "the compare feeds the branch:\n{body}");
    assert!(branch < load, "the load is past the branch:\n{body}");
    assert!(load < join, "the phi joins after the load:\n{body}");
    let phi_line = &body[join..join + body[join..].find('\n').expect("phi ends")];
    assert!(
        phi_line.contains("[ 0, %entry ]"),
        "past the end is 0: {phi_line}"
    );
    assert!(!body.contains("icmp slt"), "signed compare:\n{body}");
}

// 16.10.26i D5, P0.3: the guard survives `default<O2>`. Whatever shape the
// optimiser leaves, the one load still sits past a conditional branch on an
// unsigned compare of the index — never hoisted above it, never a `select`
// over a load that already ran.
#[test]
fn char_at_guard_survives_default_o2() {
    let module = emit_char_at(true);
    let body = probe_body(&module);
    assert_eq!(body.matches("load i8").count(), 1, "one load:\n{body}");
    // The optimiser canonicalises `idx ult len` to `len ugt idx`: either
    // orientation is the same unsigned guard on the index (`%1`).
    let guard = position(body, "icmp u");
    let guard_line = &body[guard..guard + body[guard..].find('\n').expect("icmp ends")];
    assert!(
        guard_line.contains("%1"),
        "the guard reads the index: {guard_line}"
    );
    let branch = position(body, "br i1");
    let load = position(body, "load i8");
    assert!(guard < branch, "the compare feeds the branch:\n{body}");
    assert!(branch < load, "the load is past the branch:\n{body}");
    assert!(!body.contains("icmp slt"), "signed compare:\n{body}");
}
