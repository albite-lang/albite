//! The argument init in the core's real emitted IR (ADR 7.10.26r AC 3): the
//! embedded `args.tg` through the whole code-generation run and the core's
//! `default<O2>` pipeline, read back as `.ll` text.

use super::tests::{core_ir, definition_body};

// 7.10.26r AC 3 (D1, D2, D3): after the pipeline, `tg_native_init_args_c` is
// the publish protocol itself — an acquire load of the vector, a release
// store of the count and an `acq_rel` `cmpxchg` from null, each `align 8` —
// over the two statics `albite_core` defines, which keep their C names as
// external declarations. Nothing is copied: no allocation and no call.
#[test]
fn the_core_init_publishes_into_the_runtime_store_with_no_copy() {
    let ir = core_ir(None);
    // The pipeline may add `local_unnamed_addr`; the name and the missing
    // initializer are what matter.
    for declaration in [
        "@tg_core_args_count = external ",
        "@tg_core_args_vector = external ",
    ] {
        assert!(ir.contains(declaration), "{declaration}\n{ir}");
    }
    assert!(!ir.contains("__albite_native_core__tg_core_args"), "{ir}");
    let body = definition_body(&ir, "tg_native_init_args_c").join("\n");
    for access in [
        "load atomic ptr, ptr @tg_core_args_vector acquire, align 8",
        "store atomic i64 %",
        "ptr @tg_core_args_count release, align 8",
        "cmpxchg ptr @tg_core_args_vector, ptr null, ptr %1 acq_rel acquire, align 8",
    ] {
        assert!(body.contains(access), "{access}\n{body}");
    }
    assert!(body.contains("zext i32 %0 to i64"), "{body}");
    assert!(!body.contains("call "), "the init copies nothing:\n{body}");
}
