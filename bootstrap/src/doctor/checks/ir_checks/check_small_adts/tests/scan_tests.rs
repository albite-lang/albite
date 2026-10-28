//! 28.9.26f AC1 — each category is counted, and only what it names; and
//! 28.9.26f AC2 — the recaptured fixture's counts are pinned.

use super::scan::*;

/// Wrap body lines in a `define`, so `split_ir_functions` sees a function.
fn in_function(header: &str, body: &str) -> String {
    format!("{header} {{\nentry:\n{body}\n  ret void\n}}\n")
}

pub(super) fn scan_body(body: &str) -> SmallAdtCounts {
    scan_module(&in_function("define void @f(ptr %0)", body))
}

fn shape(counts: &SmallAdtCounts, payload_bytes: u64) -> BlobShapeCounts {
    counts
        .shapes
        .get(&payload_bytes)
        .copied()
        .unwrap_or_default()
}

#[test]
fn blob_bytes_rounds_tag_plus_payload_up_to_four() {
    assert_eq!(blob_bytes(0), 4);
    assert_eq!(blob_bytes(1), 8);
    assert_eq!(blob_bytes(4), 8);
    assert_eq!(blob_bytes(5), 12);
    assert_eq!(blob_bytes(8), 12);
    assert_eq!(blob_bytes(12), 16);
    assert_eq!(blob_bytes(13), 20);
}

#[test]
fn parse_blob_prefix_reads_only_the_blob_spelling() {
    assert_eq!(parse_blob_prefix("{ i32, [8 x i8] } %1"), Some((8, " %1")));
    assert_eq!(parse_blob_prefix("{ i32, [x x i8] }"), None);
    assert_eq!(parse_blob_prefix("{ i64, [8 x i8] }"), None);
    assert_eq!(parse_blob_prefix("{ i32, [8 x i16] }"), None);
}

#[test]
fn by_value_params_and_returns_are_read_from_the_define_header() {
    let counts = scan_module(&in_function(
        r#"define { i32, [0 x i8] } @"pick$direct"(ptr %0, { i32, [0 x i8] } %1, { i32, [8 x i8] } %2, i64 %3)"#,
        "",
    ));
    assert_eq!(shape(&counts, 0).by_value_returns, 1);
    assert_eq!(shape(&counts, 0).by_value_params, 1);
    assert_eq!(shape(&counts, 8).by_value_params, 1);
    assert_eq!(shape(&counts, 8).by_value_returns, 0);
    assert_eq!(counts.defines, 1);
}

/// A quoted name may hold a `(`: the parameter list starts after the closing
/// quote, not at the first parenthesis after the `@`.
#[test]
fn a_parenthesis_inside_a_quoted_name_is_not_the_parameter_list() {
    for header in [
        r#"define void @"f("({ i32, [0 x i8] } %0)"#,
        r#"define void @"g(x"({ i32, [0 x i8] } %0)"#,
        r#"define void @"(h"({ i32, [0 x i8] } %0)"#,
    ] {
        let counts = scan_module(&in_function(header, ""));
        assert_eq!(shape(&counts, 0).by_value_params, 1, "{header}");
    }
    let bare = scan_module(&in_function("define void @k({ i32, [0 x i8] } %0)", ""));
    assert_eq!(shape(&bare, 0).by_value_params, 1);
}

#[test]
fn a_wrapped_blob_return_type_is_not_a_by_value_blob_return() {
    let counts = scan_module(&in_function(
        "define { { i32, [0 x i8] }, i64 } @f(ptr %0)",
        "",
    ));
    assert!(counts.shapes.is_empty(), "{counts:?}");
}

/// 28.9.26f AC1 — a `declare` (or a call) with `sret(…)` is not counted.
#[test]
fn sret_in_a_define_counts_and_in_a_declare_or_call_does_not() {
    let module = format!(
        "declare void @g(ptr sret({{ i32, [4 x i8] }}) %0)\n{}",
        in_function(
            "define void @\"f$direct_mt\"(ptr noalias nonnull sret({ i32, [1 x i8] }) align 4 dereferenceable(8) %0, ptr %1)",
            "  call void @\"f$direct_mt\"(ptr noalias sret({ i32, [1 x i8] }) %sret_buf, ptr null)",
        )
    );
    let counts = scan_module(&module);
    assert_eq!(shape(&counts, 1).sret_returns, 1);
    assert_eq!(
        shape(&counts, 1).by_value_params,
        0,
        "an sret param is not by value"
    );
    assert_eq!(
        shape(&counts, 4).sret_returns,
        0,
        "a declare is not counted"
    );
}

#[test]
fn indirect_buffers_count_by_their_alloca() {
    let counts = scan_body(
        "  %indirect_buf.0 = alloca { i32, [0 x i8] }, align 8\n  %indirect_buf.1 = alloca { i32, [12 x i8] }, align 8\n  %sret_buf = alloca { i32, [0 x i8] }, align 8",
    );
    assert_eq!(shape(&counts, 0).indirect_params, 1);
    assert_eq!(shape(&counts, 12).indirect_params, 1);
    assert_eq!(counts.blob_buffer_sites(), 2);
    assert_eq!(counts.small_blob_buffer_sites(), 1);
    assert_eq!(counts.small16_blob_buffer_sites(), 2);
}

#[test]
fn small_by_value_sites_are_the_eight_byte_ones() {
    let counts = scan_module(&in_function(
        "define { i32, [4 x i8] } @f({ i32, [1 x i8] } %0, { i32, [5 x i8] } %1)",
        "",
    ));
    assert_eq!(counts.small_blob_by_value_sites(), 2);
}

const CONSTRUCT_PTR: &str = "  %sum_alloca = alloca { i32, [8 x i8] }, align 16
  %data_ptr = getelementptr inbounds { i32, [8 x i8] }, ptr %sum_alloca, i32 0, i32 1
  store ptr %l, ptr %data_ptr, align 4
  %sum_val = load { i32, [8 x i8] }, ptr %sum_alloca, align 16";

#[test]
fn a_pointer_payload_construction_counts_at_n_eight() {
    let counts = scan_body(CONSTRUCT_PTR);
    assert_eq!(shape(&counts, 8).constructions, 1);
    assert_eq!(counts.blob_constructions(), 1);
    assert_eq!(counts.pointer_payload_constructions, 1);
    assert_eq!(counts.construction_allocas, 1);
}

#[test]
fn a_non_pointer_or_other_width_payload_is_not_a_niche_candidate() {
    let i64_payload = scan_body(&CONSTRUCT_PTR.replace("store ptr %l", "store i64 %l"));
    assert_eq!(i64_payload.pointer_payload_constructions, 0);
    assert_eq!(i64_payload.blob_constructions(), 1);
    let wide = scan_body(&CONSTRUCT_PTR.replace("[8 x i8]", "[16 x i8]"));
    assert_eq!(wide.pointer_payload_constructions, 0);
    assert_eq!(shape(&wide, 16).constructions, 1);
}

#[test]
fn a_match_read_through_data_ptr_is_not_a_construction() {
    let counts = scan_body(
        "  %adt_scrut_ptr = alloca { i32, [8 x i8] }, align 16
  %data_ptr = getelementptr inbounds { i32, [8 x i8] }, ptr %adt_scrut_ptr, i32 0, i32 1
  %payload_1 = load i64, ptr %data_ptr, align 4",
    );
    assert_eq!(counts.blob_constructions(), 0);
    assert_eq!(
        counts.construction_allocas, 0,
        "a scrutinee slot is not a construction"
    );
}

pub(super) const ARENA_FOLD: &str = "  %mu_alloc.malloc = call ptr @malloc(i64 20)
  %mu_alloc.arena = call ptr @__albite_alloc(i64 20, i32 0)
  %mu_alloc = phi ptr [ %mu_alloc.malloc, %alloc.malloc ], [ %mu_alloc.arena, %alloc.arena ]";

#[test]
fn a_profiled_and_an_arena_fold_each_count_once() {
    assert_eq!(scan_body(ARENA_FOLD).mu_alloc_sites, 1);
    let profiled = scan_body("  %mu_alloc = call ptr @__albite_alloc(i64 20, i32 0)");
    assert_eq!(profiled.mu_alloc_sites, 1);
    let closure_env = scan_body("  %env_alloc.arena = call ptr @__albite_alloc(i64 4, i32 1)");
    assert_eq!(closure_env.mu_alloc_sites, 0, "class 1 is not a fold");
}

#[test]
fn a_stack_fold_is_a_mu_stack_site_never_a_mu_alloc_site() {
    let counts = scan_body("  %mu_stack3 = alloca { i32, [16 x i8] }, align 16");
    assert_eq!(counts.mu_stack_sites, 1);
    assert_eq!(counts.mu_alloc_sites, 0);
}

pub(super) fn nullary_fold(payload_store: &str) -> String {
    format!(
        "  %sum_alloca = alloca {{ i32, [16 x i8] }}, align 16
  %data_ptr = getelementptr inbounds {{ i32, [16 x i8] }}, ptr %sum_alloca, i32 0, i32 1
  {payload_store}
  %sum_val = load {{ i32, [16 x i8] }}, ptr %sum_alloca, align 16
{ARENA_FOLD}
  store {{ i32, [16 x i8] }} %sum_val, ptr %mu_alloc, align 16"
    )
}

#[test]
fn an_empty_payload_fold_is_nullary() {
    let counts = scan_body(&nullary_fold(
        "store {} zeroinitializer, ptr %data_ptr, align 4",
    ));
    assert_eq!(counts.nullary_mu_alloc_sites, 1);
    assert_eq!(counts.mu_alloc_sites, 1);
}

#[test]
fn a_non_empty_payload_fold_is_not_nullary() {
    let counts = scan_body(&nullary_fold(
        "store { i64, ptr } %pair, ptr %data_ptr, align 4",
    ));
    assert_eq!(counts.nullary_mu_alloc_sites, 0);
}

#[test]
fn the_emit_log_counter_matches_the_three_warning_forms_only() {
    let log = "warning: TyVar(\"a\") reached lower_type — falling back to ptr
warning: Type::App(List, [Nat]) encountered in codegen — ptr fallback
warning: lower_type recursion depth exceeded
note: TyVar(\"a\") resolved
note: a fallback reached lower_type
note: Type::App(List, [Nat]) resolved
note: a fallback encountered in codegen
lower_type ok";
    assert_eq!(count_fallbacks(log), 3);
    assert_eq!(count_fallbacks(""), 0);
}

#[test]
fn counts_add_across_modules_but_leave_fallbacks_alone() {
    let mut total = scan_body(CONSTRUCT_PTR);
    let mut other = scan_body(ARENA_FOLD);
    other.pointer_fallbacks = Some(5);
    total.add(&other);
    total.add(&scan_body(
        "  %mu_stack = alloca { i32, [0 x i8] }, align 16",
    ));
    assert_eq!(total.defines, 3);
    assert_eq!(total.mu_alloc_sites, 1);
    assert_eq!(total.mu_stack_sites, 1);
    assert_eq!(total.pointer_payload_constructions, 1);
    assert_eq!(total.construction_allocas, 1);
    assert_eq!(total.pointer_fallbacks, None);
    let mut nullary = scan_body(&nullary_fold(
        "store {} zeroinitializer, ptr %data_ptr, align 4",
    ));
    nullary.add(&nullary.clone());
    assert_eq!(nullary.nullary_mu_alloc_sites, 2);
    assert_eq!(shape(&nullary, 16).constructions, 2);
}

#[test]
fn every_per_shape_field_sums_on_add() {
    let shape_of = |base: u64| BlobShapeCounts {
        by_value_params: base,
        by_value_returns: 2 * base,
        sret_returns: 3 * base,
        indirect_params: 4 * base,
        constructions: 5 * base,
    };
    let mut total = SmallAdtCounts::default();
    total.shapes.insert(4, shape_of(1));
    let mut other = SmallAdtCounts::default();
    other.shapes.insert(4, shape_of(10));
    total.add(&other);
    assert_eq!(shape(&total, 4), shape_of(11));
}

#[test]
fn every_scalar_count_sums_on_add() {
    let counts_of = |base: u64| SmallAdtCounts {
        defines: base,
        pointer_payload_constructions: 2 * base,
        mu_alloc_sites: 3 * base,
        mu_stack_sites: 4 * base,
        nullary_mu_alloc_sites: 5 * base,
        construction_allocas: 6 * base,
        ..Default::default()
    };
    let mut total = counts_of(1);
    total.add(&counts_of(10));
    assert_eq!(total, counts_of(11));
}

#[test]
fn counted_anything_is_false_only_for_a_module_with_no_category() {
    assert!(!scan_body("  %x = add i64 1, 2").counted_anything());
    assert!(scan_body(ARENA_FOLD).counted_anything());
    assert!(scan_body("  %mu_stack = alloca { i32, [0 x i8] }, align 16").counted_anything());
    assert!(scan_body("  %adt_alloca = alloca { i32, [0 x i8] }, align 16").counted_anything());
    assert!(
        scan_module(&in_function("define void @f({ i32, [0 x i8] } %0)", "")).counted_anything()
    );
}

const FIXTURE: &str = include_str!("../testdata/small_adts_fixture.ll.txt");

/// 28.9.26f AC2 — the §2.5 fixture, recaptured 2026-09-29 with a debug build.
/// The counts equal the drafting survey's (§6.2).
#[test]
fn the_fixture_counts_are_pinned() {
    let counts = scan_module(FIXTURE);
    assert_eq!(shape(&counts, 0).sret_returns, 1, "pick");
    assert_eq!(shape(&counts, 1).sret_returns, 1, "flip");
    assert_eq!(shape(&counts, 8).sret_returns, 1, "find");
    assert_eq!(shape(&counts, 0).indirect_params, 1, "pick");
    assert_eq!(shape(&counts, 1).indirect_params, 1, "flip");
    assert_eq!(shape(&counts, 8).indirect_params, 0);
    assert_eq!(counts.small_blob_buffer_sites(), 4);
    assert_eq!(counts.small16_blob_buffer_sites(), 5);
    assert_eq!(counts.blob_buffer_sites(), 5);
    assert_eq!(counts.mu_alloc_sites, 6);
    assert_eq!(counts.mu_stack_sites, 0);
    assert_eq!(counts.nullary_mu_alloc_sites, 2, "LNil and TMid");
    assert_eq!(counts.pointer_payload_constructions, 1, "Held");
}
