//! Tests for `region { e }`'s lowering (ADR 9.10.26o AC 4, 13.10.26q AC 3):
//! each exit calls detach, the copy and release, innermost first, only in the
//! function that opened the region; a value with no pointer copies nothing,
//! so its IR is `B1.2`'s; a tail position inside a region is no tail call.

use super::MISSING_EXIT_TYPE;
use crate::codegen::musttail_report::ReasonCode;
use crate::codegen::state::OpenRegion;
use crate::codegen::CodeGen;
use albite_core::terms::{RegionTypes, Term};
use albite_core::types::Type;
use inkwell::context::Context;

const ENTER: &str = "call void @__albite_region_enter()";
const DETACH: &str = "call void @__albite_region_detach()";
const COPY: &str = "call void @__albite_region_copy_out(";
const LOAD: &str = "load {";
const RELEASE: &str = "call void @__albite_region_release()";

/// `region { term }` whose value is a `Nat` and that nothing crosses.
fn nat_region(term: Term) -> Term {
    Term::region(RegionTypes::value_only(Type::Nat), term)
}

/// A region whose value is `value` and that a `return` of `exit` crosses.
fn crossed_region(value: Type, exit: Type, term: Term) -> Term {
    Term::region(
        RegionTypes {
            value,
            exit: Some(exit),
        },
        term,
    )
}

/// `"a" ++ "b"`: a `String` the body builds.
fn built_string() -> Term {
    Term::str_concat(Term::StringLit("a".into()), Term::StringLit("b".into()))
}

/// Compile `term` as the body of `i64 f()`, returning the IR of `f` alone
/// (and of any function it spawned), or the error.
fn compile_in_nat_function(term: &Term) -> Result<String, String> {
    compile_in_function(term, false)
}

/// Compile `term` as the body of `f`, returning an `i64` or (`string`) a
/// `{ ptr, i64 }`.
fn compile_in_function(term: &Term, string: bool) -> Result<String, String> {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "region_test");
    let fn_type = if string {
        codegen.types.string_type().fn_type(&[], false)
    } else {
        context.i64_type().fn_type(&[], false)
    };
    let function = codegen.module.add_function("f", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    codegen.compilation.current_fn = Some(function);
    let value = codegen.compile_term(term).map_err(|e| e.to_string())?;
    codegen
        .builder
        .build_return(Some(&value))
        .map_err(|e| e.to_string())?;
    Ok(codegen.module.print_to_string().to_string())
}

/// The body of the function named `name` in `ir`.
fn function_body<'a>(ir: &'a str, name: &str) -> &'a str {
    let start = ir
        .find(&format!("@{name}("))
        .unwrap_or_else(|| panic!("no function {name} in:\n{ir}"));
    let rest = &ir[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// The positions of `needle` in `haystack`, in order.
fn positions(haystack: &str, needle: &str) -> Vec<usize> {
    haystack.match_indices(needle).map(|(i, _)| i).collect()
}

// 9.10.26o AC4, 13.10.26q AC3: a Nat region's fall-through emits enter, the
// body, then detach and release, with no copy, slot or layout — B1.2's IR.
#[test]
fn a_scalar_region_enters_detaches_and_releases_with_no_copy() {
    let ir = compile_in_nat_function(&nat_region(Term::nat_lit(7))).unwrap();
    let body = function_body(&ir, "f");
    let enter = positions(body, ENTER);
    let detach = positions(body, DETACH);
    let release = positions(body, RELEASE);
    assert_eq!(
        (enter.len(), detach.len(), release.len()),
        (1, 1, 1),
        "{body}"
    );
    assert!(enter[0] < detach[0] && detach[0] < release[0], "{body}");
    assert!(release[0] < body.find("ret i64").unwrap(), "{body}");
    assert!(!ir.contains("__albite_region_copy_out"), "{ir}");
    assert!(!ir.contains("__tg_region_layout"), "{ir}");
    assert!(!body.contains("region_slot"), "{body}");
}

// 13.10.26q AC3: a String region's fall-through copies between detach and
// release, loads the copied value before release, and names its layout.
#[test]
fn a_string_region_copies_between_detach_and_release() {
    let term = Term::str_len(Term::region(
        RegionTypes::value_only(Type::String),
        built_string(),
    ));
    let ir = compile_in_nat_function(&term).unwrap();
    let body = function_body(&ir, "f");
    let order: Vec<usize> = [ENTER, DETACH, COPY, LOAD, RELEASE]
        .iter()
        .map(|needle| {
            let found = positions(body, needle);
            assert_eq!(found.len(), 1, "{needle} once in:\n{body}");
            found[0]
        })
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{body}");
    assert!(body.contains("@__tg_region_layout."), "{body}");
    assert!(
        ir.contains("linkonce_odr hidden unnamed_addr constant [2 x i64] [i64 2, i64 1]"),
        "{ir}"
    );
    let slot = body.find("%region_slot = alloca").expect("a slot");
    assert!(
        slot < body.find(ENTER).unwrap(),
        "the slot is in the entry block"
    );
}

// 9.10.26o AC4, 13.10.26q AC3: a `return` of a String from two nested
// regions copies out of and releases each, innermost first, before its `ret`.
#[test]
fn a_string_return_from_two_nested_regions_copies_each_before_ret() {
    let inner = crossed_region(Type::Nat, Type::String, Term::early_return(built_string()));
    let term = crossed_region(Type::Nat, Type::String, inner);
    let ir = compile_in_function(&Term::early_return(term), true).unwrap();
    let body = function_body(&ir, "f");
    let before_ret = &body[..body.find("ret {").unwrap()];
    let detach = positions(before_ret, DETACH);
    let copy = positions(before_ret, COPY);
    let release = positions(before_ret, RELEASE);
    assert_eq!(
        (detach.len(), copy.len(), release.len()),
        (2, 2, 2),
        "{body}"
    );
    assert!(
        detach[0] < copy[0]
            && copy[0] < release[0]
            && release[0] < detach[1]
            && detach[1] < copy[1]
            && copy[1] < release[1],
        "{body}"
    );
}

// 9.10.26o AC4: a `return` from two nested Nat regions releases both,
// innermost first, before its `ret`, and copies nothing.
#[test]
fn a_return_from_two_nested_regions_releases_both_before_ret() {
    let term = crossed_region(
        Type::Nat,
        Type::Nat,
        crossed_region(Type::Nat, Type::Nat, Term::early_return(Term::nat_lit(1))),
    );
    let ir = compile_in_nat_function(&term).unwrap();
    let body = function_body(&ir, "f");
    let before_ret = &body[..body.find("ret i64").unwrap()];
    assert_eq!(positions(before_ret, ENTER).len(), 2, "{body}");
    assert_eq!(positions(before_ret, DETACH).len(), 2, "{body}");
    assert_eq!(positions(before_ret, RELEASE).len(), 2, "{body}");
    assert!(positions(before_ret, COPY).is_empty(), "{body}");
    let detach = positions(before_ret, DETACH);
    let release = positions(before_ret, RELEASE);
    assert!(detach[0] < release[0] && release[0] < detach[1] && detach[1] < release[1]);
}

// 13.10.26q D5: a crossing `return` through a region that recorded no exit
// type is an internal error naming the ADR, never a silent skip.
#[test]
fn a_crossing_return_without_an_exit_type_is_an_internal_error() {
    let term = nat_region(Term::early_return(Term::nat_lit(1)));
    let err = compile_in_nat_function(&term).unwrap_err();
    assert!(err.contains(MISSING_EXIT_TYPE), "{err}");
}

// 13.10.26q D4: a layout disagreeing with the lowered value is refused
// rather than copied at the wrong offsets.
#[test]
fn a_layout_whose_size_disagrees_with_the_value_is_refused() {
    let term = Term::region(RegionTypes::value_only(Type::String), Term::nat_lit(1));
    let err = compile_in_nat_function(&term).unwrap_err();
    assert!(err.contains("copy layout of `String` is 16 bytes"), "{err}");
}

// 13.10.26q D11: a type the copy cannot classify keeps a refusal naming it.
#[test]
fn an_unclassifiable_value_type_is_refused_by_shape() {
    let term = Term::region(
        RegionTypes::value_only(Type::arrow(Type::Nat, Type::Nat)),
        Term::nat_lit(1),
    );
    let err = compile_in_nat_function(&term).unwrap_err();
    assert!(err.contains("has no copy layout"), "{err}");
}

// 9.10.26o AC4: a `return` inside a lambda inside a region emits no detach
// in the lambda's function; the region still releases on fall-through.
#[test]
fn a_return_inside_a_lambda_releases_nothing_in_the_lambda() {
    let lambda = Term::lambda("x", Type::Nat, Term::early_return(Term::var("x")));
    let term = nat_region(Term::app(lambda, Term::nat_lit(3)));
    let ir = compile_in_nat_function(&term).unwrap();
    let outer = function_body(&ir, "f");
    assert_eq!(positions(outer, DETACH).len(), 1, "{outer}");
    let start = ir
        .match_indices("define ")
        .map(|(i, _)| i)
        .find(|i| !ir[*i..].starts_with("define i64 @f("))
        .unwrap_or_else(|| panic!("the lambda compiles to a function of its own:\n{ir}"));
    let rest = &ir[start..];
    let lambda_ir = &rest[..rest.find("\n}\n").map_or(rest.len(), |i| i + 3)];
    assert!(lambda_ir.contains("ret "), "{lambda_ir}");
    assert!(!lambda_ir.contains(DETACH), "{lambda_ir}");
    assert!(!lambda_ir.contains(RELEASE), "{lambda_ir}");
}

// 9.10.26o AC4: the open-region stack is per function — a region opened in
// one function is not released by a `return` compiled in another.
#[test]
fn open_regions_are_counted_per_function() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "region_count_test");
    let fn_type = context.i64_type().fn_type(&[], false);
    let outer = codegen.module.add_function("outer", fn_type, None);
    let inner = codegen.module.add_function("inner", fn_type, None);
    let opened = OpenRegion {
        function: outer,
        exit: None,
    };
    codegen.compilation.open_regions = vec![opened.clone(), opened];
    codegen.compilation.current_fn = Some(outer);
    assert_eq!(codegen.open_region_count(), 2);
    codegen.compilation.current_fn = Some(inner);
    assert_eq!(codegen.open_region_count(), 0);
    codegen.compilation.current_fn = None;
    assert_eq!(codegen.open_region_count(), 0);
}

// 9.10.26o AC4: a region in tail position records `InsideRegion`, and the
// body's call is no `musttail`.
#[test]
fn a_tail_position_inside_a_region_is_no_tail_call() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "region_tail_test");
    let fn_type = context.i64_type().fn_type(&[], false);
    let function = codegen.module.add_function("f", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    codegen.builder.position_at_end(entry);
    codegen.compilation.current_fn = Some(function);
    codegen.compilation.in_tail_position = true;
    codegen.compile_term(&nat_region(Term::nat_lit(2))).unwrap();
    let decisions = codegen.musttail_decisions();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].reasons, vec![ReasonCode::InsideRegion]);
    assert_eq!(decisions[0].function, "f");
    assert!(!codegen
        .module
        .print_to_string()
        .to_string()
        .contains("musttail"));
}

// 9.10.26o D6: a pointer-valued `return` outside every region is unchanged —
// only one that crosses a region copies.
#[test]
fn a_pointer_valued_return_outside_a_region_compiles() {
    let returned =
        compile_in_function(&Term::early_return(Term::StringLit("x".into())), true).unwrap();
    assert!(!returned.contains("__albite_region"), "{returned}");
}

// 9.10.26o D7: the driver's setter is what the main wrapper reads.
#[test]
fn set_regions_used_sets_the_marker() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "region_marker_test");
    assert!(!codegen.regions.regions_used);
    codegen.set_regions_used(true);
    assert!(codegen.regions.regions_used);
    codegen.set_regions_used(false);
    assert!(!codegen.regions.regions_used);
}
