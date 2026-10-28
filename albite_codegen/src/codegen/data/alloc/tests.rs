//! The allocation helpers' tests, moved from `data/mod.rs` with the helpers
//! (ADR 7.10.26s §2.2), plus the region-bypassing call's two shapes (AC 5).

use super::{
    AllocClass, ALLOC_PARENT_SYMBOL, ALLOC_SYMBOL, ARENA_INIT_REGIONS_SYMBOL, ARENA_INIT_SYMBOL,
};
use albite_runtime::{CLASS_ENV, CLASS_MU, CLASS_MU_NULLARY, CLASS_OTHER, CLASS_REF, CLASS_STRING};

/// The discriminants cross the FFI boundary as a raw `u32`; a mismatch
/// silently misattributes allocation classes in the profiler.
#[test]
fn alloc_class_discriminants_match_runtime_constants() {
    assert_eq!(AllocClass::Mu as u32, CLASS_MU);
    assert_eq!(AllocClass::Env as u32, CLASS_ENV);
    assert_eq!(AllocClass::Ref as u32, CLASS_REF);
    assert_eq!(AllocClass::Str as u32, CLASS_STRING);
    assert_eq!(AllocClass::MuNullary as u32, CLASS_MU_NULLARY);
    // The runtime clamps out-of-range classes to CLASS_OTHER at record
    // time, so every codegen-emitted class must sit below the sentinel.
    assert!((AllocClass::MuNullary as u32) < CLASS_OTHER);
}

/// ADR 14.9.26b AC 3: the mode enum's twin. Codegen holds no mode of its
/// own — the prologue reads it — so the contract it pins is the runtime's:
/// the published `MODE_*` constants are the `ArenaMode` discriminants that
/// `__albite_arena_stats` writes into its `mode` field, and a process
/// whose prologue never ran reads as `Off`. ADR 7.10.26s D4 adds `Scoped`,
/// which generated sites never test for: any non-zero mode takes the symbol.
#[test]
fn arena_mode_discriminants_match_runtime_constants() {
    use albite_runtime::{ArenaMode, MODE_BUMP, MODE_OFF, MODE_SCOPED};
    assert_eq!(ArenaMode::Off as u32, MODE_OFF);
    assert_eq!(ArenaMode::Bump as u32, MODE_BUMP);
    assert_eq!(ArenaMode::Scoped as u32, MODE_SCOPED);
    assert_ne!(MODE_OFF, MODE_BUMP);
    assert_ne!(MODE_SCOPED, MODE_OFF);
    assert_ne!(MODE_SCOPED, MODE_BUMP);
    assert_eq!(albite_runtime::arena_mode(), ArenaMode::Off);
}

/// The symbols codegen emits by name are the ones the runtime exports, with
/// the signatures codegen declares — a rename or an arity change on either
/// side would otherwise fail only at link time.
#[test]
fn emitted_symbols_name_the_runtime_exports() {
    let alloc: unsafe extern "C" fn(u64, u32) -> *mut core::ffi::c_void =
        albite_runtime::__albite_alloc;
    let alloc_parent: unsafe extern "C" fn(u64, u32) -> *mut core::ffi::c_void =
        albite_runtime::__albite_alloc_parent;
    let init: extern "C" fn() = albite_runtime::__albite_arena_init;
    let init_regions: extern "C" fn() = albite_runtime::__albite_arena_init_regions;
    let _ = (alloc, alloc_parent, init, init_regions);
    assert_eq!(ALLOC_SYMBOL, "__albite_alloc");
    assert_eq!(ALLOC_PARENT_SYMBOL, "__albite_alloc_parent");
    assert_eq!(ARENA_INIT_SYMBOL, "__albite_arena_init");
    assert_eq!(ARENA_INIT_REGIONS_SYMBOL, "__albite_arena_init_regions");
}

/// ADR 18.9.26c AC 2: the two shapes `build_malloc_call` emits, read off the
/// printed IR of a one-allocation function — and ADR 7.10.26s AC 5's two
/// shapes of `build_parent_alloc_call`.
mod alloc_shape_tests {
    use crate::codegen::data::alloc::AllocClass;
    use crate::codegen::CodeGen;
    use inkwell::context::Context;

    /// Which helper a probe calls.
    #[derive(Clone, Copy)]
    enum Entry {
        Malloc,
        Parent,
    }

    /// Emit one `Env`-class allocation through `entry` in a fresh function
    /// and return the function's IR text (verified, so a malformed phi fails
    /// here).
    fn emit_one_allocation_through(entry: Entry, class: AllocClass, alloc_profile: bool) -> String {
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "alloc_shape");
        codegen.tracing.alloc_profile = alloc_profile;
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let function = codegen
            .module
            .add_function("probe", ptr.fn_type(&[], false), None);
        codegen
            .builder
            .position_at_end(context.append_basic_block(function, "entry"));
        let size = context.i64_type().const_int(24, false);
        let allocation = match entry {
            Entry::Malloc => codegen.build_malloc_call(size, class, "env"),
            Entry::Parent => codegen.build_parent_alloc_call(size, class, "env"),
        }
        .expect("allocation builds");
        codegen
            .builder
            .build_return(Some(&allocation))
            .expect("return");
        assert!(function.verify(true), "probe must verify");
        codegen.module.print_to_string().to_string()
    }

    fn emit_one_allocation(alloc_profile: bool) -> String {
        emit_one_allocation_through(Entry::Malloc, AllocClass::Env, alloc_profile)
    }

    #[test]
    fn non_profiled_allocation_branches_on_the_mode_flag() {
        let ir = emit_one_allocation(false);
        assert!(ir.contains("load i32, ptr @__albite_arena_mode"), "{ir}");
        assert!(ir.contains("icmp eq i32 %arena.mode, 0"), "{ir}");
        assert!(
            ir.contains("br i1 %arena.off, label %alloc.malloc, label %alloc.arena"),
            "{ir}"
        );
        assert!(
            ir.contains("label %alloc.arena, !prof !0")
                && ir.contains("!0 = !{!\"branch_weights\", i32 2000, i32 1}"),
            "the off arm is weighted as the likely one: {ir}"
        );
        assert!(ir.contains("call ptr @malloc(i64 24)"), "{ir}");
        assert!(
            ir.contains("call ptr @__albite_alloc(i64 24, i32 1)"),
            "{ir}"
        );
        assert!(
            ir.contains("phi ptr [ %env.malloc, %alloc.malloc ], [ %env.arena, %alloc.arena ]"),
            "{ir}"
        );
        assert!(ir.contains("ret ptr %env"), "{ir}");
    }

    /// The `@malloc` call sits in the `alloc.malloc` block — the invariant
    /// `check-arena-mode` scans for.
    #[test]
    fn non_profiled_malloc_call_is_inside_the_malloc_block() {
        let ir = emit_one_allocation(false);
        let block = ir
            .split("\nalloc.malloc:")
            .nth(1)
            .and_then(|rest| rest.split("\nalloc.").next())
            .expect("an alloc.malloc block");
        assert!(block.contains("call ptr @malloc("), "{ir}");
        assert!(!block.contains("@__albite_alloc"), "{ir}");
    }

    #[test]
    fn profiled_allocation_always_calls_the_symbol() {
        let ir = emit_one_allocation(true);
        assert!(
            ir.contains("%env = call ptr @__albite_alloc(i64 24, i32 1)"),
            "{ir}"
        );
        assert!(!ir.contains("call ptr @malloc"), "{ir}");
        assert!(!ir.contains("load i32, ptr @__albite_arena_mode"), "{ir}");
        assert!(!ir.contains("phi"), "{ir}");
    }

    /// ADR 7.10.26s AC 5: the region-bypassing call keeps the mode branch and
    /// its `alloc.malloc` off arm, and its arena arm names the parent symbol
    /// with the caller's class.
    #[test]
    fn non_profiled_parent_allocation_branches_to_the_parent_symbol() {
        let ir = emit_one_allocation_through(Entry::Parent, AllocClass::Str, false);
        assert!(ir.contains("icmp eq i32 %arena.mode, 0"), "{ir}");
        let malloc_block = ir
            .split("\nalloc.malloc:")
            .nth(1)
            .and_then(|rest| rest.split("\nalloc.").next())
            .expect("an alloc.malloc block");
        assert!(malloc_block.contains("call ptr @malloc(i64 24)"), "{ir}");
        assert!(
            ir.contains("%env.arena = call ptr @__albite_alloc_parent(i64 24, i32 3)"),
            "{ir}"
        );
        assert!(!ir.contains("call ptr @__albite_alloc("), "{ir}");
        assert!(
            ir.contains("phi ptr [ %env.malloc, %alloc.malloc ], [ %env.arena, %alloc.arena ]"),
            "{ir}"
        );
    }

    /// ADR 7.10.26s AC 5: profiled, the parent symbol is called alone.
    #[test]
    fn profiled_parent_allocation_always_calls_the_parent_symbol() {
        let ir = emit_one_allocation_through(Entry::Parent, AllocClass::Str, true);
        assert!(
            ir.contains("%env = call ptr @__albite_alloc_parent(i64 24, i32 3)"),
            "{ir}"
        );
        assert!(!ir.contains("call ptr @malloc"), "{ir}");
        assert!(!ir.contains("call ptr @__albite_alloc("), "{ir}");
        assert!(!ir.contains("phi"), "{ir}");
    }

    /// Two allocations in one function get distinct, `alloc.malloc`-prefixed
    /// blocks, and the fixture setups no longer double-declare `malloc`.
    #[test]
    fn repeated_allocations_uniquify_block_names_and_declare_malloc_once() {
        let context = Context::create();
        let codegen = CodeGen::new(&context, "alloc_twice");
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let function = codegen
            .module
            .add_function("probe", ptr.fn_type(&[], false), None);
        codegen
            .builder
            .position_at_end(context.append_basic_block(function, "entry"));
        let size = context.i64_type().const_int(8, false);
        let _ = codegen
            .build_malloc_call(size, AllocClass::Mu, "a")
            .unwrap();
        let second_allocation = codegen
            .build_malloc_call(size, AllocClass::Str, "b")
            .unwrap();
        codegen
            .builder
            .build_return(Some(&second_allocation))
            .unwrap();
        assert!(function.verify(true));
        let ir = codegen.module.print_to_string().to_string();
        // LLVM's suffix is a per-function counter shared by every name, so
        // only the prefix is stable — which is all `check-arena-mode` matches.
        let malloc_labels: Vec<&str> = ir
            .lines()
            .filter(|l| l.starts_with("alloc.malloc"))
            .collect();
        assert_eq!(malloc_labels.len(), 2, "{ir}");
        assert_ne!(malloc_labels[0], malloc_labels[1], "{ir}");
        assert!(
            ir.contains("call ptr @__albite_alloc(i64 8, i32 3)"),
            "{ir}"
        );
        assert!(!ir.contains("malloc.1"), "{ir}");
        assert_eq!(ir.matches("declare ptr @malloc(").count(), 1, "{ir}");
    }
}
