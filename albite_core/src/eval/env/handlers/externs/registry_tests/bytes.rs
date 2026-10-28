//! The byte-buffer entries of [`EXECUTABLE_EXTERNS`] (ADR 3.10.26n AC 2,
//! 4.10.26n AC 5): every [`ByteIntrinsic`] is registered as `Bytes`, none is
//! `Pure`, and the list of `Bytes` entries is exactly the intrinsic table.

use super::super::{entry_for, is_codegen_intrinsic, ExternKind, EXECUTABLE_EXTERNS};
use super::{dispatch_claims, sample_arguments};
use crate::eval::byte_intrinsics::ByteIntrinsic;

#[test]
fn every_byte_intrinsic_is_registered_as_bytes_and_none_is_pure() {
    for op in ByteIntrinsic::ALL {
        let entry = entry_for(op.symbol()).expect("registered");
        assert_eq!(entry.kind, ExternKind::Bytes, "{}", op.symbol());
        assert_ne!(
            entry.kind,
            ExternKind::Pure,
            "{} would be folded",
            op.symbol()
        );
    }
}

#[test]
fn the_intrinsic_list_is_exactly_the_bytes_entries() {
    let registered: Vec<&str> = EXECUTABLE_EXTERNS
        .iter()
        .filter(|entry| entry.kind == ExternKind::Bytes)
        .map(|entry| entry.name)
        .collect();
    let intrinsics: Vec<&str> = ByteIntrinsic::ALL.iter().map(|op| op.symbol()).collect();
    assert_eq!(registered, intrinsics);
    for name in registered {
        assert!(is_codegen_intrinsic(name), "{name}");
        assert!(is_codegen_intrinsic(&format!("__c_{name}")), "{name}");
    }
    assert!(!is_codegen_intrinsic("tg_bytes_nope"));
    assert!(!is_codegen_intrinsic("tg_string_builder_new"));
}

// 4.10.26n AC 5: the seventh entry is claimed by dispatch, over a live buffer.
#[test]
fn the_reserved_conversion_is_claimed_by_dispatch() {
    let name = ByteIntrinsic::IntoStringUnchecked.symbol();
    assert_eq!(name, "tg_bytes_into_string_unchecked");
    assert!(dispatch_claims(name, &sample_arguments(name)));
}

// 6.10.26n AC 2: the eighth entry is claimed by dispatch, and is not `Pure`.
#[test]
fn the_reserved_adoption_is_claimed_by_dispatch_and_is_not_pure() {
    let name = ByteIntrinsic::FromStringGrown.symbol();
    assert_eq!(name, "tg_bytes_from_string_grown");
    assert!(dispatch_claims(name, &sample_arguments(name)));
    assert_eq!(entry_for(name).expect("registered").kind, ExternKind::Bytes);
}

// 9.10.26n D2: the ninth entry is claimed by dispatch, and is not `Pure`.
#[test]
fn the_reserved_unfilled_new_is_claimed_by_dispatch_and_is_not_pure() {
    let name = ByteIntrinsic::NewUninit.symbol();
    assert_eq!(name, "tg_bytes_new_uninit");
    assert!(dispatch_claims(name, &sample_arguments(name)));
    assert_eq!(entry_for(name).expect("registered").kind, ExternKind::Bytes);
    assert_eq!(ByteIntrinsic::ALL.len(), 9);
}
