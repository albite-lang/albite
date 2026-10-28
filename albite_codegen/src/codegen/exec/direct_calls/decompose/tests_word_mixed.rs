//! IR test for a word entry beside an indirect buffer (ADR 2.10.26h AC 3),
//! split from `tests_word.rs` to keep that file under the size threshold
//! (ADR 2.10.26k). Shares its helpers.

use super::tests_word::{blob, direct_type, signature_line};
use super::ParamLowering;
use crate::codegen::CodeGen;
use inkwell::context::Context;

/// ADR 2.10.26h AC3: beside a 20-byte blob, the 4-byte enum becomes a word
/// and the large blob stays an indirect buffer leading before `env`.
#[test]
fn a_large_blob_stays_indirect_beside_a_word() {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx, "mixed");
    let fn_type = direct_type(
        &ctx,
        ctx.i64_type().into(),
        &[blob(&ctx, 0).into(), blob(&ctx, 16).into()],
    );
    cg.module.add_function("mixed$direct", fn_type, None);
    let lowerings = cg.declare_decomposed_entry("mixed", fn_type).unwrap();
    assert_eq!(
        lowerings,
        vec![ParamLowering::Word(32), ParamLowering::Indirect]
    );
    let line = signature_line(&cg, "mixed$direct_mt");
    // [0]=indirect buffer, [1]=env, [2]=word.
    let params = line
        .split_once('(')
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    assert!(
        params.starts_with("ptr noalias nonnull align 4 dereferenceable(20)"),
        "got: {line}"
    );
    assert!(params.trim_end().ends_with("ptr, i32)"), "got: {line}");
}
