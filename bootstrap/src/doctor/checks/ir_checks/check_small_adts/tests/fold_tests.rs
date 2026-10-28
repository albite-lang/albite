//! 28.9.26f AC1 — the nullary-fold rule's edge cases: tracked by SSA name,
//! never by proximity; transitive through nested blobs; bounded on a cycle.
//! Split from `scan_tests.rs` to keep both under the file-size gate.

use super::scan_tests::{nullary_fold, scan_body, ARENA_FOLD};

/// The proximity trap: a `{}` store sits on the line before the fold, but
/// into a DIFFERENT alloca than the one the folded value was loaded from.
#[test]
fn a_nearby_empty_store_does_not_make_an_unrelated_fold_nullary() {
    let counts = scan_body(&format!(
        "  %sum_alloca = alloca {{ i32, [16 x i8] }}, align 16
  %data_ptr = getelementptr inbounds {{ i32, [16 x i8] }}, ptr %sum_alloca, i32 0, i32 1
  store i64 %n, ptr %data_ptr, align 4
  %sum_val = load {{ i32, [16 x i8] }}, ptr %sum_alloca, align 16
  %sum_alloca1 = alloca {{ i32, [16 x i8] }}, align 16
  %data_ptr2 = getelementptr inbounds {{ i32, [16 x i8] }}, ptr %sum_alloca1, i32 0, i32 1
  store {{}} zeroinitializer, ptr %data_ptr2, align 4
{ARENA_FOLD}
  store {{ i32, [16 x i8] }} %sum_val, ptr %mu_alloc, align 16"
    ));
    assert_eq!(counts.nullary_mu_alloc_sites, 0);
}

/// The transitive clause: `Sum(A, Sum(B, C))`'s inner empty `B` wrapped in the
/// outer blob is still an empty constructor.
#[test]
fn a_fold_of_a_blob_wrapping_an_empty_blob_is_nullary() {
    let counts = scan_body(&format!(
        "  %sum_alloca = alloca {{ i32, [0 x i8] }}, align 16
  %data_ptr = getelementptr inbounds {{ i32, [0 x i8] }}, ptr %sum_alloca, i32 0, i32 1
  store {{}} zeroinitializer, ptr %data_ptr, align 4
  %sum_val = load {{ i32, [0 x i8] }}, ptr %sum_alloca, align 16
  %sum_alloca1 = alloca {{ i32, [16 x i8] }}, align 16
  %data_ptr2 = getelementptr inbounds {{ i32, [16 x i8] }}, ptr %sum_alloca1, i32 0, i32 1
  store {{ i32, [0 x i8] }} %sum_val, ptr %data_ptr2, align 4
  %sum_val3 = load {{ i32, [16 x i8] }}, ptr %sum_alloca1, align 16
{ARENA_FOLD}
  store {{ i32, [16 x i8] }} %sum_val3, ptr %mu_alloc, align 16"
    ));
    assert_eq!(counts.nullary_mu_alloc_sites, 1);
}

/// A fold whose value wraps an empty constructor `wrappers` blobs deep: the
/// innermost alloca stores `{}`, each outer one stores the blob loaded from
/// the one inside it.
fn nested_empty_fold(wrappers: usize) -> String {
    let mut body = String::from(
        "  %data_ptr = getelementptr inbounds { i32, [0 x i8] }, ptr %sum_alloca, i32 0, i32 1
  store {} zeroinitializer, ptr %data_ptr, align 4
  %v0 = load { i32, [0 x i8] }, ptr %sum_alloca, align 16\n",
    );
    for level in 1..=wrappers {
        body.push_str(&format!(
            "  %data_ptr{level} = getelementptr inbounds {{ i32, [0 x i8] }}, ptr %sum_alloca{level}, i32 0, i32 1
  store {{ i32, [0 x i8] }} %v{inner}, ptr %data_ptr{level}, align 4
  %v{level} = load {{ i32, [0 x i8] }}, ptr %sum_alloca{level}, align 16\n",
            inner = level - 1
        ));
    }
    body.push_str(&format!(
        "{ARENA_FOLD}\n  store {{ i32, [0 x i8] }} %v{wrappers}, ptr %mu_alloc, align 16"
    ));
    body
}

/// The depth bound only stops a malformed cycle: sixteen wrappers are still
/// read to the bottom, seventeen are past the bound.
#[test]
fn the_nesting_bound_reads_sixteen_wrappers_and_stops_at_seventeen() {
    assert_eq!(scan_body(&nested_empty_fold(16)).nullary_mu_alloc_sites, 1);
    assert_eq!(scan_body(&nested_empty_fold(17)).nullary_mu_alloc_sites, 0);
}

/// A value whose alloca's payload is that same value terminates, as not
/// nullary, instead of recursing forever.
#[test]
fn a_self_referential_payload_terminates_as_not_nullary() {
    let counts = scan_body(&format!(
        "  %data_ptr = getelementptr inbounds {{ i32, [0 x i8] }}, ptr %sum_alloca, i32 0, i32 1
  store {{ i32, [0 x i8] }} %v, ptr %data_ptr, align 4
  %v = load {{ i32, [0 x i8] }}, ptr %sum_alloca, align 16
{ARENA_FOLD}
  store {{ i32, [0 x i8] }} %v, ptr %mu_alloc, align 16"
    ));
    assert_eq!(counts.nullary_mu_alloc_sites, 0);
}

#[test]
fn a_blob_stored_anywhere_but_a_mu_alloc_cell_is_not_a_fold() {
    let counts = scan_body(
        &nullary_fold("store {} zeroinitializer, ptr %data_ptr, align 4").replace(
            "ptr %mu_alloc, align 16",
            "ptr %mu_alloc.arena_copy, align 16",
        ),
    );
    assert_eq!(counts.nullary_mu_alloc_sites, 0);
}
