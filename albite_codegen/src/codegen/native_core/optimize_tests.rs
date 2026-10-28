//! The shim fold's attribute (ADR 13.10.26p D2, AC 3).

use super::NATIVE_CORE_INLINED_INTO_SHIM;
use crate::codegen::CodeGen;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::context::Context;

/// Define `name() -> void` with an empty body.
fn define_void<'ctx>(codegen: &CodeGen<'ctx>, context: &'ctx Context, name: &str) {
    let function = codegen
        .module
        .add_function(name, context.void_type().fn_type(&[], false), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    builder.build_return(None).expect("ret");
}

fn is_always_inline(codegen: &CodeGen<'_>, name: &str) -> bool {
    let kind = Attribute::get_named_enum_kind_id("alwaysinline");
    codegen
        .module
        .get_function(name)
        .expect("defined")
        .get_enum_attribute(AttributeLoc::Function, kind)
        .is_some()
}

// 13.10.26p AC3: the pipeline marks only the owned join's prefixed
// `$direct_mt` `alwaysinline`, so `default<O2>` folds it into its shim; the
// concat's is left to the inliner's own cost model.
#[test]
fn the_pipeline_marks_only_the_owned_join_always_inline() {
    assert_eq!(NATIVE_CORE_INLINED_INTO_SHIM, &["string_concat_owned"]);
    let context = Context::create();
    let codegen = CodeGen::new(&context, "inline_marks");
    let owned = "__albite_native_core__string_concat_owned$direct_mt";
    let concat = "__albite_native_core__string_concat$direct_mt";
    define_void(&codegen, &context, owned);
    define_void(&codegen, &context, concat);
    codegen.optimize_native_core().expect("pipeline");
    assert!(is_always_inline(&codegen, owned));
    assert!(!is_always_inline(&codegen, concat));
}
