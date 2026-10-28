//! Tests for E0095's handle half (ADR 9.10.26o AC 3): every route a handle
//! could take into a region body is refused at the offending name, and a
//! region that reaches none is accepted. Pure walks over parsed source.

use super::tests::findings;
use super::*;

fn reached(name: &str) -> (HandleFindingKind, String, String) {
    (
        HandleFindingKind::ReachedFromRegion,
        name.into(),
        name.into(),
    )
}

// 9.10.26o AC3: a handle function named inside a region is refused.
#[test]
fn a_handle_function_inside_a_region_is_refused() {
    assert_eq!(
        findings("fn f() -> Nat { region { bytes_len(bytes_new(4)) } }"),
        vec![reached("bytes_len"), reached("bytes_new")]
    );
}

// 9.10.26o AC3: an outer handle variable and a handle constructor are refused.
#[test]
fn an_outer_handle_variable_and_a_handle_constructor_are_refused() {
    assert_eq!(
        findings("fn f(b: Bytes) -> Nat { region { let c = b; 0 } }"),
        vec![reached("b")]
    );
    assert_eq!(
        findings("fn f() -> Nat { region { let c = MkBytes(1); 0 } }"),
        vec![reached("MkBytes")]
    );
}

// 9.10.26o AC3: a region reaching no handle, a local shadowing a handle
// function's name, and a handle use outside any region are all accepted.
#[test]
fn a_region_reaching_no_handle_is_accepted() {
    assert!(findings("fn f(n: Nat) -> Nat { region { n + 1 } }").is_empty());
    assert!(findings("fn f(bytes_new: Nat) -> Nat { region { bytes_new } }").is_empty());
    assert!(findings("fn f() -> Nat { bytes_len(bytes_new(4)) }").is_empty());
}

// 9.10.26o AC3: the floor is restored after the region, and nests.
#[test]
fn the_region_floor_is_scoped_to_the_body() {
    assert_eq!(
        findings("fn f(b: Bytes) -> Nat { let n = region { region { 1 } }; bytes_len(b) + n }"),
        vec![]
    );
    assert_eq!(
        findings("fn f(b: Bytes) -> Nat { region { region { let c = b; 0 } } }"),
        vec![reached("b")]
    );
}

// 9.10.26o AC3: the finding becomes E0095, naming the handle and definition.
#[test]
fn a_region_finding_becomes_e0095() {
    let error = finding_error(
        "f",
        HandleFinding {
            kind: HandleFindingKind::ReachedFromRegion,
            handle: "b".into(),
            span: crate::span::Span::new(3, 4),
        },
    );
    assert_eq!(error.kind.code(), "E0095");
    assert_eq!(
        error.message,
        "a region body reaches `b`, a resource handle, in `f`"
    );
}
