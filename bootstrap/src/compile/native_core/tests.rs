//! The core's real emitted IR (ADR 4.10.26n AC 2, AC 3): the embedded sources
//! through the whole code-generation run, read back as `.ll` text.

use super::{
    alloc_profile_filter, compile_native_core, NativeCoreArtifact, NativeCoreBuild, NATIVE_CORE_LL,
    NATIVE_CORE_UNIT,
};
use crate::compile::per_module::OutputKind;
use albite_codegen::{is_native_core_export, NativeCoreSwitch, NATIVE_CORE_PREFIX};

/// The symbol a `define` line names, unquoted.
fn defined_symbol(line: &str) -> Option<String> {
    let at = line.find('@')?;
    let rest = &line[at + 1..];
    let name = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => rest.split('(').next()?,
    };
    Some(name.to_string())
}

/// Every externally linked definition: `define` without `private`/`internal`.
fn external_definitions(ir: &str) -> Vec<String> {
    ir.lines()
        .filter(|line| line.starts_with("define "))
        .filter(|line| !line.contains(" private ") && !line.contains(" internal "))
        .filter_map(defined_symbol)
        .collect()
}

#[test]
fn defined_symbols_are_read_quoted_or_bare() {
    assert_eq!(
        defined_symbol("define i64 @\"a$direct\"(ptr %0) {").as_deref(),
        Some("a$direct")
    );
    assert_eq!(
        defined_symbol("define i64 @b(ptr %0) {").as_deref(),
        Some("b")
    );
    assert_eq!(
        external_definitions("define private i64 @p() {\ndefine i64 @q() {\n"),
        vec!["q"]
    );
}

// 4.10.26n AC 2, AC 3: the core's run defines `tg_native_string_concat` as a
// one-call shim, and every other external definition carries the prefix —
// `<name>`, `$direct`, `$direct_mt`, lambdas and the `__wrap_` helpers alike.
#[test]
fn every_external_definition_of_the_core_is_prefixed_or_an_export_row() {
    let artifact = compile_native_core(&core_build(false, None)).expect("core");
    let module = artifact.module();
    assert_eq!(module.kind, OutputKind::Ll);
    assert_eq!(module.name, NATIVE_CORE_UNIT);
    let ir = std::fs::read_to_string(&module.output_path).expect("core .ll");
    let defined = external_definitions(&ir);
    assert!(defined.len() > 10, "{defined:?}");
    for name in &defined {
        assert!(
            name.starts_with(NATIVE_CORE_PREFIX) || is_native_core_export(name),
            "{name} escaped the prefix"
        );
    }
    for kind in [
        "string_concat",
        "string_concat$direct",
        "string_concat$direct_mt",
    ] {
        let prefixed = format!("{NATIVE_CORE_PREFIX}{kind}");
        assert!(
            defined.contains(&prefixed),
            "{prefixed} missing: {defined:?}"
        );
    }
    assert!(
        defined.iter().any(|name| name.contains("_lambda_")),
        "{defined:?}"
    );
    assert!(defined.contains(&"tg_native_string_concat".to_string()));
    // After the pipeline (7.10.26p D2) the shim's `$direct` call is inlined,
    // leaving the call `$direct` made — to `$direct_mt` — so either kind is
    // the concat's body.
    let shim_calls: Vec<String> = definition_body(&ir, "tg_native_string_concat")
        .into_iter()
        .filter_map(callee)
        .collect();
    let concat_body = format!("{NATIVE_CORE_PREFIX}string_concat$direct");
    assert!(
        shim_calls.iter().any(|name| name.starts_with(&concat_body)),
        "{shim_calls:?}"
    );
    // The core uses no `++`, so it never calls a concat export itself.
    assert!(!ir.contains("call { ptr, i64 } @tg_string_concat("), "{ir}");
    assert!(
        !ir.contains("call { ptr, i64 } @tg_native_string_concat("),
        "{ir}"
    );
}

#[test]
fn the_core_writes_an_object_when_asked_and_the_tree_path_is_fixed() {
    let artifact = compile_native_core(&core_build(true, None)).expect("core");
    let module = artifact.module();
    assert_eq!(module.kind, OutputKind::Obj);
    assert!(
        std::fs::metadata(&module.output_path)
            .expect("object")
            .len()
            > 0
    );
    assert_eq!(
        NativeCoreArtifact::emit_llvm_dest(std::path::Path::new("out")),
        std::path::Path::new("out")
            .join(NATIVE_CORE_UNIT)
            .join(NATIVE_CORE_LL)
    );
}

fn core_build(emit_obj: bool, alloc_profile: Option<&str>) -> NativeCoreBuild<'_> {
    NativeCoreBuild {
        emit_obj,
        verbose: false,
        alloc_profile,
        switch: NativeCoreSwitch::On,
    }
}

/// The core's `.ll`, after its pass pipeline.
pub(super) fn core_ir(alloc_profile: Option<&str>) -> String {
    let artifact = compile_native_core(&core_build(false, alloc_profile)).expect("core");
    std::fs::read_to_string(artifact.module().output_path).expect("core .ll")
}

/// The body of the definition of `symbol`: its `define` line to the closing `}`.
pub(super) fn definition_body<'ir>(ir: &'ir str, symbol: &str) -> Vec<&'ir str> {
    let mut lines = ir.lines();
    let found = lines
        .by_ref()
        .any(|line| line.starts_with("define ") && defined_symbol(line).as_deref() == Some(symbol));
    assert!(found, "{symbol} is not defined");
    lines.take_while(|line| *line != "}").collect()
}

/// The callee of a `call` instruction, when it is a named function.
fn callee(line: &str) -> Option<String> {
    let call = line.find("call ")?;
    defined_symbol(&line[call..])
}

#[test]
fn a_call_line_names_its_callee() {
    assert_eq!(
        callee("  %r = musttail call { ptr, i64 } @\"x$direct_mt\"(ptr %e)").as_deref(),
        Some("x$direct_mt")
    );
    assert_eq!(
        callee("  call void @llvm.memcpy.p0.p0.i64(ptr %d)").as_deref(),
        Some("llvm.memcpy.p0.p0.i64")
    );
    assert_eq!(callee("  %r = add i64 %a, %b"), None);
    let ir = "define i64 @f() {\nentry:\n  ret i64 0\n}\ndefine i64 @g() {\n  ret i64 1\n}\n";
    assert_eq!(definition_body(ir, "g"), vec!["  ret i64 1"]);
}

// 7.10.26p AC 2: after the pipeline, neither concat calls a byte helper —
// no `bytes_*` wrapper and no `__wrap_tg_bytes_*` stub, of any symbol kind —
// from either of its bodies. `$direct` forwards to `$direct_mt`, which holds
// the join, so that one must still copy: its `memcpy` is the non-vacuity.
#[test]
fn the_optimised_concats_call_no_byte_helper() {
    let ir = core_ir(None);
    for concat in ["string_concat", "string_concat_owned"] {
        for kind in ["$direct", "$direct_mt"] {
            let symbol = format!("{NATIVE_CORE_PREFIX}{concat}{kind}");
            let calls: Vec<String> = definition_body(&ir, &symbol)
                .into_iter()
                .filter_map(callee)
                .collect();
            let helpers: Vec<&String> = calls
                .iter()
                .filter(|name| name.contains("bytes_"))
                .collect();
            assert!(helpers.is_empty(), "{symbol} still calls {helpers:?}");
            if kind == "$direct_mt" {
                assert!(
                    calls.iter().any(|name| name.starts_with("llvm.memcpy")),
                    "{symbol} copies nothing: {calls:?}"
                );
            }
        }
    }
}

// 7.10.26p AC 2: the pipeline keeps every export shim's name and the
// signature of the Rust export it shadows.
#[test]
fn the_optimised_shims_keep_their_names_and_signatures() {
    let ir = core_ir(None);
    for row in albite_codegen::NATIVE_CORE_EXPORTS {
        let define = ir
            .lines()
            .find(|line| {
                line.starts_with("define ") && defined_symbol(line).as_deref() == Some(row.symbol)
            })
            .unwrap_or_else(|| panic!("{} is not defined", row.symbol));
        // The trap takes `tg_int_trap`'s `void (i32)` (7.10.26q D3), the
        // init `tg_init_args_c`'s `void (i32, ptr)` (7.10.26r D3).
        let signature = if row.shadows == "tg_int_trap" {
            format!("void @{}(i32", row.symbol)
        } else if row.shadows == "tg_init_args_c" {
            format!("void @{}(i32 %0, ptr %1)", row.symbol)
        } else {
            format!("{{ ptr, i64 }} @{}({{ ptr, i64 }}", row.symbol)
        };
        assert!(define.contains(&signature), "{define}");
        assert!(
            !define.contains(" internal ") && !define.contains(" private "),
            "{define}"
        );
    }
}

// 7.10.26p AC 2: the core's optimised IR is fixed — two runs, one text.
#[test]
fn the_optimised_core_is_deterministic() {
    assert_eq!(core_ir(None), core_ir(None));
}

// 7.10.26p AC 6 (D5): a core compiled with `--alloc-profile` records its
// allocations — each function announces itself to the profiler and every
// allocation goes through `__albite_alloc`, never `malloc` directly.
#[test]
fn a_profiled_core_routes_its_allocations_through_the_profiler() {
    let profiled = core_ir(Some(""));
    assert!(
        profiled.contains("call void @__albite_alloc_profile_set_fn("),
        "{profiled}"
    );
    assert!(profiled.contains("@__albite_alloc("), "{profiled}");
    assert!(!profiled.contains("call ptr @malloc("), "{profiled}");
    let unprofiled = core_ir(None);
    assert!(!unprofiled.contains("@__albite_alloc_profile_set_fn("));
}

#[test]
fn the_alloc_profile_flag_is_a_filter_unless_empty() {
    assert_eq!(alloc_profile_filter(""), None);
    assert_eq!(alloc_profile_filter("main").as_deref(), Some("main"));
}

/// The destination operand of an `llvm.memcpy` call line: the last token of
/// its first argument.
pub(super) fn memcpy_destination(line: &str) -> Option<&str> {
    let args = line.split_once("@llvm.memcpy")?.1.split_once('(')?.1;
    args.split(',').next()?.split_whitespace().last()
}

#[test]
fn a_memcpy_line_names_its_destination() {
    let line = "  tail call void @llvm.memcpy.p0.p0.i64(ptr nonnull align 1 %blk, ptr align 1 %2, i64 %3, i1 false)";
    assert_eq!(memcpy_destination(line), Some("%blk"));
    assert_eq!(memcpy_destination("  %r = add i64 %a, %b"), None);
}

/// The instruction that defines the SSA value `ssa_name` inside `body`, if any.
pub(super) fn defining_line<'ir>(body: &[&'ir str], ssa_name: &str) -> Option<&'ir str> {
    let prefix = format!("  {ssa_name} = ");
    body.iter().copied().find(|line| line.starts_with(&prefix))
}

// 7.10.26p AC 3: the optimised join still writes `right` after `left`. Its
// two copies are distinct, and the second lands at an offset from the block —
// a `getelementptr` — so the pipeline kept the blit's position. 6.10.26m's
// non-vacuity drill, the second blit at offset 0, leaves the second copy
// writing to the reloaded block start instead (a `load`), which turns this red.
#[test]
fn the_optimised_join_copies_right_at_an_offset() {
    let ir = core_ir(None);
    let symbol = format!("{NATIVE_CORE_PREFIX}string_concat$direct_mt");
    let body = definition_body(&ir, &symbol);
    let destinations: Vec<&str> = body
        .iter()
        .copied()
        .filter_map(memcpy_destination)
        .collect();
    assert_eq!(destinations.len(), 2, "{destinations:?}");
    assert_ne!(destinations[0], destinations[1], "{destinations:?}");
    let second = defining_line(&body, destinations[1]).expect("the second copy's destination");
    assert!(second.contains("getelementptr"), "{second}");
}

#[test]
fn a_defining_line_is_found_by_its_value() {
    let body = [
        "  %a = load ptr, ptr %h",
        "  %b = getelementptr i8, ptr %a, i64 %n",
    ];
    assert_eq!(defining_line(&body, "%b"), Some(body[1]));
    assert_eq!(defining_line(&body, "%c"), None);
}

/// The core's `.ll` under `switch`, after its pass pipeline.
fn core_ir_under(switch: NativeCoreSwitch) -> String {
    let build = NativeCoreBuild {
        switch,
        ..core_build(false, None)
    };
    let artifact = compile_native_core(&build).expect("core");
    std::fs::read_to_string(artifact.module().output_path).expect("core .ll")
}

/// Every function `root` reaches through direct calls inside `ir`'s own
/// definitions, `root` included.
fn reached_definitions(ir: &str, root: &str) -> Vec<String> {
    let defined = external_definitions(ir);
    let mut reached = vec![root.to_string()];
    let mut next = 0;
    while next < reached.len() {
        let callees: Vec<String> = definition_body(ir, &reached[next])
            .into_iter()
            .filter_map(callee)
            .filter(|name| defined.contains(name) && !reached.contains(name))
            .collect();
        for name in callees {
            if !reached.contains(&name) {
                reached.push(name);
            }
        }
        next += 1;
    }
    reached
}

// 7.10.26q AC 4 (D4): the trap path has no trap site. `int_trap`, and every
// core definition it reaches, calls neither trap symbol, under `all` (where
// the core's own byte checks call the native trap) as under `off`. The walk
// is not inert: it reaches the shim's target and writes through `write`.
#[test]
fn the_trap_path_calls_no_trap() {
    for switch in [NativeCoreSwitch::Off, NativeCoreSwitch::All] {
        let ir = core_ir_under(switch);
        let root = format!("{NATIVE_CORE_PREFIX}int_trap");
        let reached = reached_definitions(&ir, &root);
        let mut calls = Vec::new();
        for name in &reached {
            calls.extend(definition_body(&ir, name).into_iter().filter_map(callee));
        }
        for trap in ["tg_int_trap", "tg_native_int_trap"] {
            assert!(
                !calls.iter().any(|name| name == trap),
                "{switch:?}: the trap path calls {trap}: {calls:?}"
            );
        }
        assert!(calls.iter().any(|name| name == "write"), "{calls:?}");
        assert!(calls.iter().any(|name| name == "abort"), "{calls:?}");
    }
}

// 7.10.26q AC 4 (D3): under `all` the core's byte checks call the native
// trap, which the core defines exactly once — never as `.1` beside an
// undefined export — as a shim that widens and calls `int_trap`.
#[test]
fn under_all_the_core_defines_one_native_trap_and_its_checks_call_it() {
    let ir = core_ir_under(NativeCoreSwitch::All);
    let defines = ir
        .lines()
        .filter(|line| line.starts_with("define ") && line.contains("@tg_native_int_trap"))
        .count();
    assert_eq!(defines, 1, "{ir}");
    assert!(!ir.contains("tg_native_int_trap."), "{ir}");
    assert!(ir.contains("call void @tg_native_int_trap(i32 "), "{ir}");
    assert!(!ir.contains("call void @tg_int_trap("), "{ir}");
    let shim = definition_body(&ir, "tg_native_int_trap");
    assert!(
        shim.iter().any(|line| line.contains("zext i32")),
        "{shim:?}"
    );
    assert!(
        shim.iter().any(|line| line.trim() == "unreachable"),
        "{shim:?}"
    );
    let off = core_ir_under(NativeCoreSwitch::Off);
    assert!(off.contains("call void @tg_int_trap(i32 "), "{off}");
}
