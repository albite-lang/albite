//! The core's object level (ADR 13.10.26p D2, AC 3): written at
//! `NATIVE_CORE_OBJECT_LEVEL`, which is `Default`, whatever the program's.

use super::tests::{core_ir, definition_body};
use super::{
    compile_native_core, compile_native_core_at, NativeCoreBuild, NATIVE_CORE_OBJECT_LEVEL,
};
use albite_codegen::inkwell::OptimizationLevel;
use albite_codegen::NativeCoreSwitch;

fn object_build() -> NativeCoreBuild<'static> {
    NativeCoreBuild {
        emit_obj: true,
        verbose: false,
        alloc_profile: None,
        switch: NativeCoreSwitch::All,
    }
}

fn object_bytes(level: Option<OptimizationLevel>) -> Vec<u8> {
    let build = object_build();
    let artifact = match level {
        Some(level) => compile_native_core_at(&build, level),
        None => compile_native_core(&build),
    }
    .expect("core");
    std::fs::read(artifact.module().output_path).expect("core object")
}

// 13.10.26p AC3: the level is `Default`, and the object `compile_native_core`
// writes is the core written at it — not at `None`, the program's level, which
// is what reverting D2 would write.
#[test]
fn the_core_object_is_written_at_the_default_level() {
    assert_eq!(NATIVE_CORE_OBJECT_LEVEL, OptimizationLevel::Default);
    let shipped = object_bytes(None);
    assert_eq!(shipped, object_bytes(Some(OptimizationLevel::Default)));
    assert_ne!(shipped, object_bytes(Some(OptimizationLevel::None)));
}

// 13.10.26p AC3 (D2's shim): `alwaysinline` folds the owned join into its
// shim, so the shim no longer calls `string_concat_owned$direct_mt`.
#[test]
fn the_owned_shim_has_its_join_inlined() {
    let ir = core_ir(None);
    let shim = definition_body(&ir, "tg_native_string_concat_owned");
    assert!(!shim.is_empty(), "the owned shim is defined");
    assert!(
        !shim
            .iter()
            .any(|line| line.contains("call") && line.contains("string_concat_owned$direct_mt")),
        "the shim still calls the owned join: {shim:#?}"
    );
}
